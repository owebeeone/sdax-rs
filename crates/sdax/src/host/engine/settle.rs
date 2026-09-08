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
    /// Whether this scope is the run's own: not a component's inner scope and
    /// not a template instance. Only the root emits `Settling` and only the
    /// root arms its budget at the settle (T7; a nested scope arms it when its
    /// release graph may open, T7a).
    pub(super) fn is_root(&self, scope: usize) -> bool {
        self.t.scopes[scope].component.is_none() && self.t.scopes[scope].instance.is_none()
    }

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
        let root = self.is_root(scope);
        if root {
            self.emit_run(TraceKind::Settling);
        }
        // T7: the root's budget starts now, and a nested scope's starts when
        // its release graph may open (`open_component`), never here. An inner
        // scope can settle long before INV-5 lets its releases run — a
        // `terminal` service finishing, an inner fault, a parent that reached
        // it — and arming the clock at the settle spent the whole budget
        // waiting for a gate, so every inner release was started and abandoned
        // in the same instant with the parent's budget untouched. Until then
        // the inner scope is bounded by the parent's deadline alone, which is
        // what its own in-flight bodies are cancelled against.
        if root {
            self.arm_scope_budget(scope);
        } else {
            self.scopes[scope].deadline = self.t.scopes[scope]
                .parent
                .and_then(|p| self.scopes[p].deadline);
        }
        let because = match cause {
            Cause::Fault(n) => Some(n),
            Cause::Parent(b) => b,
            _ => None,
        };
        for n in self.t.scopes[scope].nodes.clone() {
            self.keep_template_live(n);
            match self.slots[n].st {
                St::Pending | St::Waiting => {
                    self.slots[n].st = St::Skipped;
                    self.slots[n].queued = None;
                    self.slots[n].because = because;
                    let path = because.map(|b| self.t.nodes[b].path.clone());
                    self.emit(n, TraceKind::Skipped { because: path });
                    // A node waiting for its *next* attempt (a zero backoff,
                    // or a grant it never got) still owns the faults of the
                    // attempts that already failed (INV-9).
                    self.flush_faults(n);
                }
                St::Backoff => {
                    // The initialization attempt, or serving episode, that
                    // would have followed is the one cut short; the failed
                    // one keeps its fault record.
                    let id = self.slots[n].timer.take();
                    self.drop_timer(id);
                    self.slots[n].st = St::Interrupted;
                    if self.t.nodes[n].kind == Kind::Service && self.slots[n].initialized {
                        self.slots[n].episode = self.slots[n].restarts + 1;
                    } else {
                        self.slots[n].attempt += 1;
                    }
                    self.emit(n, TraceKind::Interrupted { held: false });
                    self.flush_faults(n);
                }
                St::Running | St::Publishing => self.interrupt(n, because),
                // A between-attempt release is shielded from cancellation,
                // but the scope budget that just became active still bounds
                // it and must become visible through its existing context.
                St::RetryRelease => self.fx.push(Effect::RefreshDeadline(self.t.nodes[n].key)),
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
            self.keep_template_live(n);
            if matches!(self.slots[n].st, St::Pending | St::Waiting) {
                self.slots[n].st = St::Skipped;
                self.slots[n].queued = None;
                self.slots[n].because = because;
                let path = because.map(|b| self.t.nodes[b].path.clone());
                self.emit(n, TraceKind::Skipped { because: path });
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
            Kind::Join => {
                self.slots[n].st = St::Skipped;
                self.slots[n].because = because;
                self.emit(
                    n,
                    TraceKind::Skipped {
                        because: because.map(|b| self.t.nodes[b].path.clone()),
                    },
                );
            }
            // T5: a blocking body is told to stop, and never aborted (T7) — a
            // thread cannot be dropped. `cx.is_stopping()` is how a blocking
            // body learns the run is ending (contract § 5, "all phases"); with
            // no `Signal` it read `false` for ever and the request never
            // reached the body at all. Nothing bounds the wait but the budget,
            // which abandons the node and leaks the thread (T7).
            Kind::BlockingStep => {
                self.slots[n].cancelling = true;
                self.slots[n].signalled = true;
                self.fx.push(Effect::Signal(key));
            }
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
                // An initializer in flight: signal-then-deadline, the service's
                // own cancel mode; the deadline is its stop budget.
                self.slots[n].cancelling = true;
                self.slots[n].signalled = true;
                let id = self.slots[n].timer.take();
                self.drop_timer(id);
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
