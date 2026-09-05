//! A [`BodySource`] that runs a [`Script`] instead of a plan's own bodies.
//!
//! This is what makes suite (c) a *differential* check rather than a second
//! suite: the same plans, the same scripts and the same expectations, run once
//! on the pure machine through [`ScriptedDriver`](crate::ScriptedDriver) and
//! once on a real runtime through the tokio run driver. The machine is
//! identical in both; only the thing performing the effects changes.
//!
//! Every wait is on the engine's injected clock (`cx.sleep`), so under
//! `start_paused(true)` a scripted run takes no wall-clock time at all
//! (LBT-008).

use sdax::host::engine::{EngineError, Machine};
use sdax::host::{BodySource, CxInner, RawKey, Task, Time};
use sdax::{
    Acquire, At, Body, Cleanup, Cx, Ending, Error, Kind, Plan, Run, Script, Serve, Serving, Start,
};
use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once};

/// The error a scripted body returns.
#[derive(Debug)]
struct Scripted(String);
impl std::fmt::Display for Scripted {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for Scripted {}

/// The payload a scripted panic carries. The driver turns it into
/// `FaultKind::Panic`, exactly as the simulator does.
const SCRIPTED_PANIC: &str = "scripted panic";

static QUIET: Once = Once::new();

/// Stop the default panic hook from printing the scripted panics.
///
/// A scripted panic is an *input*, not a failure, and a suite that exercises
/// `C-06`, `C-31` and `R-06` raises dozens of them. Only that exact payload is
/// swallowed; every other panic still prints in full.
pub fn quiet_scripted_panics() {
    QUIET.call_once(|| {
        let previous = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let p = info.payload();
            // `panic!("literal")` carries a `&'static str`; `panic!("{x}")`
            // formats and carries a `String`. Both spellings appear.
            let scripted = p.downcast_ref::<&str>().map(|s| *s == SCRIPTED_PANIC) == Some(true)
                || p.downcast_ref::<String>().map(|s| s == SCRIPTED_PANIC) == Some(true);
            if !scripted {
                previous(info);
            }
        }));
    });
}

struct NodeScript {
    kind: Kind,
    bodies: Vec<Body>,
    serves: Vec<Serve>,
    cleanup: Cleanup,
}

/// A plan's bodies, replaced by what a [`Script`] says they do.
pub struct ScriptedBodies {
    nodes: HashMap<RawKey, NodeScript>,
    episodes: Mutex<HashMap<RawKey, usize>>,
}

impl ScriptedBodies {
    /// The scripted bodies of `plan` under `script`.
    ///
    /// Refuses a plan the machine refuses, and a script naming a node the plan
    /// does not declare — the same two refusals `Plan::simulate` makes.
    pub fn new<Out: Send + Sync + 'static>(
        plan: &Plan<Out>,
        script: &Script,
    ) -> Result<Arc<ScriptedBodies>, EngineError> {
        let machine = Machine::new(plan)?;
        let mut nodes = HashMap::new();
        for (key, path, kind) in machine.nodes() {
            let p = path.to_string();
            nodes.insert(
                key,
                NodeScript {
                    kind,
                    bodies: script
                        .body_of(&p)
                        .map(|b| b.to_vec())
                        .unwrap_or_else(|| vec![Body::default()]),
                    serves: script
                        .serve_of(&p)
                        .map(|s| s.to_vec())
                        .unwrap_or_else(|| vec![Serve::default()]),
                    cleanup: script.cleanup_of(&p).cloned().unwrap_or_default(),
                },
            );
        }
        Ok(Arc::new(ScriptedBodies {
            nodes,
            episodes: Mutex::new(HashMap::new()),
        }))
    }
}

/// Resolve a script instant against the moment the body started.
fn target(at: At, start: Time) -> Time {
    match at {
        At::Tick(d) => (Time::ZERO + d).max(start),
        At::After(d) => start + d,
    }
}

async fn wait_until<P>(cx: &Cx<P>, when: Time) {
    if let Some(d) = when.checked_duration_since(cx.now()) {
        if !d.is_zero() {
            cx.sleep(d).await;
        }
    }
}

/// The value a scripted resource or effect registers. Nothing reads it: a
/// scripted run exercises the machine, not the author's dataflow.
struct Registered;

/// One attempt of a scripted prepare, run or start body.
async fn scripted_body(
    inner: Arc<CxInner>,
    kind: Kind,
    spec: Body,
    serve: Option<Serve>,
) -> Result<(), Error> {
    let cx: Cx<Run> = Cx::new(inner.clone());
    let start = cx.now();
    let end = match &spec.ending {
        Ending::Ok(at) | Ending::Fail(at, _) | Ending::Panic(at) => Some(target(*at, start)),
        Ending::Pending => None,
    };
    if kind.can_hold() {
        let held = match (spec.held, &spec.ending) {
            (Some(at), _) => Some(target(at, start)),
            (None, Ending::Ok(_)) => end,
            _ => None,
        };
        // A body registers its value inside itself, so a hold can never be
        // later than the body's own ending (the simulator clamps it the same
        // way).
        if let Some(t) = held.map(|h| match end {
            Some(e) => h.min(e),
            None => h,
        }) {
            wait_until(&cx, t).await;
            let acq: Cx<Acquire> = Cx::new(inner.clone());
            let _ = acq.hold_value(Registered);
        }
    }
    match end {
        None => std::future::pending::<()>().await,
        Some(t) => wait_until(&cx, t).await,
    }
    match spec.ending {
        Ending::Ok(_) => {}
        // A try-step's `Err` is its value, not a fault (OD-5): the body the
        // engine sees returns `Ok`.
        Ending::Fail(_, _) if kind == Kind::TryStep => {}
        Ending::Fail(_, msg) => return Err(Box::new(Scripted(msg))),
        Ending::Panic(_) => panic!("{}", SCRIPTED_PANIC),
        Ending::Pending => unreachable!("pending never ends"),
    }
    if kind == Kind::Service {
        let s = serve.unwrap_or_default();
        let start_cx: Cx<Start> = Cx::new(inner.clone());
        let serving = Serving::new((), scripted_serve(inner.clone(), s));
        let (handle, fut) = serving.into_parts();
        inner.put_output(Box::new(Arc::new(handle)));
        inner.put_serve(fut);
        let _ = start_cx;
    } else if !kind.can_hold() {
        inner.put_output(Box::new(Arc::new(())));
    }
    Ok(())
}

/// One serving episode.
async fn scripted_serve(inner: Arc<CxInner>, spec: Serve) -> Result<(), Error> {
    let cx: Cx<Run> = Cx::new(inner);
    let start = cx.now();
    match spec {
        Serve::Ok(at) => {
            wait_until(&cx, target(at, start)).await;
            Ok(())
        }
        Serve::Err(at, msg) => {
            wait_until(&cx, target(at, start)).await;
            Err(Box::new(Scripted(msg)))
        }
        Serve::IgnoreStop => std::future::pending().await,
        Serve::StopsAfter(d) => {
            cx.stop().await;
            if !d.is_zero() {
                cx.sleep(d).await;
            }
            Ok(())
        }
    }
}

/// One release or compensation.
async fn scripted_cleanup(inner: Arc<CxInner>, spec: Cleanup) -> Result<(), Error> {
    let cx: Cx<sdax::Release> = Cx::new(inner);
    match spec {
        Cleanup::Ok(d) => {
            if !d.is_zero() {
                cx.sleep(d).await;
            }
            Ok(())
        }
        Cleanup::Fail(d, msg) => {
            if !d.is_zero() {
                cx.sleep(d).await;
            }
            Err(Box::new(Scripted(msg)))
        }
        Cleanup::Panic(d) => {
            if !d.is_zero() {
                cx.sleep(d).await;
            }
            panic!("{}", SCRIPTED_PANIC)
        }
        Cleanup::IgnoreStop => std::future::pending().await,
    }
}

impl BodySource for ScriptedBodies {
    fn body(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task> {
        let ns = self.nodes.get(&node)?;
        let attempt = Cx::<Run>::new(cx.clone()).attempt() as usize;
        let spec = ns.bodies[(attempt - 1).min(ns.bodies.len() - 1)].clone();
        let serve = if ns.kind == Kind::Service {
            let mut eps = self.episodes.lock().expect("episodes poisoned");
            let e = eps.entry(node).or_insert(0);
            let s = ns.serves[(*e).min(ns.serves.len() - 1)].clone();
            *e += 1;
            Some(s)
        } else {
            None
        };
        // Always async, even for a blocking step: a scripted body's whole
        // behaviour is a wait on the engine clock, and a pool thread cannot
        // wait on virtual time. `R-04` covers the real blocking path.
        Some(Task::Async(Box::pin(scripted_body(
            cx.clone(),
            ns.kind,
            spec,
            serve,
        ))))
    }

    fn cleanup(&self, node: RawKey, cx: &Arc<CxInner>) -> Option<Task> {
        let ns = self.nodes.get(&node)?;
        Some(Task::Async(Box::pin(scripted_cleanup(
            cx.clone(),
            ns.cleanup.clone(),
        ))))
    }

    /// Nothing reads a scripted node's value, so nothing is kept.
    fn store(&self, _node: RawKey, _value: Box<dyn Any + Send + Sync>) {}

    /// A scripted run has no typed output: its values are placeholders, so a
    /// report from one carries `output: None` rather than a fabricated value.
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        None
    }
}
