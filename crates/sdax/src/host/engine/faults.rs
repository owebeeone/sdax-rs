//! T2–T4 — what a body's outcome means: hold, ready, fail, retry with
//! backoff, timeouts, ambiguity (INV-11), a service's serve outcome and its
//! restarts, and how an inner scope's fault reaches its component.

use super::state::{Cause, Machine, Purpose, St};
use super::{Effect, RunState};
use crate::plan::Kind;
use crate::policy::{Ambiguity, Backoff, Policy};
use crate::report::{FaultKind, FaultLabel, Phase, TraceKind};
use crate::view::NodePath;
use std::time::Duration;

/// The fault a component carries when one of its inner nodes faulted.
#[derive(Debug)]
struct InnerFault(NodePath);

impl std::fmt::Display for InnerFault {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "inner node {} faulted", self.0)
    }
}
impl std::error::Error for InnerFault {}

/// The wait before the attempt after `failures` failures.
pub(super) fn backoff_wait(b: Option<Backoff>, failures: u32) -> Duration {
    match b {
        None => Duration::ZERO,
        Some(Backoff::Fixed(d)) => d,
        Some(Backoff::Exponential {
            initial,
            factor,
            cap,
        }) => {
            let mult = factor
                .checked_pow(failures.saturating_sub(1))
                .unwrap_or(u32::MAX);
            initial.saturating_mul(mult).min(cap)
        }
    }
}

impl Machine {
    pub(super) fn admitting(&self, scope: usize) -> bool {
        matches!(
            self.scopes[scope].st,
            RunState::Admitting | RunState::Steady
        )
    }

    fn body_phase(&self, n: usize) -> Phase {
        match self.t.nodes[n].kind {
            Kind::Resource | Kind::Effect | Kind::Component => Phase::Prepare,
            _ => Phase::Run,
        }
    }

    fn label(kind: &FaultKind) -> FaultLabel {
        kind.label()
    }

    // ------------------------------------------------------------ T2, T3

    pub(super) fn on_held(&mut self, n: usize) -> Result<(), &'static str> {
        if self.slots[n].st != St::Running {
            return Err("Held for a node whose body is not in flight");
        }
        self.slots[n].held = true;
        self.emit(n, TraceKind::Held);
        Ok(())
    }

    pub(super) fn on_ok(&mut self, n: usize) -> Result<(), &'static str> {
        match self.slots[n].st {
            // After an `Abort` the body's own outcome stands: the abort lands
            // between polls, and this body had already finished. After a
            // `Signal` the return is the answer to the cancel (C-60).
            St::Running if self.slots[n].signalled => self.finish_interrupted(n),
            St::Running => self.ready(n),
            St::Releasing => {
                self.slots[n].st = St::Released;
                self.emit(n, TraceKind::ReleaseOk);
                self.after_cleanup(n);
            }
            St::Compensating => {
                self.slots[n].st = St::Compensated;
                self.emit(n, TraceKind::CompensateOk);
                self.after_cleanup(n);
            }
            St::RetryRelease => {
                let kind = if self.t.nodes[n].kind == Kind::Effect {
                    TraceKind::CompensateOk
                } else {
                    TraceKind::ReleaseOk
                };
                self.emit(n, kind);
                self.resume_retry(n);
            }
            St::Abandoned => {}
            _ if self.t.nodes[n].kind == Kind::BlockingStep => {}
            _ => return Err("NodeOk for a node with no body in flight"),
        }
        Ok(())
    }

    fn ready(&mut self, n: usize) {
        if self.t.nodes[n].kind != Kind::Service {
            self.release_grants(n);
        }
        let id = self.slots[n].timer.take();
        self.drop_timer(id);
        self.slots[n].cancelling = false;
        self.slots[n].timing_out = false;
        self.slots[n].st = St::Ready;
        self.slots[n].faults.clear();
        self.emit(n, TraceKind::Ready);
        self.after_settle(n);
    }

    // ------------------------------------------------------------ T4

    pub(super) fn on_err(&mut self, n: usize, kind: FaultKind) -> Result<(), &'static str> {
        match self.slots[n].st {
            St::Running if self.slots[n].signalled => {
                // INV-10: the engine cancelled this body, so whatever it
                // returns now is not a fault. A panic is still observed.
                if Self::label(&kind) == FaultLabel::Panic {
                    let phase = self.body_phase(n);
                    self.emit(n, TraceKind::Fail(phase, FaultLabel::Panic));
                }
                self.finish_interrupted(n);
            }
            St::Running => {
                let phase = self.body_phase(n);
                self.fail_attempt(n, phase, kind);
            }
            St::Releasing => {
                self.slots[n].st = St::ReleaseFailed;
                self.emit(n, TraceKind::ReleaseFail(Self::label(&kind)));
                let fault = self.fault(n, Phase::ReleaseBody, kind);
                self.cleanup_failures.push(fault);
                self.after_cleanup(n);
            }
            St::Compensating => {
                self.slots[n].st = St::ReleaseFailed;
                self.emit(n, TraceKind::CompensateFail(Self::label(&kind)));
                let fault = self.fault(n, Phase::Compensate, kind);
                self.cleanup_failures.push(fault);
                self.after_cleanup(n);
            }
            St::RetryRelease => {
                let (phase, ev) = if self.t.nodes[n].kind == Kind::Effect {
                    (
                        Phase::Compensate,
                        TraceKind::CompensateFail(Self::label(&kind)),
                    )
                } else {
                    (
                        Phase::ReleaseBody,
                        TraceKind::ReleaseFail(Self::label(&kind)),
                    )
                };
                self.emit(n, ev);
                let fault = self.fault(n, phase, kind);
                self.cleanup_failures.push(fault);
                self.resume_retry(n);
            }
            St::Abandoned => {}
            _ if self.t.nodes[n].kind == Kind::BlockingStep => {}
            _ => return Err("NodeErr for a node with no body in flight"),
        }
        Ok(())
    }

    /// The attempts a node may make in total.
    fn attempts_allowed(&self, n: usize, ambiguous: bool) -> u32 {
        let attrs = &self.t.nodes[n].attrs;
        let declared = attrs.retry.map(|r| r.max_attempts()).unwrap_or(1);
        if ambiguous && attrs.on_ambiguous == Some(Ambiguity::Retry) {
            declared.max(2)
        } else {
            declared
        }
    }

    /// An attempt ended in a fault: retry (releasing first if it held), or
    /// exhaust.
    fn fail_attempt(&mut self, n: usize, phase: Phase, kind: FaultKind) {
        let ambiguous = self.slots[n].st == St::Ambiguous;
        self.emit(n, TraceKind::Fail(phase, Self::label(&kind)));
        let fault = self.fault(n, phase, kind);
        self.slots[n].faults.push(fault);
        self.release_grants(n);
        let id = self.slots[n].timer.take();
        self.drop_timer(id);
        self.slots[n].cancelling = false;
        self.slots[n].timing_out = false;
        let scope = self.t.nodes[n].scope;
        let can_retry =
            self.slots[n].attempt < self.attempts_allowed(n, ambiguous) && self.admitting(scope);
        if !can_retry {
            self.node_failed(n);
        } else if self.slots[n].held && self.can_owe(n) {
            self.slots[n].st = St::RetryRelease;
            let key = self.t.nodes[n].key;
            if self.t.nodes[n].kind == Kind::Effect {
                self.emit(n, TraceKind::CompensateStart);
                self.fx.push(Effect::Compensate(key));
            } else {
                self.emit(n, TraceKind::ReleaseStart);
                self.fx.push(Effect::Release(key));
            }
        } else {
            self.slots[n].st = St::Backoff;
            self.schedule_backoff(n);
        }
    }

    /// The held attempt's release ended: on to the next attempt, unless the
    /// scope stopped admitting meanwhile.
    fn resume_retry(&mut self, n: usize) {
        self.slots[n].held = false;
        let scope = self.t.nodes[n].scope;
        if self.admitting(scope) {
            self.slots[n].st = St::Backoff;
            self.schedule_backoff(n);
        } else {
            self.slots[n].st = St::Interrupted;
            self.slots[n].attempt += 1;
            self.emit(n, TraceKind::Interrupted { held: false });
            let faults = std::mem::take(&mut self.slots[n].faults);
            self.faults.extend(faults);
            self.after_settle(n);
        }
    }

    fn schedule_backoff(&mut self, n: usize) {
        let failures = self.slots[n].attempt;
        let wait = backoff_wait(
            self.t.nodes[n].attrs.retry.and_then(|r| r.backoff_policy()),
            failures,
        );
        self.backoff(n, wait);
    }

    fn backoff(&mut self, n: usize, wait: Duration) {
        if wait.is_zero() {
            self.slots[n].st = St::Pending;
            self.after_settle(n);
        } else {
            self.slots[n].st = St::Backoff;
            self.slots[n].backoff_until = self.now + wait;
            let id = self.timer(Purpose::Backoff(n), self.now + wait);
            self.slots[n].timer = Some(id);
        }
    }

    pub(super) fn on_backoff_timer(&mut self, n: usize) {
        if self.slots[n].st == St::Backoff {
            self.slots[n].timer = None;
            self.slots[n].st = St::Pending;
            self.after_settle(n);
        }
    }

    /// Attempts exhausted: the node is `Failed`, its faults are the report's,
    /// and the scope's policy applies.
    fn node_failed(&mut self, n: usize) {
        self.slots[n].st = St::Failed;
        let faults = std::mem::take(&mut self.slots[n].faults);
        self.faults.extend(faults);
        let scope = self.t.nodes[n].scope;
        match self.t.scopes[scope].policy {
            Policy::FailFast => self.settle(scope, Cause::Fault(n)),
            Policy::Isolate => self.skip_dependents(n, n),
        }
        if let Some(c) = self.t.scopes[scope].component {
            self.component_faulted(c, n);
        }
        self.after_settle(n);
    }

    /// An inner node faulted: the component faults in its parent, once, and
    /// the parent's policy applies (contract § 11: inner fault propagation).
    fn component_faulted(&mut self, c: usize, inner: usize) {
        if !matches!(self.slots[c].st, St::Running | St::Ready) {
            return;
        }
        self.slots[c].st = St::Failed;
        self.emit(c, TraceKind::Fail(Phase::Prepare, FaultLabel::Error));
        let fault = self.fault(
            c,
            Phase::Prepare,
            FaultKind::Error(Box::new(InnerFault(self.t.nodes[inner].path.clone()))),
        );
        self.faults.push(fault);
        // The component has failed, so nothing else starts inside it (T5).
        // Under an inner `Isolate` the inner scope was still admitting, and
        // a sibling that became ready afterwards started a body after the run
        // had settled. Its inner graph is torn down by the component's own
        // release, which `open_component` opens when the gate allows.
        if let Some(i) = self.t.nodes[c].inner {
            if matches!(self.scopes[i].st, RunState::Admitting | RunState::Steady) {
                self.settle(i, Cause::Parent(Some(inner)));
            }
        }
        let parent = self.t.nodes[c].scope;
        match self.t.scopes[parent].policy {
            Policy::FailFast => self.settle(parent, Cause::Fault(c)),
            Policy::Isolate => self.skip_dependents(c, inner),
        }
        if let Some(grand) = self.t.scopes[parent].component {
            self.component_faulted(grand, inner);
        }
        self.after_settle(c);
    }

    // ------------------------------------------------------------ T5 results

    /// A `within` deadline passed while the body was in flight.
    pub(super) fn on_within_timer(&mut self, n: usize) {
        if self.slots[n].st != St::Running || self.slots[n].cancelling {
            return;
        }
        self.slots[n].timer = None;
        if self.t.nodes[n].kind == Kind::BlockingStep {
            // T7: a blocking body cannot be aborted. The deadline is a fault
            // now; the thread's later outcome is ignored.
            self.fail_attempt(n, Phase::Run, FaultKind::Timeout);
            return;
        }
        self.slots[n].cancelling = true;
        self.slots[n].timing_out = true;
        self.fx.push(Effect::Abort(self.t.nodes[n].key));
    }

    /// The body the engine cancelled has been joined (T5): `Interrupted`, or
    /// `Ambiguous` for an effect that never held, or — when the cancel was a
    /// `within` deadline — a `Timeout` fault under T4.
    pub(super) fn finish_interrupted(&mut self, n: usize) {
        let id = self.slots[n].timer.take();
        self.drop_timer(id);
        let held = self.slots[n].held;
        let effect = self.t.nodes[n].kind == Kind::Effect;
        if self.slots[n].timing_out {
            self.slots[n].timing_out = false;
            self.slots[n].cancelling = false;
            if effect && !held {
                self.emit(n, TraceKind::Ambiguous);
                let rec = self.record(n);
                self.ambiguous.push(rec);
                self.slots[n].st = St::Ambiguous;
                self.emit(n, TraceKind::Fail(Phase::Prepare, FaultLabel::Timeout));
                let fault = self.fault(n, Phase::Prepare, FaultKind::Timeout);
                self.slots[n].faults.push(fault);
                self.release_grants(n);
                let scope = self.t.nodes[n].scope;
                let retry = self.t.nodes[n].attrs.on_ambiguous == Some(Ambiguity::Retry)
                    && self.slots[n].attempt < self.attempts_allowed(n, true)
                    && self.admitting(scope);
                if retry {
                    self.backoff_after_ambiguity(n);
                } else {
                    self.ambiguous_failed(n);
                }
            } else {
                let phase = self.body_phase(n);
                self.fail_attempt(n, phase, FaultKind::Timeout);
            }
            return;
        }
        self.slots[n].cancelling = false;
        self.slots[n].signalled = false;
        self.release_grants(n);
        if effect && !held {
            self.slots[n].st = St::Ambiguous;
            self.emit(n, TraceKind::Ambiguous);
            let rec = self.record(n);
            self.ambiguous.push(rec);
        } else {
            self.slots[n].st = St::Interrupted;
            self.emit(n, TraceKind::Interrupted { held });
        }
        let faults = std::mem::take(&mut self.slots[n].faults);
        self.faults.extend(faults);
        self.after_settle(n);
    }

    /// An ambiguous, retryable effect: the next attempt after the backoff.
    fn backoff_after_ambiguity(&mut self, n: usize) {
        self.slots[n].st = St::Backoff;
        self.schedule_backoff(n);
    }

    /// An ambiguous effect that is not retried ends here; its timeout is a
    /// fault under the scope's policy, and it stays `Ambiguous`.
    fn ambiguous_failed(&mut self, n: usize) {
        let faults = std::mem::take(&mut self.slots[n].faults);
        self.faults.extend(faults);
        let scope = self.t.nodes[n].scope;
        match self.t.scopes[scope].policy {
            Policy::FailFast => self.settle(scope, Cause::Fault(n)),
            Policy::Isolate => self.skip_dependents(n, n),
        }
        if let Some(c) = self.t.scopes[scope].component {
            self.component_faulted(c, n);
        }
        self.after_settle(n);
    }

    pub(super) fn on_cancelled(&mut self, n: usize, held: bool) -> Result<(), &'static str> {
        match self.slots[n].st {
            St::Running => {
                self.slots[n].held |= held;
                self.finish_interrupted(n);
                Ok(())
            }
            St::Abandoned | St::Stopping | St::Releasing | St::Compensating => Ok(()),
            _ => Err("NodeCancelled for a node with no body in flight"),
        }
    }

    // ------------------------------------------------------------ serve

    pub(super) fn on_serve_ended(
        &mut self,
        n: usize,
        fault: Option<FaultKind>,
    ) -> Result<(), &'static str> {
        if self.t.nodes[n].kind != Kind::Service {
            return Err("ServeEnded for a node that is not a service");
        }
        match self.slots[n].st {
            St::Ready => {
                self.release_grants(n);
                let scope = self.t.nodes[n].scope;
                match fault {
                    None => {
                        self.slots[n].st = St::Finished;
                        self.emit(n, TraceKind::Stopped);
                        if self.t.nodes[n].attrs.terminal && self.admitting(scope) {
                            self.settle(scope, Cause::Terminal);
                        }
                        self.after_settle(n);
                    }
                    Some(kind) => {
                        self.emit(n, TraceKind::Fail(Phase::Serve, Self::label(&kind)));
                        let restart = self.t.nodes[n].attrs.restart;
                        let can_restart = restart.is_some()
                            && restart
                                .and_then(|r| r.limit())
                                .map_or(true, |l| self.slots[n].restarts < l)
                            && self.admitting(scope);
                        if can_restart {
                            // The episode that just died is a fault like any
                            // other: parked on the node, cleared if the
                            // restart reaches `Ready` and flushed to the
                            // report if it never does (INV-9). Dropping it
                            // here lost a serve failure whenever the restart
                            // was cut short.
                            let fault = self.fault(n, Phase::Serve, kind);
                            self.slots[n].faults.push(fault);
                            self.slots[n].restarts += 1;
                            let wait =
                                backoff_wait(restart.map(|r| r.backoff()), self.slots[n].restarts);
                            self.backoff(n, wait);
                        } else {
                            let fault = self.fault(n, Phase::Serve, kind);
                            self.slots[n].faults.push(fault);
                            self.node_failed(n);
                        }
                    }
                }
            }
            St::Stopping => {
                let id = self.slots[n].timer.take();
                self.drop_timer(id);
                match fault {
                    None => {
                        self.slots[n].st = St::Stopped;
                        self.emit(n, TraceKind::Stopped);
                    }
                    Some(kind) => {
                        self.slots[n].st = St::ReleaseFailed;
                        self.emit(n, TraceKind::Fail(Phase::Stop, Self::label(&kind)));
                        let fault = self.fault(n, Phase::Stop, kind);
                        self.cleanup_failures.push(fault);
                    }
                }
                self.after_cleanup(n);
            }
            St::Abandoned | St::Finished | St::Failed => {}
            _ => return Err("ServeEnded for a service that is not serving"),
        }
        Ok(())
    }
}
