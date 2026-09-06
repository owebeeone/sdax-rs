//! The three chores every terminal ramp owes, in one place.
//!
//! A node or a scope can stop for many reasons — the scope settles, a fault
//! exhausts the attempts, a component's release opens, the shutdown budget
//! expires — and each ramp used to spell the same rules out again. Forgetting
//! one is not a design mistake, it is a copy that was never made, which is why
//! the same three rules kept going missing on new ramps:
//!
//! * [`flush_faults`](Machine::flush_faults) — INV-9: what already faulted is
//!   the report's, whatever ends the node.
//! * [`end_component_attempt`](Machine::end_component_attempt) — INV-15: a
//!   component's `Start(Prepare)` gets a terminal observation before its
//!   `ReleaseStart`, never an orphan.
//! * [`settle_or_skip_inner`](Machine::settle_or_skip_inner) — INV-5: a
//!   component's inner scope is settled if it is live and skipped if it never
//!   started, so the import-release gates it holds shut can open.

use super::state::{Cause, Machine, St};
use super::RunState;
use crate::report::TraceKind;

impl Machine {
    /// INV-9: the faults of the attempts that already failed are the report's,
    /// whatever ends the node — a skip, an interrupt, an exhausted retry, an
    /// abandonment. The budget running out is not a reason for an observed
    /// fault to disappear.
    pub(super) fn flush_faults(&mut self, n: usize) {
        let faults = std::mem::take(&mut self.slots[n].faults);
        self.faults.extend(faults);
        // INV-11: an ambiguity travels with the faults of the attempt that
        // raised it. Reported eagerly, an `Ambiguity::Retry` whose retry
        // succeeded still ended unclean, so `Retry` could never yield a clean
        // run after one timeout — and the contract offers it as the policy for
        // an idempotent effect that can simply be done again.
        if let Some(rec) = self.slots[n].ambiguity.take() {
            self.ambiguous.push(rec);
        }
    }

    /// INV-15: end a component's own attempt with a terminal observation.
    ///
    /// A component has no body of its own; its attempt is the inner graph
    /// coming up. Every route into the component's release — the parent
    /// settling, the inner scope settling for a reason of its own, the joins
    /// completing under a settling parent — has to close that attempt first,
    /// or the trace goes `Start(Prepare)` → `ReleaseStart` with nothing
    /// between it and an orphan (INV-15). It never became `Ready`, and the
    /// engine stopped admitting under it, so `Interrupted` is the honest
    /// state, and it is never a fault (INV-10).
    ///
    /// `held` is `started`, which is necessarily `true` here: `St::Running` is
    /// assigned in exactly one place in the engine (`admit::start`), on the
    /// line after `started = true`, and `started` is never cleared. There is
    /// an inner graph left to tear down, which is what opens the release.
    pub(super) fn end_component_attempt(&mut self, c: usize) {
        if !matches!(self.slots[c].st, St::Running | St::Publishing) {
            return;
        }
        // `St::Running ⟹ started`: `St::Running` is assigned in exactly one
        // place (`admit::start`), on the line after `started = true`, and
        // `started` is never cleared. Asserted here so the next writer of a
        // path into `Running` — a restart, a template instance — trips on it
        // rather than shipping an `Interrupted{held: false}` for a component
        // whose inner graph is still up.
        debug_assert!(self.slots[c].started, "St::Running implies started");
        self.slots[c].st = St::Interrupted;
        let held = self.slots[c].started;
        self.emit(c, TraceKind::Interrupted { held });
        self.flush_faults(c);
    }

    /// A node's inner scope stops admitting with the node.
    ///
    /// `Planned` means the component never started, so the inner nodes are
    /// still `Pending`; left alone they hold the release gate of everything
    /// they import shut (INV-5) and the run cannot finish, so they are
    /// skipped, all the way down. `Admitting | Steady` means the inner graph
    /// is live and settles with its component — nothing new starts inside it
    /// either (T5), and its release cannot open while its scope is still
    /// `Steady`, because `in_flight` would count the component and the run
    /// would wait for a component that is waiting for the run. A scope that
    /// is already `Settling | Cleanup | Ended` is left alone.
    ///
    /// A **template** is the same rule over its live instances: each is a
    /// scope of its own, and each stops admitting with the node.
    ///
    /// A node with neither an inner scope nor instances is a no-op, so every
    /// ramp can call this without asking what it is holding.
    pub(super) fn settle_or_skip_inner(&mut self, n: usize, because: Option<usize>) {
        if self.t.nodes[n].kind == crate::plan::Kind::Template {
            self.settle_instances(n, because);
            return;
        }
        let Some(inner) = self.t.nodes[n].inner else {
            return;
        };
        match self.scopes[inner].st {
            RunState::Planned => self.skip_scope(inner, because),
            RunState::Admitting | RunState::Steady => self.settle(inner, Cause::Parent(because)),
            RunState::Settling | RunState::Cleanup | RunState::Ended => {}
        }
    }
}
