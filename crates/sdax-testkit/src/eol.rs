//! Queries over a [`Trace`] in the observation vocabulary of the canonical
//! tests (EOL v1, `CanonicalTests.md` § 0): `start(N)`, `ready(N)`,
//! `cleanup(N)` as an interval, `before`, `never`, `max-concurrent`.
//!
//! Times are seconds on the virtual clock, as `f64`, so a test reads
//! `assert_eq!(t.ready("Db"), Some(1.0))`. Order questions use positions in
//! the trace (`pos`), because two events at one tick still have an order.

use sdax::host::InstanceId;
use sdax::{Outcome, Phase, Trace, TraceEvent, TraceKind};

/// The trace, queried.
#[derive(Clone, Copy)]
pub struct Eol<'a>(pub &'a Trace);

/// Seconds on the virtual clock.
pub fn secs_of(e: &TraceEvent) -> f64 {
    e.at.as_nanos() as f64 / 1e9
}

/// `start(N.*)`.
pub fn is_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Start(phase) if *phase != Phase::Serve)
}
/// The start of one serving episode.
pub fn is_episode_start(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Start(Phase::Serve))
}
/// `held(N)`.
pub fn is_held(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Held)
}
/// `ok(N.prepare)` / `ready(N)`.
pub fn is_ready(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Ready)
}
/// `fail(N.*)` of a body (not a cleanup).
pub fn is_fail(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::Fail(Phase::Prepare | Phase::Run | Phase::Serve, _)
    )
}
/// `cancelled(N.*)`.
pub fn is_interrupted(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Interrupted { .. })
}
/// The start of `cleanup(N)`: a release, a compensation or a stop request.
pub fn is_cleanup_start(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::ReleaseStart
            | TraceKind::CompensateStart
            | TraceKind::RecoveryStart
            | TraceKind::StopRequested
    )
}
/// The end of `cleanup(N)`, however it ended.
pub fn is_cleanup_end(k: &TraceKind) -> bool {
    matches!(
        k,
        TraceKind::ReleaseOk
            | TraceKind::ReleaseFail(_)
            | TraceKind::RecoveryOk
            | TraceKind::RecoveryFail(_)
            | TraceKind::CompensateOk
            | TraceKind::CompensateFail(_)
            | TraceKind::Stopped
            | TraceKind::Fail(Phase::Stop, _)
            | TraceKind::Abandoned
    )
}
/// `stop(N)` requested.
pub fn is_stop_requested(k: &TraceKind) -> bool {
    matches!(k, TraceKind::StopRequested)
}
/// The serve future returned `Ok`.
pub fn is_stopped(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Stopped)
}
/// `abandon(N)`.
pub fn is_abandoned(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Abandoned)
}
/// The node was reported ambiguous.
pub fn is_ambiguous(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Ambiguous)
}
/// The node was skipped.
pub fn is_skipped(k: &TraceKind) -> bool {
    matches!(k, TraceKind::Skipped { .. })
}

impl<'a> Eol<'a> {
    fn events(&self, node: &str) -> impl Iterator<Item = (usize, &'a TraceEvent)> {
        let node = node.to_string();
        self.0
            .events
            .iter()
            .enumerate()
            .filter(move |(_, e)| e.node.as_ref().map(|p| *p == *node.as_str()) == Some(true))
    }

    /// The instance a trace event belongs to, from its record order: the
    /// first instance named along the path from the root.
    fn instance_of(e: &TraceEvent) -> Option<InstanceId> {
        e.order
            .as_ref()
            .and_then(|o| o.steps.iter().find_map(|(_, i)| *i))
    }

    /// Events of `node` **in one instance**. Several live instances share a
    /// path, so a query that does not say which one merges them.
    fn events_of(
        &self,
        node: &str,
        inst: InstanceId,
    ) -> impl Iterator<Item = (usize, &'a TraceEvent)> {
        self.events(node)
            .filter(move |(_, e)| Self::instance_of(e) == Some(inst))
    }

    /// Position of the first matching event of one instance's copy of `node`.
    pub fn pos_of(
        &self,
        want: fn(&TraceKind) -> bool,
        node: &str,
        inst: InstanceId,
    ) -> Option<usize> {
        self.events_of(node, inst)
            .find(|(_, e)| want(&e.kind))
            .map(|(i, _)| i)
    }

    /// Time of the first matching event of one instance's copy of `node`.
    pub fn at_of(&self, want: fn(&TraceKind) -> bool, node: &str, inst: InstanceId) -> Option<f64> {
        self.events_of(node, inst)
            .find(|(_, e)| want(&e.kind))
            .map(|(_, e)| secs_of(e))
    }

    /// The instances a template created, in the order they were spawned.
    pub fn spawned(&self, template: &str) -> Vec<InstanceId> {
        self.events(template)
            .filter_map(|(_, e)| match e.kind {
                TraceKind::InstanceSpawned(id) => Some(id),
                _ => None,
            })
            .collect()
    }

    /// The instances of a template that ended, and how.
    pub fn instance_ends(&self, template: &str) -> Vec<(InstanceId, Outcome)> {
        self.events(template)
            .filter_map(|(_, e)| match e.kind {
                TraceKind::InstanceEnded(id, o) => Some((id, o)),
                _ => None,
            })
            .collect()
    }

    /// When one instance of a template ended.
    pub fn instance_end_at(&self, template: &str, inst: InstanceId) -> Option<f64> {
        self.events(template)
            .find(|(_, e)| matches!(e.kind, TraceKind::InstanceEnded(i, _) if i == inst))
            .map(|(_, e)| secs_of(e))
    }

    /// Position in the trace of the first event of `node` matching `want`.
    /// `None` sorts after every `Some`, so `a < b` reads "a happens, and
    /// before b" only when both happened; check presence separately.
    pub fn pos(&self, want: fn(&TraceKind) -> bool, node: &str) -> Option<usize> {
        self.events(node)
            .find(|(_, e)| want(&e.kind))
            .map(|(i, _)| i)
    }

    /// Position of the last matching event.
    pub fn last_pos(&self, want: fn(&TraceKind) -> bool, node: &str) -> Option<usize> {
        self.events(node)
            .filter(|(_, e)| want(&e.kind))
            .last()
            .map(|(i, _)| i)
    }

    /// Time of the first matching event.
    pub fn at(&self, want: fn(&TraceKind) -> bool, node: &str) -> Option<f64> {
        self.events(node)
            .find(|(_, e)| want(&e.kind))
            .map(|(_, e)| secs_of(e))
    }

    /// Time of the last matching event.
    pub fn last_at(&self, want: fn(&TraceKind) -> bool, node: &str) -> Option<f64> {
        self.events(node)
            .filter(|(_, e)| want(&e.kind))
            .last()
            .map(|(_, e)| secs_of(e))
    }

    /// How many events of `node` match.
    pub fn count(&self, want: fn(&TraceKind) -> bool, node: &str) -> usize {
        self.events(node).filter(|(_, e)| want(&e.kind)).count()
    }

    /// `start(N)` (first attempt).
    pub fn start(&self, node: &str) -> Option<f64> {
        self.at(is_start, node)
    }
    /// The start of attempt `k`.
    pub fn start_attempt(&self, node: &str, k: u32) -> Option<f64> {
        self.events(node)
            .find(|(_, e)| is_start(&e.kind) && e.order.as_ref().map(|o| o.attempt) == Some(k))
            .map(|(_, e)| secs_of(e))
    }
    /// How many attempts started.
    pub fn attempts(&self, node: &str) -> u32 {
        self.events(node)
            .filter(|(_, e)| is_start(&e.kind))
            .filter_map(|(_, e)| e.order.as_ref().map(|o| o.attempt))
            .max()
            .unwrap_or(0)
    }
    /// `held(N)`.
    pub fn held(&self, node: &str) -> Option<f64> {
        self.at(is_held, node)
    }
    /// `ok(N.prepare)` / `ready(N)`.
    pub fn ready(&self, node: &str) -> Option<f64> {
        self.at(is_ready, node)
    }
    /// The first body failure.
    pub fn fail(&self, node: &str) -> Option<f64> {
        self.at(is_fail, node)
    }
    /// `cancelled(N)`, with its `held` flag.
    pub fn interrupted(&self, node: &str) -> Option<bool> {
        self.events(node).find_map(|(_, e)| match e.kind {
            TraceKind::Interrupted { held } => Some(held),
            _ => None,
        })
    }
    /// Whether the node was reported ambiguous.
    pub fn ambiguous(&self, node: &str) -> bool {
        self.pos(is_ambiguous, node).is_some()
    }
    /// Whether the node was skipped at all — with or without a cause node.
    pub fn skipped(&self, node: &str) -> bool {
        self.pos(is_skipped, node).is_some()
    }
    /// The cause node of `Skipped{because}`. `None` both when the node was
    /// never skipped and when the run itself ended its eligibility (a request,
    /// a `Finite` scope, a `terminal` service); [`Eol::skipped`] tells them
    /// apart.
    pub fn skipped_because(&self, node: &str) -> Option<String> {
        self.events(node).find_map(|(_, e)| match &e.kind {
            TraceKind::Skipped { because } => because.as_ref().map(|b| b.to_string()),
            _ => None,
        })
    }
    /// The start of `cleanup(N)`.
    pub fn cleanup_start(&self, node: &str) -> Option<f64> {
        self.at(is_cleanup_start, node)
    }
    /// The end of `cleanup(N)` (the last, when a retry released earlier).
    pub fn cleanup_end(&self, node: &str) -> Option<f64> {
        self.last_at(is_cleanup_end, node)
    }
    /// `stop(N)` requested.
    pub fn stop_requested(&self, node: &str) -> Option<f64> {
        self.at(is_stop_requested, node)
    }
    /// A `Stopped` strictly before `t` (a service that finished by itself).
    pub fn stopped_before(&self, t: f64, node: &str) -> Option<f64> {
        self.events(node)
            .filter(|(_, e)| is_stopped(&e.kind))
            .map(|(_, e)| secs_of(e))
            .find(|&s| s < t)
    }
    /// `abandon(N)`.
    pub fn abandoned(&self, node: &str) -> Option<f64> {
        self.at(is_abandoned, node)
    }
    /// `end(run)`.
    pub fn end(&self) -> Option<(f64, Outcome)> {
        self.0.events.iter().find_map(|e| match e.kind {
            TraceKind::End(o) => Some((secs_of(e), o)),
            _ => None,
        })
    }
    /// When the run stopped admitting.
    pub fn settling(&self) -> Option<f64> {
        self.0
            .events
            .iter()
            .find(|e| matches!(e.kind, TraceKind::Settling))
            .map(secs_of)
    }
    /// Whether a run-level event of this kind exists.
    pub fn has_run_event(&self, want: fn(&TraceKind) -> bool) -> bool {
        self.0
            .events
            .iter()
            .any(|e| e.node.is_none() && want(&e.kind))
    }

    /// The body intervals `[start, end)` of a node, per attempt.
    fn intervals(&self, node: &str) -> Vec<(usize, usize)> {
        let mut out = Vec::new();
        let mut open: Option<usize> = None;
        for (i, e) in self.events(node) {
            if is_start(&e.kind) {
                open = Some(i);
            } else if open.is_some()
                && (is_ready(&e.kind)
                    || is_fail(&e.kind)
                    || is_interrupted(&e.kind)
                    || is_ambiguous(&e.kind)
                    || is_abandoned(&e.kind))
            {
                out.push((open.take().expect("open"), i));
            }
        }
        if let Some(s) = open {
            out.push((s, usize::MAX));
        }
        out
    }

    /// `max-concurrent({N…})`: the most bodies of these nodes in flight at
    /// once, by trace position.
    pub fn max_concurrent(&self, nodes: &[&str]) -> usize {
        let mut best = 0;
        let all: Vec<(usize, usize)> = nodes.iter().flat_map(|n| self.intervals(n)).collect();
        for &(s, _) in &all {
            let live = all.iter().filter(|&&(a, b)| a <= s && s < b).count();
            best = best.max(live);
        }
        best
    }

    /// How many body intervals of `a` overlap a body interval of `b`.
    pub fn overlap_count(&self, a: &[&str], b: &[&str]) -> usize {
        let ia: Vec<(usize, usize)> = a.iter().flat_map(|n| self.intervals(n)).collect();
        let ib: Vec<(usize, usize)> = b.iter().flat_map(|n| self.intervals(n)).collect();
        ia.iter()
            .filter(|&&(s1, e1)| ib.iter().any(|&(s2, e2)| s1 < e2 && s2 < e1))
            .count()
    }

    /// The trace, one line per event, for a failure message.
    pub fn render(&self) -> String {
        let mut s = String::new();
        for (i, e) in self.0.events.iter().enumerate() {
            let node = e.node.as_ref().map(|p| p.to_string()).unwrap_or_default();
            let attempt = e.order.as_ref().map(|o| o.attempt).unwrap_or(0);
            s.push_str(&format!(
                "{i:3}  t={:<6} {:<24} #{attempt} {:?}\n",
                secs_of(e),
                node,
                e.kind
            ));
        }
        s
    }
}
