//! T6–T8 — the release graph, the shutdown budget, and the end.
//!
//! A node's obligation starts iff every node that needs or imports it has
//! finished its cleanup (T6); unrelated obligations overlap (INV-6); a cleanup
//! body is never dropped by a request, only abandoned by the budget (INV-7,
//! T7); the run ends when every obligation is discharged or abandoned (T8).
//! A component is one unit: its inner scope settles and cleans up when the
//! parent's graph reaches it, or as soon as it settled by itself and nothing
//! outside still depends on it.

use super::state::{Machine, Purpose, St};
use super::{Effect, RunState};
use crate::plan::Kind;
use crate::report::{Report, TraceKind};

impl Machine {
    /// In-flight bodies and publication acknowledgements that must settle
    /// before cleanup may begin (P-17: no release before the joins). Publication
    /// remains outstanding after lifecycle cancellation. A component counts while
    /// its inner scope still has in-flight bodies, and a template counts for
    /// every live instance the same way.
    pub(super) fn in_flight(&self, scope: usize) -> usize {
        let own = self.t.scopes[scope]
            .nodes
            .iter()
            .filter(|&&n| {
                self.slots[n].publication_pending
                    || match self.t.nodes[n].inner {
                        Some(inner) => {
                            (self.slots[n].st == St::Running
                                && matches!(
                                    self.scopes[inner].st,
                                    RunState::Admitting | RunState::Steady
                                ))
                                || self.in_flight(inner) > 0
                        }
                        None => self.slots[n].st == St::Running,
                    }
            })
            .count();
        let instances: usize = self.t.scopes[scope]
            .nodes
            .iter()
            .filter(|&&n| self.t.nodes[n].kind == Kind::Template)
            .map(|&n| self.instances_in_flight(n))
            .sum();
        own + instances
    }

    /// An obligation exists and has not started.
    pub(super) fn owes(&self, n: usize) -> bool {
        let s = &self.slots[n];
        let kind = self.t.nodes[n].kind;
        if kind == Kind::Template {
            // A template's obligation is stopping every live instance
            // (contract § 1). It is owed while the node is live — so a
            // template that reached `Live` always closes its record, and a
            // reader can tell "done with its instances" from "still holding
            // them" — and also while any instance is live whatever state the
            // node itself reached, because a template an isolated fault had
            // already `Skipped` still holds the instances a body spawned
            // before the fault (Monte Carlo seed 10566988825915931949: the run
            // ended with one still running, against INV-16).
            return match s.st {
                // The obligation has started, ended, or been abandoned by the
                // budget. Leaving `Abandoned` out let `advance_cleanup` open
                // the obligation a second time, so a template got both an
                // `Abandoned` and a `Stopped` (`monte_carlo_big` seed
                // 12339652235566683353).
                St::StoppingInstances | St::Stopped | St::Abandoned => false,
                St::Live => true,
                _ => self.ever_spawned(n),
            };
        }
        let inner_live = self.t.nodes[n]
            .inner
            .is_some_and(|i| self.scopes[i].st != RunState::Ended && s.started);
        match s.st {
            St::Ready => match kind {
                Kind::Resource | Kind::Effect => s.held && self.can_owe(n),
                Kind::Service => true,
                Kind::Component => inner_live,
                _ => false,
            },
            St::Failed | St::Interrupted => match kind {
                Kind::Resource | Kind::Effect => s.held && self.can_owe(n),
                Kind::Component => inner_live,
                _ => false,
            },
            St::Ambiguous => self.compensates_ambiguity(n),
            _ => false,
        }
    }

    /// Whether `d`'s cleanup has not ended (INV-5 waits for the end).
    pub(super) fn blocks_release(&self, d: usize) -> bool {
        self.slots[d].publication_pending || matches!(
            self.slots[d].st,
            St::Pending
                | St::Waiting
                | St::Running
                | St::Backoff
                | St::RetryRelease
                | St::Releasing
                | St::Compensating
                | St::Stopping
                | St::StoppingInstances
        ) || self.owes(d)
            // A template blocks while any instance of it is still live, even
            // once the budget has abandoned the template itself: an instance
            // is a scope of its own and its release graph is still running.
            // Without this the root ended while an instance was mid-cleanup
            // (`monte_carlo_big` seed 1520897640266743839); the scope's zero
            // timer (T7b) is what then abandons whatever is left, so the run
            // still terminates.
            || (self.t.nodes[d].kind == Kind::Template && !self.live_instances(d).is_empty())
    }

    pub(super) fn gate_open(&self, n: usize) -> bool {
        !self.slots[n].publication_pending
            && self.t.nodes[n]
                .dependents
                .iter()
                .all(|&d| !self.blocks_release(d))
    }

    /// `Settling → Cleanup` once nothing is in flight; a component's inner
    /// scope also needs its own gate in the parent (T6).
    pub(super) fn try_cleanup(&mut self, scope: usize) {
        match self.scopes[scope].st {
            RunState::Settling => {
                for c in self.t.scopes[scope].nodes.clone() {
                    if let Some(inner) = self.t.nodes[c].inner {
                        if self.slots[c].st == St::Running
                            && self.scopes[inner].st == RunState::Settling
                            && self.in_flight(inner) == 0
                        {
                            self.end_component_attempt(c);
                        }
                    }
                }
                if self.in_flight(scope) > 0 {
                    return;
                }
                // An instance's release graph is gated by nothing in the
                // parent: INV-16 forbids a parent node to name an instance
                // node, so an instance is only ever a *dependent*. It cleans
                // up as soon as its own bodies are joined, which is what lets
                // the keys it imports be released afterwards.
                if self.t.scopes[scope].instance.is_some() {
                    self.scopes[scope].st = RunState::Cleanup;
                    self.arm_scope_budget(scope);
                    self.advance_cleanup(scope);
                    self.check_end(scope);
                    return;
                }
                match self.t.scopes[scope].component {
                    None => {
                        self.scopes[scope].st = RunState::Cleanup;
                        self.advance_cleanup(scope);
                        self.check_end(scope);
                    }
                    Some(c) => {
                        if self.slots[c].st == St::Releasing || self.gate_open(c) {
                            self.open_component(c);
                        } else {
                            let parent = self.t.nodes[c].scope;
                            self.try_cleanup(parent);
                        }
                    }
                }
            }
            RunState::Cleanup => {
                self.advance_cleanup(scope);
                self.check_end(scope);
            }
            _ => {}
        }
    }

    /// Start every obligation whose gate is open, to a fixpoint.
    pub(super) fn advance_cleanup(&mut self, scope: usize) {
        if self.scopes[scope].st != RunState::Cleanup {
            return;
        }
        loop {
            let mut progress = false;
            for n in self.t.scopes[scope].nodes.clone() {
                if self.owes(n) && self.gate_open(n) {
                    self.start_obligation(n);
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        if self.scopes[scope].spent {
            self.arm_zero(scope);
        }
    }

    fn start_obligation(&mut self, n: usize) {
        let key = self.t.nodes[n].key;
        match self.t.nodes[n].kind {
            Kind::Resource => {
                self.slots[n].st = St::Releasing;
                self.emit(n, TraceKind::ReleaseStart);
                self.fx.push(Effect::Release(key));
            }
            Kind::Effect => {
                let recovery = self.slots[n].st == St::Ambiguous;
                self.slots[n].recovering = recovery;
                self.slots[n].st = St::Compensating;
                if recovery {
                    self.emit(n, TraceKind::RecoveryStart);
                    self.fx.push(Effect::Recover(key));
                } else {
                    self.emit(n, TraceKind::CompensateStart);
                    self.fx.push(Effect::Compensate(key));
                }
            }
            Kind::Service => {
                self.slots[n].st = St::Stopping;
                self.emit(n, TraceKind::StopRequested);
                self.fx.push(Effect::StopService(key));
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
                    let id = self.timer(Purpose::StopDeadline(n), at);
                    self.slots[n].timer = Some(id);
                }
            }
            Kind::Component => self.open_component(n),
            Kind::Template => self.start_template_stop(n),
            _ => {}
        }
    }

    /// The parent's graph reached a component, or the component settled by
    /// itself with nothing outside depending on it: settle the inner scope
    /// and, once its bodies are joined, run its release graph.
    /// T7: the scope's shutdown budget, `min`-ed with the parent's so it never
    /// outlives it (`V-BUDGET-ORDER`). Armed once: for the root at its settle,
    /// for a nested scope when its release graph opens.
    pub(super) fn arm_scope_budget(&mut self, scope: usize) {
        if self.scopes[scope].budget_timer.is_some() || self.scopes[scope].spent {
            return;
        }
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
    }

    fn open_component(&mut self, c: usize) {
        if self.slots[c].publication_pending {
            return;
        }
        let inner = self.t.nodes[c].inner.expect("component");
        if self.slots[c].st != St::Releasing {
            // The component's own attempt ends before its release opens. An
            // inner scope can settle for a reason of its own — a `terminal`
            // service inside it finishing, say — with the parent still
            // admitting, and then nothing has cut the component's attempt
            // short the way `settle::interrupt` does when the parent settles.
            // Left alone the component went `Start(Prepare)` → `ReleaseStart`
            // with no terminal observation, an orphan (INV-15). It never
            // became `Ready`: the inner run never reached steady state.
            self.end_component_attempt(c);
            self.slots[c].st = St::Releasing;
            self.emit(c, TraceKind::ReleaseStart);
        }
        self.settle_or_skip_inner(c, None);
        // T7: the inner scope's own budget starts here — the first instant its
        // releases are allowed to run — and is capped by the parent's.
        self.arm_scope_budget(inner);
        if self.scopes[inner].st == RunState::Settling && self.in_flight(inner) == 0 {
            self.scopes[inner].st = RunState::Cleanup;
            self.advance_cleanup(inner);
            self.check_end(inner);
        }
    }

    /// The service's stop deadline passed: abort and abandon (T7).
    pub(super) fn on_stop_deadline(&mut self, n: usize) {
        if self.slots[n].st == St::Stopping {
            self.slots[n].timer = None;
            self.abandon(n);
            self.after_cleanup(n);
        }
    }

    fn abandon(&mut self, n: usize) {
        if self.slots[n].st == St::Compensating && self.slots[n].recovering {
            self.emit(n, TraceKind::RecoveryFail(crate::FaultLabel::Timeout));
            let fault = self.fault(n, crate::Phase::Recover, crate::FaultKind::Timeout);
            self.cleanup_failures.push(fault);
        }
        // A blocking body cannot be aborted (T7); a template has no task at
        // all, so an `Abort` for one would name a node the driver never
        // spawned.
        if !matches!(self.t.nodes[n].kind, Kind::BlockingStep | Kind::Template) {
            self.fx.push(Effect::Abort(self.t.nodes[n].key));
        }
        let id = self.slots[n].timer.take();
        self.drop_timer(id);
        self.release_grants(n);
        self.slots[n].cancelling = false;
        self.slots[n].timed_out = false;
        self.slots[n].st = St::Abandoned;
        self.emit(n, TraceKind::Abandoned);
        // The budget running out is not a reason for an observed fault to
        // disappear (INV-9).
        self.flush_faults(n);
        let rec = self.record(n);
        self.incomplete.push(rec);
    }

    /// Abandon everything still running in a scope, and in the scopes inside
    /// it. `bodies` says whether in-flight prepare bodies count too (the
    /// budget: yes; the zero-length re-check after it: only cleanups).
    fn abandon_all(&mut self, scope: usize, bodies: bool) {
        // Two passes. Every ordinary node's attempt ends first, in declaration
        // order; only then are the components' inner release graphs opened, in
        // reverse declaration order. A component's `ReleaseStart` is a cleanup
        // *start*, and INV-5 does not allow one while a dependent of the
        // component has not finished — which is what a single declaration-order
        // pass did whenever a node outside a component still had a body in
        // flight when the budget ran out. A node can only `needs` a key that
        // already exists, so reverse declaration order is a cleanup order.
        for n in self.t.scopes[scope].nodes.clone() {
            if self.t.nodes[n].inner.is_some() {
                continue;
            }
            if self.t.nodes[n].kind == Kind::Template {
                continue;
            }
            match self.slots[n].st {
                St::Running if bodies => self.abandon(n),
                St::Releasing | St::Compensating | St::Stopping | St::RetryRelease => {
                    self.abandon(n)
                }
                _ => {}
            }
        }
        // A template's instances are abandoned with it, in reverse declaration
        // order for the same reason: an instance is a cleanup unit, and its
        // `Stopped` is a cleanup end that INV-5 orders after its dependents'.
        for n in self.t.scopes[scope].nodes.clone().into_iter().rev() {
            if self.t.nodes[n].kind == Kind::Template {
                if !self.live_instances(n).is_empty() || self.slots[n].st == St::StoppingInstances {
                    self.settle_instances(n, None);
                    for x in self.live_instances(n) {
                        let inst = self.instances[x].scope;
                        self.abandon_all(inst, true);
                        self.scopes[inst].spent = true;
                        self.arm_zero(inst);
                    }
                    if self.slots[n].st == St::StoppingInstances {
                        self.abandon(n);
                    }
                }
                continue;
            }
            let Some(inner) = self.t.nodes[n].inner else {
                continue;
            };
            match self.slots[n].st {
                St::Running if bodies => self.abandon_inner(n),
                St::Releasing | St::Compensating | St::Stopping | St::RetryRelease => {
                    self.abandon_inner(n)
                }
                St::Ready | St::Failed | St::Interrupted
                    if self.scopes[inner].st != RunState::Ended && self.slots[n].started =>
                {
                    self.abandon_inner(n)
                }
                _ => {}
            }
        }
    }

    fn abandon_inner(&mut self, n: usize) {
        let inner = self.t.nodes[n].inner.expect("component");
        self.settle_or_skip_inner(n, None);
        self.abandon_all(inner, true);
        self.scopes[inner].spent = true;
        // The scope is left `Settling`, with no `ReleaseStart` for the
        // component. An inner scope's release *is* its component's release,
        // and INV-5 does not allow one to start while a dependent of the
        // component has not finished — a *dependent component* abandoned in
        // this same instant only reaches `ReleaseOk` in the sweep that
        // follows, which no ordering of the passes above can bring forward.
        // So the transition, and the `ReleaseStart` that goes with it, belong
        // to `open_component`, which the gate guards; `sweep` asks
        // `try_cleanup` to open it as soon as the gate allows. This is the
        // same shape as `on_budget_timer` for the root.
    }

    /// The shutdown budget expired (T7).
    pub(super) fn on_budget_timer(&mut self, scope: usize) {
        self.scopes[scope].budget_timer = None;
        if self.scopes[scope].st == RunState::Ended {
            return;
        }
        self.abandon_all(scope, true);
        self.scopes[scope].spent = true;
        // Only the root may go straight to cleanup here. An inner scope's
        // release *is* its component's release, and that one opens through
        // `open_component` when the gate in the parent allows (INV-5) — which
        // is also where the component's own attempt ends and its
        // `ReleaseStart` is emitted. Forcing the transition here ended the
        // inner scope with neither, so the component's `ReleaseOk` had no
        // start and its attempt was an orphan (INV-15). `sweep` below asks
        // `try_cleanup` to open it as soon as the gate allows.
        if self.scopes[scope].st == RunState::Settling
            && self.t.scopes[scope].component.is_none()
            && self.in_flight(scope) == 0
        {
            self.scopes[scope].st = RunState::Cleanup;
        }
        self.sweep();
    }

    pub(super) fn arm_zero(&mut self, scope: usize) {
        let running = self.t.scopes[scope].nodes.iter().any(|&n| {
            matches!(
                self.slots[n].st,
                St::Releasing | St::Compensating | St::Stopping | St::RetryRelease
            )
        });
        if running && self.scopes[scope].zero_timer.is_none() {
            let id = self.timer(Purpose::Zero(scope), self.now);
            self.scopes[scope].zero_timer = Some(id);
        }
    }

    /// The zero-length re-check after the budget: whatever is still running
    /// now is abandoned; obligations that completed in the same instant were
    /// delivered first and stand.
    pub(super) fn on_zero_timer(&mut self, scope: usize) {
        self.scopes[scope].zero_timer = None;
        if self.scopes[scope].st == RunState::Ended {
            return;
        }
        self.abandon_all(scope, false);
        self.sweep();
    }

    /// An obligation ended: gates elsewhere may have opened.
    ///
    /// Not only cleanup gates. An **abandonment** ends an obligation by
    /// [`release_grants`](Self::release_grants) and nothing else — no
    /// `after_settle`, because the node never settles — so a lock or a pool
    /// slot the abandoned node held becomes free here and nowhere else. A
    /// waiter for it lives in whatever scope named the resource, which for an
    /// imported one is not the abandoned node's scope; and a scope whose own
    /// budget abandoned something can have a *parent* that is still admitting.
    /// Without this, an inner service abandoned at its `stop_within` left a
    /// waiter outside the component `Waiting` for a lock nobody held, for the
    /// rest of the run and with an empty `on` — the same shape `admit_all`
    /// was written for, reached down a different path (MC seed
    /// `9106096978137470251`).
    ///
    /// `admit_all` starts nothing in a scope that is not `Admitting` or
    /// `Steady`, so the budget's own `abandon_all` sweep is unaffected.
    pub(super) fn after_cleanup(&mut self, _n: usize) {
        self.admit_all();
        self.sweep();
    }

    /// Advance every scope that can advance, inner scopes ending before the
    /// components that contain them.
    pub(super) fn sweep(&mut self) {
        for s in 0..self.scopes.len() {
            match self.scopes[s].st {
                RunState::Settling => self.try_cleanup(s),
                RunState::Cleanup => self.advance_cleanup(s),
                _ => {}
            }
        }
        for s in (0..self.scopes.len()).rev() {
            self.check_end(s);
        }
    }

    /// T8 for one scope.
    pub(super) fn check_end(&mut self, scope: usize) {
        if self.scopes[scope].st != RunState::Cleanup {
            return;
        }
        let done = self.t.scopes[scope]
            .nodes
            .iter()
            .all(|&n| !self.blocks_release(n));
        if !done {
            return;
        }
        self.scopes[scope].st = RunState::Ended;
        let id = self.scopes[scope].budget_timer.take();
        self.drop_timer(id);
        let id = self.scopes[scope].zero_timer.take();
        self.drop_timer(id);
        if self.t.scopes[scope].instance.is_some() {
            self.end_instance(scope);
            return;
        }
        match self.t.scopes[scope].component {
            None => self.end_root(),
            Some(c) => {
                // The component is `Releasing`: every route into an inner
                // `Cleanup` goes through `open_component`, which ends the
                // component's own attempt and emits its `ReleaseStart` first.
                // A `Ready → Finished` arm here was unreachable, and the
                // checker accepted a shape the machine never emits.
                debug_assert_eq!(self.slots[c].st, St::Releasing);
                self.slots[c].st = St::Released;
                self.emit(c, TraceKind::ReleaseOk);
                self.after_cleanup(c);
            }
        }
    }

    /// `Ended(Report)` for the root (T8), the report already in F4 order.
    pub(super) fn end_root(&mut self) {
        if self.report.is_some() {
            return;
        }
        self.scopes[0].st = RunState::Ended;
        let outcome = self.outcome(0);
        self.emit_run(TraceKind::End(outcome));
        self.fx.push(Effect::End(outcome));
        let mut report: Report<()> = Report::empty(outcome);
        report.faults = std::mem::take(&mut self.faults);
        report.cleanup_failures = std::mem::take(&mut self.cleanup_failures);
        report.incomplete = std::mem::take(&mut self.incomplete);
        report.ambiguous = std::mem::take(&mut self.ambiguous);
        report.sort();
        self.report = Some(report);
    }
}
