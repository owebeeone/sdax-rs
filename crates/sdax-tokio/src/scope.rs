//! The run's [`Scope`]: what `cx.spawn` reaches, and the [`Child`] it hands
//! back.
//!
//! A body runs in a task of its own and cannot touch the machine, which lives
//! behind the driver task. So the decision is taken against a [`SpawnTable`] —
//! a snapshot the driver publishes after every step, whose `check` is
//! `Machine::spawn_check` as of that step — and the instance is then announced
//! to the driver as an [`Event::InstanceSpawned`].
//!
//! Between that check and the event the run may settle. The machine resolves
//! it: the instance is created and ended at once, and `Child::ready()` answers
//! `Err` rather than hanging.

use crate::body::{Msg, Tx};
use sdax::host::engine::{Event, RunState, SpawnTable};
use sdax::host::ChildControl;
use sdax::host::{BoxFuture, InstanceId, RawKey, Scope, StopSignal};
use sdax::{Child, Error, Outcome, SpawnError, Stop};
use std::any::Any;
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

/// An instance that ended before it was ready.
#[derive(Debug)]
pub struct InstanceEnded(
    /// How it ended.
    pub Outcome,
);

impl std::fmt::Display for InstanceEnded {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "the instance ended ({}) before it was ready", self.0)
    }
}
impl std::error::Error for InstanceEnded {}

/// One instance's readiness latch, the same shape as the run's own.
struct Latch {
    sig: Arc<StopSignal>,
    state: Mutex<Option<Result<(), Outcome>>>,
}

impl Latch {
    fn new() -> Arc<Latch> {
        Arc::new(Latch {
            sig: StopSignal::new(),
            state: Mutex::new(None),
        })
    }

    fn set(&self, v: Result<(), Outcome>) {
        let mut st = self.state.lock().expect("latch poisoned");
        if st.is_none() {
            *st = Some(v);
            self.sig.request();
        }
    }
}

/// The instances this run created, as a `Child` drives them.
struct Children {
    tx: Tx,
    latches: Mutex<BTreeMap<InstanceId, ChildLatch>>,
}

struct ChildLatch {
    latch: Arc<Latch>,
    answered: bool,
}

impl Children {
    fn latch(&self, id: InstanceId) -> Option<Arc<Latch>> {
        self.latches
            .lock()
            .expect("children poisoned")
            .get(&id)
            .map(|entry| entry.latch.clone())
    }
}

impl ChildControl for Children {
    fn stop(&self, id: InstanceId) {
        let _ = self.tx.send(Msg::Request(Event::StopInstance(id)));
    }

    fn ready(&self, id: InstanceId) -> BoxFuture<'static, Result<(), Error>> {
        let latch = self.latch(id);
        Box::pin(async move {
            let Some(latch) = latch else {
                // The instance is gone: its latch was answered and dropped.
                return Ok(());
            };
            Stop::on(latch.sig.clone()).await;
            let v = *latch.state.lock().expect("latch poisoned");
            match v {
                Some(Ok(())) | None => Ok(()),
                Some(Err(o)) => Err(Box::new(InstanceEnded(o)) as Error),
            }
        })
    }
}

/// The run, as a body may reach it.
pub(crate) struct RunScope {
    tx: Tx,
    table: Mutex<SpawnTable>,
    next: Mutex<u64>,
    inputs: Mutex<Vec<(InstanceId, Box<dyn Any + Send + Sync>)>>,
    children: Arc<Children>,
}

impl RunScope {
    pub(crate) fn new(tx: Tx) -> Arc<RunScope> {
        Arc::new(RunScope {
            tx: tx.clone(),
            table: Mutex::new(SpawnTable::default()),
            next: Mutex::new(0),
            inputs: Mutex::new(Vec::new()),
            children: Arc::new(Children {
                tx,
                latches: Mutex::new(BTreeMap::new()),
            }),
        })
    }

    /// The driver publishes the gate after every step.
    pub(crate) fn publish(&self, table: SpawnTable) {
        *self.table.lock().expect("scope poisoned") = table;
    }

    /// The per-instance input the body handed over, for
    /// [`Effect::SpawnInstance`](sdax::host::engine::Effect::SpawnInstance).
    pub(crate) fn take_input(&self, id: InstanceId) -> Box<dyn Any + Send + Sync> {
        let mut inputs = self.inputs.lock().expect("scope poisoned");
        match inputs.iter().position(|(i, _)| *i == id) {
            Some(p) => inputs.remove(p).1,
            None => Box::new(()),
        }
    }

    /// Answer every `Child::ready()` the machine's instance states decide:
    /// steady is `Ok`.
    pub(crate) fn resolve(&self, instances: &[(InstanceId, RunState)]) {
        // Wake outside the children lock: a waker can immediately re-enter
        // the child control interface. Retain only newly answered latches.
        let ready = {
            let mut latches = self.children.latches.lock().expect("children poisoned");
            let mut ready = Vec::new();
            for (id, state) in instances {
                if *state != RunState::Steady {
                    continue;
                }
                if let Some(entry) = latches.get_mut(id) {
                    if !entry.answered {
                        entry.answered = true;
                        ready.push(entry.latch.clone());
                    }
                }
            }
            ready
        };
        for latch in ready {
            latch.set(Ok(()));
        }
    }

    /// An instance ended: whatever awaits its readiness is answered with the
    /// outcome it ended on, and its latch is dropped.
    pub(crate) fn ended(&self, id: InstanceId, outcome: Outcome) {
        let latch = {
            let mut l = self.children.latches.lock().expect("children poisoned");
            l.remove(&id).map(|entry| entry.latch)
        };
        if let Some(l) = latch {
            l.set(Err(outcome));
        }
    }
}

impl Scope for RunScope {
    fn spawn_instance(
        &self,
        spawner: RawKey,
        template: RawKey,
        input: Box<dyn Any + Send + Sync>,
    ) -> Result<Child, SpawnError> {
        self.table
            .lock()
            .expect("scope poisoned")
            .check(spawner, template)?;
        let id = {
            let mut next = self.next.lock().expect("scope poisoned");
            *next += 1;
            InstanceId(*next)
        };
        self.children
            .latches
            .lock()
            .expect("children poisoned")
            .insert(
                id,
                ChildLatch {
                    latch: Latch::new(),
                    answered: false,
                },
            );
        self.inputs
            .lock()
            .expect("scope poisoned")
            .push((id, input));
        // The machine owns the instance's lifecycle from here; the `Child` is
        // valid whatever it decides, because a refusal after this point ends
        // the instance and answers `ready()`.
        let _ = self.tx.send(Msg::Request(Event::InstanceSpawned {
            spawner,
            template,
            id,
        }));
        Ok(Child::new(id, self.children.clone()))
    }
}

#[cfg(test)]
#[path = "scope_tests.rs"]
mod tests;
