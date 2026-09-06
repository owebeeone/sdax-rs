//! Everything the driver says to the outside world: the report at `End`, the
//! drop of a live `Running`, a refused event, and what it says on its way out
//! when the runtime is torn down under it — each through one boundary around
//! the [`Observer`](sdax::host::Observer) callbacks.
//!
//! Contract § 10 forbids an observer to block or to panic, so a panic here is
//! the observer's defect. What made it *this* crate's defect was the blast
//! radius: the callbacks run on the driver task, and an unguarded panic
//! unwound it — every live body detached (dropping a `JoinHandle` does not
//! abort), no report, the readiness latch never set, and the awaiter handed an
//! empty `Cancelled`. That is INV-15's failure mode, reached in silence. The
//! panic is caught where it happens, recorded as
//! [`TraceKind::ObserverPanicked`], and the run goes on.

use super::Driver;
use sdax::host::engine::Rejected;
use sdax::host::{Runtime, TaskHandle};
use sdax::{Outcome, Report, TraceEvent, TraceKind};
use std::panic::{catch_unwind, AssertUnwindSafe};

impl<R: Runtime> Driver<R> {
    /// Hand one event to the observer, and survive it if it panics.
    pub(super) fn notify(&mut self, ev: &TraceEvent) {
        let panicked = catch_unwind(AssertUnwindSafe(|| self.rt.observer().event(ev))).is_err();
        if panicked {
            self.note_observer_panic();
        }
    }

    /// Hand the report over, and survive it if it panics.
    ///
    /// The copy the observer saw was made before the note existed, so the note
    /// is added afterwards for the copy the awaiter gets.
    pub(super) fn tell_report(&mut self, report: &mut Report<()>) {
        let panicked =
            catch_unwind(AssertUnwindSafe(|| self.rt.observer().report(report))).is_err();
        if panicked {
            self.note_observer_panic();
            report.trace = Some(self.trace.clone());
        }
    }

    /// `Running` was dropped while the run was live (C-14).
    pub(super) fn emit_dropped(&mut self) {
        let ev = TraceEvent::at(self.machine.now(), TraceKind::DroppedWhileRunning);
        self.notify(&ev);
        self.trace.events.push(ev);
    }

    /// D1: the machine refused an event this driver fed it, which is a driver
    /// bug. It was recorded for a harness and shouted at stderr for nobody;
    /// neither reaches a production consumer, so it is a trace event too and
    /// travels with the report.
    pub(super) fn reject(&mut self, r: Rejected) {
        let what = format!("{} — {}", r.reason, r.event);
        self.ctl.reject(what.clone());
        let ev = TraceEvent::at(self.machine.now(), TraceKind::Rejected(what));
        self.notify(&ev);
        self.trace.events.push(ev);
    }

    /// The run is over: no timer of ours may outlive it (INV-15), the report
    /// reaches the observer, and whoever is awaiting gets it.
    pub(super) fn finish(&mut self) {
        for (_, t) in self.timers.drain() {
            t.abort();
        }
        let outcome = self.ended.unwrap_or(Outcome::Cancelled);
        let mut report: Report<()> = self.machine.take_report().unwrap_or_else(|| {
            // No `End`: the loop left with nothing to deliver. Saying so is
            // better than inventing a clean report.
            self.ctl
                .reject("the run stopped short of End with nothing left to deliver".to_string());
            Report::empty(outcome)
        });
        report.trace = Some(self.trace.clone());
        // The latch first: `ready()` must not wait on an observer, and the
        // report reaches its awaiter whatever the observer does with it.
        self.ctl.ended(report.outcome);
        self.tell_report(&mut report);
        let out = self.src.export();
        if let Some(done) = self.done.take() {
            let _ = done.send((report, out));
        }
    }

    /// Record the panic, keeping `End` the last event of the trace (T8).
    ///
    /// A note is written immediately before the event whose delivery panicked,
    /// which for every call but [`tell_report`](Self::tell_report) is an event
    /// not yet pushed — so it simply goes on the end.
    fn note_observer_panic(&mut self) {
        let ev = TraceEvent::at(self.machine.now(), TraceKind::ObserverPanicked);
        let evs = &mut self.trace.events;
        match evs.last() {
            Some(e) if matches!(e.kind, TraceKind::End(_)) => {
                let last = evs.len() - 1;
                evs.insert(last, ev);
            }
            _ => evs.push(ev),
        }
    }
}

impl<R: Runtime> Drop for Driver<R> {
    /// The driver future was dropped before it reached `End`: the tokio
    /// runtime was torn down under a live run or a pending drainer.
    ///
    /// Nothing will ever speak for this run again — the bodies are gone with
    /// the runtime, no release will run and no `End` will be emitted — so the
    /// loss is announced here rather than vanishing. `TokioRuntime`'s own
    /// `Drop` covers only the order in which the adapter goes first; this
    /// covers both, because it is the driver's own future that is dropped
    /// either way.
    ///
    /// Nothing here touches a tokio API: `Machine::now()` is the last clock
    /// reading the machine was given, not a fresh one, so this is safe on
    /// whatever thread the runtime's shutdown runs on.
    fn drop(&mut self) {
        // `finish` takes `done`, so a run that reached `End` is already
        // accounted for and this is a no-op.
        let Some(done) = self.done.take() else { return };
        let ev = TraceEvent::at(self.machine.now(), TraceKind::RuntimeDroppedWithLiveRuns);
        self.notify(&ev);
        self.trace.events.push(ev);
        let mut report: Report<()> = Report::empty(Outcome::Cancelled);
        report.trace = Some(self.trace.clone());
        // Latch first: a `ready()` waiter must not depend on an observer.
        self.ctl.ended(Outcome::Cancelled);
        self.tell_report(&mut report);
        let _ = done.send((report, None));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::running::Control;
    use crate::scope::RunScope;
    use crate::TokioRuntime;
    use sdax::host::engine::{Effect, Machine, Rejected};
    use sdax::host::Observer;
    use sdax::{Mode, Plan, Policy, Shutdown, TraceEvent};
    use std::sync::{Arc, Mutex};
    use std::time::Duration;

    /// Everything the observer was told.
    #[derive(Default)]
    struct Heard(Mutex<Vec<TraceKind>>);

    impl Observer for Heard {
        fn event(&self, e: &TraceEvent) {
            self.0.lock().expect("heard").push(e.kind.clone());
        }
    }

    fn a_plan() -> Plan {
        let mut p = Plan::builder("Reject");
        p.step("S").run(|_cx, ()| async move { Ok(()) });
        p.build(
            Policy::FailFast,
            Shutdown::within(Duration::from_secs(1)),
            Mode::Finite,
        )
        .expect("valid")
    }

    /// `S-12`: a rejection reaches the observer and the trace, not only stderr
    /// and a harness's `RunRecord`.
    ///
    /// D1 calls a refused event "a driver bug: log it loudly". Loudly used to
    /// mean one `eprintln!` from a library, which a production consumer never
    /// sees; the run's own record of itself said nothing at all. There is no
    /// plan and no `BodySource` that can make the machine refuse an event this
    /// driver feeds it — that is what obligations 1 to 5 are for — so the
    /// rejection is handed over directly here.
    #[test]
    fn s12_a_rejection_reaches_the_observer_and_the_trace() {
        let plan = a_plan();
        let tokio_rt = tokio::runtime::Builder::new_current_thread()
            .enable_time()
            .build()
            .expect("runtime");
        let heard = Arc::new(Heard::default());
        let rt = Arc::new(
            TokioRuntime::current_thread_no_background_drain(tokio_rt.handle().clone())
                .with_observer(heard.clone()),
        );
        let (tx, rx) = tokio::sync::mpsc::unbounded_channel();
        let (done, _rx) = tokio::sync::oneshot::channel();
        let mut driver = Driver::new(
            rt,
            Machine::new(&plan).expect("the machine accepts the plan"),
            sdax::host::bodies_of::<(), ()>(&plan),
            tx.clone(),
            rx,
            Arc::new(Control::detached(tx.clone())),
            None,
            done,
            RunScope::new(tx),
        );
        driver.perform(Effect::Reject(Rejected {
            event: "Started(node)".to_string(),
            reason: "Started for a node with no body in flight",
        }));
        let want = TraceKind::Rejected(
            "Started for a node with no body in flight — Started(node)".to_string(),
        );
        assert!(
            driver.trace.events.iter().any(|e| e.kind == want),
            "the report's trace carries it: {:?}",
            driver.trace.events
        );
        assert!(
            heard.0.lock().expect("heard").contains(&want),
            "and so does the observer: {:?}",
            heard.0.lock().expect("heard")
        );
    }
}
