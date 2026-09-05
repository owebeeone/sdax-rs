//! Suite (a): what the compiler rejects, witnessed.
//!
//! This module has no items. Its documentation *is* the witness suite: every
//! block below is a `compile_fail` doctest, so `cargo test` fails the moment
//! one of these programs starts compiling. Each block also carries the error
//! code it is expected to produce; `scripts/compile-fail.sh` extracts the
//! blocks and asserts those codes with `rustc`, which a doctest cannot do.
//!
//! Ids are the rows of `sdax-v1/B/CanonicalTests.md` § 2.
//!
//! ## W-01 — a dependency named before it exists (`E-R2`, and why a cycle is
//! unwritable)
//!
//! A key exists only after its node's terminal method ran, so "A needs B and B
//! needs A" has no spelling at all.
//!
//! ```compile_fail
//! // expect: E0425
//! use sdax::*;
//! let mut p = Plan::builder("I-35");
//! let a = p.step("A").needs(b).run(|_cx, _b: std::sync::Arc<u8>| async { Ok(()) });
//! let b = p.step("B").run(|_cx, ()| async { Ok(1u8) });
//! let _ = (a, b);
//! ```
//!
//! ## W-02 — the effect performed outside the wrapper (`X1`)
//!
//! The body binds the socket itself and returns the bare value. `acquire`
//! requires `Held<T>`, which only `cx.hold(..)` mints.
//!
//! ```compile_fail
//! // expect: E0271
//! use sdax::*;
//! struct Port;
//! async fn bind() -> Port { Port }
//! let mut p = Plan::builder("I-09");
//! let _port = p
//!     .resource("Port")
//!     .acquire(|_cx, ()| async move { Ok(bind().await) })
//!     .release(|_cx, _v| async move { Ok(()) });
//! ```
//!
//! ## W-03 — a forged registration (`X1`)
//!
//! `Held` has no public constructor and no public fields.
//!
//! ```compile_fail
//! // expect: E0451
//! use sdax::*;
//! use std::sync::Arc;
//! struct Port;
//! fn forge(arc: Arc<Port>, node: RawKey) -> Held<Port> {
//!     Held { arc, node }
//! }
//! ```
//!
//! ## W-04 — a seam call in the wrong phase (`U16`)
//!
//! `hold` exists only on `Cx<Acquire>`; a step body receives `Cx<Run>`.
//!
//! ```compile_fail
//! // expect: E0599
//! use sdax::*;
//! let mut p = Plan::builder("I-20");
//! let _s = p.step("P1").run(|cx, ()| async move {
//!     let _h = cx.hold(async { Ok::<u8, std::io::Error>(5) }).await;
//!     Ok(())
//! });
//! ```
//!
//! ## W-05 — a body whose parameters disagree with its `needs` (`U2`)
//!
//! ```compile_fail
//! // expect: E0631
//! use sdax::*;
//! use std::sync::Arc;
//! struct PeerStore;
//! struct RoutingTable;
//! let mut p = Plan::builder("I-01");
//! let peers = p.step("PeerStore").run(|_cx, ()| async { Ok(PeerStore) });
//! let _reg = p
//!     .step("Registration")
//!     .needs(peers)
//!     .run(|_cx, _r: Arc<RoutingTable>| async { Ok(()) });
//! ```
//!
//! ## W-06 — a resource with no release (`U3`)
//!
//! `.acquire(..)` yields `NeedsRelease<T>`, which is not a `Key<T>`, so the
//! node is never added and the mistake is a type error at the binding.
//!
//! ```compile_fail
//! // expect: E0308
//! use sdax::*;
//! struct Port;
//! let mut p = Plan::builder("I-01");
//! let _port: Key<Port> =
//!     p.resource("Port").acquire(|cx, ()| async move { Ok(cx.hold_value(Port)) });
//! ```
//!
//! ## W-07 — a parent naming a key that exists only inside a child plan
//! (`E-R3`)
//!
//! The child's keys are lexically scoped to the function that builds it.
//!
//! ```compile_fail
//! // expect: E0425
//! use sdax::*;
//! use std::sync::Arc;
//! struct Session;
//! fn conn_plan() -> Result<Plan, Invalid> {
//!     let mut t = Plan::builder("Conn");
//!     let session = t
//!         .resource("Session")
//!         .acquire(|cx, ()| async move { Ok(cx.hold_value(Session)) })
//!         .release(|_cx, _s| async move { Ok(()) });
//!     let _ = session;
//!     t.build(Policy::FailFast, Shutdown::within(std::time::Duration::from_secs(1)), Mode::Finite)
//! }
//! let mut p = Plan::builder("Process");
//! let _ = p
//!     .service("Reporter")
//!     .needs(session)
//!     .start(|_cx, _s: Arc<Session>| async move { Ok(Serving::new((), async { Ok(()) })) });
//! ```
//!
//! ## W-09 — an effect with no ambiguity policy (`X4`)
//!
//! `perform` does not exist on `Node<_, _, Effect<NoAmbiguity>>`.
//!
//! ```compile_fail
//! // expect: E0599
//! use sdax::*;
//! struct Receipt;
//! let mut p = Plan::builder("I-15");
//! let _e = p
//!     .effect("Registration")
//!     .perform(|cx, ()| async move { Ok(cx.hold_value(Receipt)) })
//!     .compensate(|_cx, _r| async move { Ok(()) });
//! ```
//!
//! ## W-10 — a blocking step with no pool
//!
//! `run` does not exist on `Node<_, _, Blocking<NoPool>>`.
//!
//! ```compile_fail
//! // expect: E0599
//! use sdax::*;
//! let mut p = Plan::builder("I-29");
//! let _v = p.blocking_step("Verify").run(|_cx, ()| Ok(()));
//! ```
//!
//! ## W-11 — `build` without the run mode (`F3`)
//!
//! Policy, shutdown and mode are required arguments, so omitting one is an
//! arity error rather than a silent default.
//!
//! ```compile_fail
//! // expect: E0061
//! use sdax::*;
//! let mut p = Plan::builder("I-01");
//! p.step("A").run(|_cx, ()| async { Ok(()) });
//! let _ = p.build(Policy::FailFast, Shutdown::within(std::time::Duration::from_secs(10)));
//! ```
//!
//! ## W-12 — a template used where a component is required (`E-R3`)
//!
//! A template is `Plan<Out, In>` with `In != ()`; a component is `Plan<Out>`.
//!
//! ```compile_fail
//! // expect: E0308
//! use sdax::*;
//! struct Link;
//! let mut t = Plan::template::<Link>("Link");
//! t.step("Inner").run(|_cx, ()| async { Ok(()) });
//! let child = t
//!     .build(Policy::Isolate, Shutdown::within(std::time::Duration::from_secs(1)), Mode::Finite)
//!     .unwrap();
//! let mut p = Plan::builder("Mesh");
//! let _net = p.component("Net", &child);
//! ```
//!
//! ## W-13 — a `!Send` body
//!
//! Bodies are spawned tasks, so they must be `Send + 'static`. The compiler
//! names the offending type.
//!
//! ```compile_fail
//! // expect: E0277
//! use sdax::*;
//! use std::cell::Cell;
//! use std::rc::Rc;
//! let mut p = Plan::builder("I-20");
//! let counter = Rc::new(Cell::new(0u8));
//! let _s = p.step("P1").run(move |_cx, ()| {
//!     let counter = counter.clone();
//!     async move {
//!         counter.set(1);
//!         Ok(())
//!     }
//! });
//! ```
//!
//! ## W-16 — an effect that says nothing about its record (`F2`)
//!
//! After `perform` the author must choose: `compensate(..)` or `persistent()`.
//! `NeedsCompensate` is not a `Key`.
//!
//! ```compile_fail
//! // expect: E0308
//! use sdax::*;
//! struct Receipt;
//! let mut p = Plan::builder("I-15");
//! let _r: Key<Receipt> = p
//!     .effect("Registration")
//!     .on_ambiguous(Ambiguity::Report)
//!     .perform(|cx, ()| async move { Ok(cx.hold_value(Receipt)) });
//! ```
//!
//! ## W-17 — forging readiness (`X7`)
//!
//! `Serving` has no public fields, so "ready because I said so" cannot be
//! written; only `Serving::new` with a serve future produces one.
//!
//! ```compile_fail
//! // expect: E0451
//! use sdax::*;
//! fn forge() -> Serving<()> {
//!     Serving { handle: (), serve: Box::pin(async { Ok(()) }) }
//! }
//! ```
