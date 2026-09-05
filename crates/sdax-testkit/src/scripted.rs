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
use sdax::host::sim::{SpawnOutcome, FOREIGN};
use sdax::host::{BodySource, CxInner, InstanceId, RawKey, Task, Time};
use sdax::{
    Acquire, At, Body, Child, Cleanup, Cx, Ending, Error, Kind, Plan, Run, Script, Serve, Serving,
    SpawnSpec, Start,
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
    path: String,
    bodies: Vec<Body>,
    serves: Vec<Serve>,
    cleanup: Cleanup,
    spawns: Vec<SpawnSpec>,
}

/// What one scripted body needs to perform its `cx.spawn` directives.
struct Spawning {
    node: String,
    specs: Vec<SpawnSpec>,
    templates: Arc<Vec<(String, RawKey)>>,
    log: Arc<Mutex<Vec<SpawnOutcome>>>,
}

/// A plan's bodies, replaced by what a [`Script`] says they do.
///
/// Nodes are keyed by their **declaration** key, so one script line covers
/// every instance of a template; the serving-episode counter is per instance,
/// because two instances of one service are two services.
pub struct ScriptedBodies {
    nodes: HashMap<RawKey, NodeScript>,
    templates: Arc<Vec<(String, RawKey)>>,
    episodes: Mutex<HashMap<(RawKey, Option<InstanceId>), usize>>,
    spawns: Arc<Mutex<Vec<SpawnOutcome>>>,
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
        Machine::new(plan)?;
        let declared = Machine::declarations(plan);
        let mut nodes = HashMap::new();
        for (key, path, kind) in &declared {
            let p = path.to_string();
            nodes.insert(
                *key,
                NodeScript {
                    kind: *kind,
                    bodies: script
                        .body_of(&p)
                        .map(|b| b.to_vec())
                        .unwrap_or_else(|| vec![Body::default()]),
                    serves: script
                        .serve_of(&p)
                        .map(|s| s.to_vec())
                        .unwrap_or_else(|| vec![Serve::default()]),
                    cleanup: script.cleanup_of(&p).cloned().unwrap_or_default(),
                    spawns: script.spawns_of(&p).map(|s| s.to_vec()).unwrap_or_default(),
                    path: p,
                },
            );
        }
        let templates = declared
            .iter()
            .filter(|(_, _, k)| *k == Kind::Template)
            .map(|(key, path, _)| (path.to_string(), *key))
            .collect();
        Ok(Arc::new(ScriptedBodies {
            nodes,
            templates: Arc::new(templates),
            episodes: Mutex::new(HashMap::new()),
            spawns: Arc::new(Mutex::new(Vec::new())),
        }))
    }

    /// Every scripted `cx.spawn` and what it answered, in order.
    pub fn spawns(&self) -> Vec<SpawnOutcome> {
        self.spawns.lock().expect("spawns poisoned").clone()
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

/// Perform this attempt's `cx.spawn` directives, in time order.
///
/// A directive whose time falls after the body's own ending is dropped: a body
/// that has returned cannot spawn, and the simulator drops it the same way.
/// A template path this plan does not declare stands for a handle from another
/// plan, and is presented as [`FOREIGN`] so the run really answers
/// `SpawnError::ForeignTemplate` (`C-51`).
async fn do_spawns(
    cx: &Cx<Run>,
    inner: &Arc<CxInner>,
    sp: &Spawning,
    start: Time,
    end: Option<Time>,
) -> Vec<(Child, Option<At>, bool)> {
    let mut order: Vec<(Time, usize)> = sp
        .specs
        .iter()
        .enumerate()
        .map(|(i, s)| (target(s.at, start), i))
        .filter(|(t, _)| end.map(|e| *t <= e).unwrap_or(true))
        .collect();
    order.sort();
    let mut out = Vec::new();
    for (t, i) in order {
        wait_until(cx, t).await;
        let spec = &sp.specs[i];
        let key = sp
            .templates
            .iter()
            .find(|(p, _)| *p == spec.template)
            .map(|(_, k)| *k)
            .unwrap_or(FOREIGN);
        let result = inner.spawn_instance(key, Box::new(()));
        sp.log.lock().expect("spawns poisoned").push(SpawnOutcome {
            node: sp.node.clone(),
            template: spec.template.clone(),
            result: result.as_ref().map(|c| c.id()).map_err(|e| *e),
        });
        if let Ok(child) = result {
            out.push((child, spec.stop, spec.await_ready));
        }
    }
    out
}

/// One attempt of a scripted prepare, run or start body.
async fn scripted_body(
    inner: Arc<CxInner>,
    kind: Kind,
    spec: Body,
    serve: Option<Serve>,
    spawning: Spawning,
) -> Result<(), Error> {
    let cx: Cx<Run> = Cx::new(inner.clone());
    let start = cx.now();
    let end = match &spec.ending {
        Ending::Ok(at) | Ending::Fail(at, _) | Ending::Panic(at) => Some(target(*at, start)),
        Ending::Pending => None,
    };
    let children = do_spawns(&cx, &inner, &spawning, start, end).await;
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
    // INV-17: a body that awaits `Child::ready()` returns only once the
    // instances it awaits have answered. An instance that ended before it was
    // ready answers `Err`, which is an answer and not a fault of this body.
    for (child, _, awaited) in &children {
        if *awaited {
            let _ = child.ready().await;
        }
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
        let stops: Vec<(Child, At)> = children
            .into_iter()
            .filter_map(|(c, at, _)| at.map(|a| (c, a)))
            .collect();
        let serving = Serving::new((), scripted_serve(inner.clone(), s, stops));
        let (handle, fut) = serving.into_parts();
        inner.put_output(Box::new(Arc::new(handle)));
        inner.put_serve(fut);
        let _ = start_cx;
    } else if !kind.can_hold() {
        inner.put_output(Box::new(Arc::new(())));
    }
    Ok(())
}

/// One serving episode, with the instance stops the start body's directives
/// asked for running beside it.
async fn scripted_serve(
    inner: Arc<CxInner>,
    spec: Serve,
    stops: Vec<(Child, At)>,
) -> Result<(), Error> {
    let cx: Cx<Run> = Cx::new(inner.clone());
    let start = cx.now();
    if stops.is_empty() {
        return serve_body(inner, spec).await;
    }
    let stopper = async move {
        let mut order: Vec<(Time, Child)> = stops
            .into_iter()
            .map(|(c, at)| (target(at, start), c))
            .collect();
        order.sort_by_key(|(t, _)| *t);
        for (t, child) in order {
            wait_until(&cx, t).await;
            child.stop();
        }
        std::future::pending::<Result<(), Error>>().await
    };
    let serve = serve_body(inner, spec);
    // The serve future decides the episode; the stopper only ever runs
    // beside it, so the first of the two to finish is always the serve.
    let mut stopper = std::pin::pin!(stopper);
    let mut serve = std::pin::pin!(serve);
    std::future::poll_fn(move |cx| {
        let _ = std::future::Future::poll(stopper.as_mut(), cx);
        std::future::Future::poll(serve.as_mut(), cx)
    })
    .await
}

/// The serve behaviour itself.
async fn serve_body(inner: Arc<CxInner>, spec: Serve) -> Result<(), Error> {
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
    fn body(&self, node: RawKey, instance: Option<InstanceId>, cx: &Arc<CxInner>) -> Option<Task> {
        let ns = self.nodes.get(&node)?;
        let attempt = Cx::<Run>::new(cx.clone()).attempt() as usize;
        let spec = ns.bodies[(attempt - 1).min(ns.bodies.len() - 1)].clone();
        let serve = if ns.kind == Kind::Service {
            let mut eps = self.episodes.lock().expect("episodes poisoned");
            let e = eps.entry((node, instance)).or_insert(0);
            let s = ns.serves[(*e).min(ns.serves.len() - 1)].clone();
            *e += 1;
            Some(s)
        } else {
            None
        };
        let spawning = Spawning {
            node: ns.path.clone(),
            specs: ns.spawns.clone(),
            templates: self.templates.clone(),
            log: self.spawns.clone(),
        };
        // Always async, even for a blocking step: a scripted body's whole
        // behaviour is a wait on the engine clock, and a pool thread cannot
        // wait on virtual time. `R-04` covers the real blocking path.
        Some(Task::Async(Box::pin(scripted_body(
            cx.clone(),
            ns.kind,
            spec,
            serve,
            spawning,
        ))))
    }

    fn cleanup(
        &self,
        node: RawKey,
        _instance: Option<InstanceId>,
        cx: &Arc<CxInner>,
    ) -> Option<Task> {
        let ns = self.nodes.get(&node)?;
        Some(Task::Async(Box::pin(scripted_cleanup(
            cx.clone(),
            ns.cleanup.clone(),
        ))))
    }

    /// Nothing reads a scripted node's value, so nothing is kept.
    fn store(
        &self,
        _node: RawKey,
        _instance: Option<InstanceId>,
        _value: Box<dyn Any + Send + Sync>,
    ) {
    }

    /// A scripted instance has no slots of its own: its bodies read nothing
    /// and the per-instance input is never looked at.
    fn open_instance(
        &self,
        _template: RawKey,
        _parent: Option<InstanceId>,
        _id: InstanceId,
        _input: Box<dyn Any + Send + Sync>,
    ) {
    }

    fn close_instance(&self, _id: InstanceId) {}

    /// A scripted run has no typed output: its values are placeholders, so a
    /// report from one carries `output: None` rather than a fabricated value.
    fn export(&self) -> Option<Box<dyn Any + Send + Sync>> {
        None
    }
}
