//! T5 — a scope stops admitting: every in-flight non-service body is cancelled
//! per its cancel mode, backoffs end at once (INV-12), unstarted nodes are
//! skipped, and the shutdown budget starts (T7). Also the external requests
//! and what a request during cleanup means (INV-7).

use super::state::{Cause, Machine, Purpose, St};
use super::{Effect, RunState};
use crate::plan::Kind;
use crate::policy::CancelMode;
use crate::report::TraceKind;

impl Machine {
    /// `Admitting | Steady → Settling` for one scope. Idempotent: a scope
    /// that is already settling or cleaning up is left alone.
    pub(super) fn settle(&mut self, scope: usize, cause: Cause) {
        if !matches!(
            self.scopes[scope].st,
            RunState::Admitting | RunState::Steady
        ) {
            return;
        }
        self.scopes[scope].st = RunState::Settling;
        self.scopes[scope].cause = Some(cause);
        if self.t.scopes[scope].component.is_none() {
            self.emit_run(TraceKind::Settling);
        }
        // T7: the budget starts now, and never outlives the parent's.
        let parent_deadline = self.t.scopes[scope]
            .parent
            .and_then(|p| self.scopes[p].deadline);
        let own = self.t.scopes[scope].shutdown.budget().map(|d| self.now + d);
        let deadline = match (own, parent_deadline) {
            (Some(a), Some(b)) => Some(a.min(b)),
            (a, b) => a.or(b),
        };
        self.scopes[scope].deadline = deadline;
        if let Some(at) = deadline {
            let id = self.timer(Purpose::Budget(scope), at);
            self.scopes[scope].budget_timer = Some(id);
        }
        let because = match cause {
            Cause::Fault(n) => Some(n),
            Cause::Parent(b) => b,
            _ => None,
        };
        for n in self.t.scopes[scope].nodes.clone() {
            match self.slots[n].st {
                St::Pending | St::Waiting => {
                    self.slots[n].st = St::Skipped;
                    self.slots[n].queued = None;
                    self.slots[n].because = because;
                    if let Some(b) = because {
                        let path = self.t.nodes[b].path.clone();
                        self.emit(n, TraceKind::Skipped { because: path });
                    }
                    // A node waiting for its *next* attempt (a zero backoff,
                    // or a grant it never got) still owns the faults of the
                    // attempts that already failed (INV-9).
                    self.flush_faults(n);
                }
                St::Backoff => {
                    // The attempt that would have followed is the one cut
                    // short; the failed one keeps its fault record.
                    let id = self.slots[n].timer.take();
                    self.drop_timer(id);
                    self.slots[n].st = St::Interrupted;
                    self.slots[n].attempt += 1;
                    self.emit(n, TraceKind::Interrupted { held: false });
                    self.flush_faults(n);
                }
                St::Running => self.interrupt(n, because),
                _ => {}
            }
            // A component's inner scope stops admitting with the node, whether
            // the node had started it or not: a component whose inner graph is
            // already up settles with the scope that holds it, and one that
            // never started is skipped so the gates it holds shut can open.
            // `settle` and `skip_scope` are both idempotent, so the arms above
            // may have done it already.
            self.settle_or_skip_inner(n, because);
        }
    }

    /// A component that never started: its inner scope never admitted, so its
    /// nodes are still `Pending`. Left alone they hold the release gate of
    /// everything they import shut (INV-5) and the run cannot finish, so they
    /// are skipped with the component, all the way down.
    pub(super) fn skip_scope(&mut self, scope: usize, because: Option<usize>) {
        if self.scopes[scope].st != RunState::Planned {
            return;
        }
        self.scopes[scope].st = RunState::Ended;
        for n in self.t.scopes[scope].nodes.clone() {
            if matches!(self.slots[n].st, St::Pending | St::Waiting) {
                self.slots[n].st = St::Skipped;
                self.slots[n].queued = None;
                self.slots[n].because = because;
                if let Some(b) = because {
                    let path = self.t.nodes[b].path.clone();
                    self.emit(n, TraceKind::Skipped { because: path });
                }
            }
            if let Some(inner) = self.t.nodes[n].inner {
                self.skip_scope(inner, because);
            }
        }
    }

    /// Cancel one in-flight body per its cancel mode. A blocking body cannot
    /// be cancelled (T7) and a component is settled as a scope.
    fn interrupt(&mut self, n: usize, because: Option<usize>) {
        if self.slots[n].cancelling {
            return;
        }
        let key = self.t.nodes[n].key;
        match self.t.nodes[n].kind {
            Kind::BlockingStep => {}
            Kind::Component => {
                self.settle_or_skip_inner(n, because);
                // The component's own attempt ends here: its inner graph was
                // cut short before it came up. Without this the component
                // would go from `Start(Prepare)` straight to `ReleaseStart`
                // with no terminal observation of its own — an orphan
                // (INV-15).
                self.end_component_attempt(n);
            }
            Kind::Service => {
                // A start body in flight: signal-then-deadline, the service's
                // own cancel mode; the deadline is its stop budget.
                self.slots[n].cancelling = true;
                self.slots[n].signalled = true;
                self.fx.push(Effect::Signal(key));
                let scope = self.t.nodes[n].scope;
                let at = match (
                    self.t.nodes[n].attrs.stop_within,
                    self.scopes[scope].deadline,
                ) {
                    (Some(d), Some(b)) => Some((self.now + d).min(b)),
                    (Some(d), None) => Some(self.now + d),
                    (None, b) => b,
                };
                if let Some(at) = at {
                    let id = self.timer(Purpose::Grace(n), at);
                    self.slots[n].timer = Some(id);
                }
            }
            _ => {
                self.slots[n].cancelling = true;
                let id = self.slots[n].timer.take();
                self.drop_timer(id);
                match self.t.nodes[n].attrs.cancel {
                    CancelMode::Drop => self.fx.push(Effect::Abort(key)),
                    CancelMode::Cooperative(g) => {
                        self.slots[n].signalled = true;
                        self.fx.push(Effect::Signal(key));
                        let id = self.timer(Purpose::Grace(n), self.now + g);
                        self.slots[n].timer = Some(id);
                    }
                }
            }
        }
    }

    /// The cooperative grace ran out: abort.
    pub(super) fn on_grace_timer(&mut self, n: usize) {
        if self.slots[n].st == St::Running && self.slots[n].cancelling {
            self.slots[n].timer = None;
            self.fx.push(Effect::Abort(self.t.nodes[n].key));
        }
    }

    /// `shutdown()`: a normal end from wherever the run is.
    pub(super) fn on_shutdown(&mut self) -> Result<(), &'static str> {
        match self.scopes[0].st {
            RunState::Planned => {
                self.scopes[0].cause = Some(Cause::Shutdown);
                self.end_root();
                Ok(())
            }
            RunState::Admitting | RunState::Steady => {
                self.settle(0, Cause::Shutdown);
                self.try_cleanup(0);
                Ok(())
            }
            RunState::Settling | RunState::Cleanup => {
                self.emit_run(TraceKind::RequestDuringCleanup);
                Ok(())
            }
            RunState::Ended => Err("shutdown after End"),
        }
    }

    /// `cancel()` or a drop of `Running`: the run ends `Cancelled` (INV-10),
    /// and a cleanup already running is never interrupted by it (INV-7).
    pub(super) fn on_cancel(&mut self) -> Result<(), &'static str> {
        match self.scopes[0].st {
            RunState::Planned => {
                self.scopes[0].cause = Some(Cause::Cancel);
                self.end_root();
                Ok(())
            }
            RunState::Admitting | RunState::Steady => {
                self.settle(0, Cause::Cancel);
                self.try_cleanup(0);
                Ok(())
            }
            RunState::Settling | RunState::Cleanup => {
                self.emit_run(TraceKind::RequestDuringCleanup);
                Ok(())
            }
            RunState::Ended => Err("cancel after End"),
        }
    }
}
