//! What a simulated run is told: per-body outcomes at virtual times, external
//! requests at virtual times, and the schedule's preference among nodes that
//! become eligible together. The notation follows
//! `sdax-v1/B/CanonicalTests.md` § 0.

use std::time::Duration;

/// When something happens: at an absolute virtual time, or a duration after
/// the body started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum At {
    /// An absolute time on the virtual clock, from its origin.
    Tick(Duration),
    /// This long after the body was spawned.
    After(Duration),
}

impl At {
    /// `@t`, in seconds.
    pub fn tick(secs: f64) -> At {
        At::Tick(Duration::from_secs_f64(secs))
    }
    /// `@+d`, in seconds.
    pub fn plus(secs: f64) -> At {
        At::After(Duration::from_secs_f64(secs))
    }
}

/// How a prepare, run or start body ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Ending {
    /// Returns `Ok` (a service: returns `Serving`).
    Ok(At),
    /// Returns `Err` with this message.
    Fail(At, String),
    /// Panics.
    Panic(At),
    /// Never returns.
    Pending,
}

/// One attempt of a prepare, run or start body.
///
/// For a resource or an effect, `held` is when `hold` registers; an `ok`
/// with no explicit `held` registers in the completing poll, as `hold` does.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Body {
    /// When `hold` registered, if the body says so explicitly.
    pub held: Option<At>,
    /// How the attempt ends.
    pub ending: Ending,
}

impl Body {
    /// `ok@t`.
    pub fn ok(at: At) -> Body {
        Body {
            held: None,
            ending: Ending::Ok(at),
        }
    }
    /// `fail@t "msg"`.
    pub fn fail(at: At, msg: &str) -> Body {
        Body {
            held: None,
            ending: Ending::Fail(at, msg.to_string()),
        }
    }
    /// `panic@t`.
    pub fn panic(at: At) -> Body {
        Body {
            held: None,
            ending: Ending::Panic(at),
        }
    }
    /// `pending`.
    pub fn pending() -> Body {
        Body {
            held: None,
            ending: Ending::Pending,
        }
    }
    /// `held@t1 …`: the effect completed inside `hold` at `t1`.
    ///
    /// A body registers its value before it returns, so a hold at or after
    /// the body's own ending happens at the ending instant instead.
    pub fn held(mut self, at: At) -> Body {
        self.held = Some(at);
        self
    }
}

impl Default for Body {
    /// Bodies take 0 ticks unless the script says otherwise.
    fn default() -> Body {
        Body::ok(At::plus(0.0))
    }
}

/// How a service's serve future behaves, per serving episode.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Serve {
    /// Returns `Ok` by itself.
    Ok(At),
    /// Returns `Err` by itself.
    Err(At, String),
    /// Never returns, stop signal or not.
    IgnoreStop,
    /// Returns `Ok` this long after the stop signal.
    StopsAfter(Duration),
}

impl Default for Serve {
    fn default() -> Serve {
        Serve::StopsAfter(Duration::ZERO)
    }
}

/// How a release or compensation body ends.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Cleanup {
    /// `ok@+d`.
    Ok(Duration),
    /// `fail@+d "msg"`.
    Fail(Duration, String),
    /// `panic` (after this long).
    Panic(Duration),
    /// Never completes.
    IgnoreStop,
}

impl Default for Cleanup {
    fn default() -> Cleanup {
        Cleanup::Ok(Duration::ZERO)
    }
}

/// What a body does about a template it declared (F1).
///
/// A scripted `cx.spawn`: the start body asks for an instance at `at`,
/// optionally awaits its readiness before returning (INV-17), and the serve
/// future asks it to stop at `stop`. Every attempt of the body runs its
/// directives again, exactly as a real body would.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnSpec {
    /// The template's path, as `inspect()` shows it.
    pub template: String,
    /// When the body asks for the instance, measured from the body's start.
    pub at: At,
    /// Whether the body awaits `Child::ready()` before it returns.
    pub await_ready: bool,
    /// When the serve future asks this instance to stop, measured from the
    /// start of the serving episode.
    pub stop: Option<At>,
}

impl SpawnSpec {
    /// `spawn Template @+d`.
    pub fn new(template: &str, at: At) -> SpawnSpec {
        SpawnSpec {
            template: template.to_string(),
            at,
            await_ready: false,
            stop: None,
        }
    }
    /// Await `Child::ready()` before the body returns (INV-17).
    pub fn awaited(mut self) -> SpawnSpec {
        self.await_ready = true;
        self
    }
    /// Ask this instance to stop, this long into the serving episode.
    pub fn stopped(mut self, at: At) -> SpawnSpec {
        self.stop = Some(at);
        self
    }
}

/// An external request to the run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Request {
    /// `shutdown()`.
    Shutdown,
    /// `cancel()`.
    Cancel,
}

/// The preference among nodes that become eligible in the same step.
///
/// The machine takes grants in FIFO order among waiters (T1); when several
/// nodes become need-ready together, this decides their order in the queue.
/// It is an input of the run, so a test can force either interleaving and
/// get it every time (INV-14).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum Schedule {
    /// Declaration order among simultaneously eligible nodes.
    #[default]
    Fifo,
    /// These nodes first, in this order; the rest in declaration order.
    Order(Vec<String>),
}

impl Schedule {
    /// `order [N1, N2, …]`.
    pub fn order<S: AsRef<str>>(nodes: impl IntoIterator<Item = S>) -> Schedule {
        Schedule::Order(nodes.into_iter().map(|s| s.as_ref().to_string()).collect())
    }
}

/// Everything a simulated run is told. Nodes are named by path
/// (`Net/Transport`); an unknown path is reported when the run is built.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Script {
    pub(crate) bodies: Vec<(String, Vec<Body>)>,
    pub(crate) spawns: Vec<(String, Vec<SpawnSpec>)>,
    pub(crate) serves: Vec<(String, Vec<Serve>)>,
    pub(crate) cleanups: Vec<(String, Cleanup)>,
    pub(crate) requests: Vec<(Duration, Request)>,
    pub(crate) schedule: Schedule,
}

impl Script {
    /// A script in which every body takes 0 ticks and succeeds.
    pub fn new() -> Script {
        Script::default()
    }

    /// The outcome of a node's prepare, run or start body, attempt by
    /// attempt; the last entry repeats for later attempts. A later call for
    /// the same node replaces the earlier one.
    pub fn body(mut self, node: &str, attempts: impl IntoIterator<Item = Body>) -> Script {
        let attempts: Vec<Body> = attempts.into_iter().collect();
        self.bodies.retain(|(n, _)| n != node);
        self.bodies.push((node.to_string(), attempts));
        self
    }

    /// One attempt outcome for a node's body.
    pub fn prepare(self, node: &str, body: Body) -> Script {
        self.body(node, [body])
    }

    /// The instances a node's body creates (F1). A later call for the same
    /// node replaces the earlier one.
    pub fn spawns(mut self, node: &str, specs: impl IntoIterator<Item = SpawnSpec>) -> Script {
        self.spawns.retain(|(n, _)| n != node);
        self.spawns
            .push((node.to_string(), specs.into_iter().collect()));
        self
    }

    /// What this script says a node's body spawns.
    pub fn spawns_of(&self, node: &str) -> Option<&[SpawnSpec]> {
        self.spawns
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, s)| s.as_slice())
    }

    /// A service's serve behaviour, episode by episode (restarts are later
    /// episodes); the last entry repeats.
    pub fn serve(mut self, node: &str, episodes: impl IntoIterator<Item = Serve>) -> Script {
        self.serves.retain(|(n, _)| n != node);
        self.serves
            .push((node.to_string(), episodes.into_iter().collect()));
        self
    }

    /// How a node's release or compensation ends.
    pub fn cleanup(mut self, node: &str, c: Cleanup) -> Script {
        self.cleanups.retain(|(n, _)| n != node);
        self.cleanups.push((node.to_string(), c));
        self
    }

    /// `@t request`. Requests at the origin are delivered before the run's
    /// first step, so `@0 cancel` is a cancel before the first poll.
    pub fn at(mut self, secs: f64, r: Request) -> Script {
        self.requests.push((Duration::from_secs_f64(secs), r));
        self
    }

    /// The schedule preference.
    pub fn schedule(mut self, s: Schedule) -> Script {
        self.schedule = s;
        self
    }

    /// The schedule this script carries.
    pub fn schedule_ref(&self) -> &Schedule {
        &self.schedule
    }

    /// What this script says a node's body does, attempt by attempt.
    ///
    /// A harness that runs the script on a real runtime rather than on the
    /// stepping simulator reads it back through these three, and through
    /// [`requests`](Self::requests).
    pub fn body_of(&self, node: &str) -> Option<&[Body]> {
        self.bodies
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, b)| b.as_slice())
    }

    /// What this script says a service's serve future does, episode by
    /// episode.
    pub fn serve_of(&self, node: &str) -> Option<&[Serve]> {
        self.serves
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, s)| s.as_slice())
    }

    /// What this script says a node's release or compensation does.
    pub fn cleanup_of(&self, node: &str) -> Option<&Cleanup> {
        self.cleanups
            .iter()
            .find(|(n, _)| n == node)
            .map(|(_, c)| c)
    }

    /// The external requests, in the order written.
    pub fn requests(&self) -> &[(Duration, Request)] {
        &self.requests
    }

    /// Every node path the script names, for validation against a plan.
    pub fn named_nodes(&self) -> Vec<&str> {
        self.bodies
            .iter()
            .map(|(n, _)| n.as_str())
            .chain(self.serves.iter().map(|(n, _)| n.as_str()))
            .chain(self.cleanups.iter().map(|(n, _)| n.as_str()))
            .chain(self.spawns.iter().map(|(n, _)| n.as_str()))
            .collect()
    }
}
