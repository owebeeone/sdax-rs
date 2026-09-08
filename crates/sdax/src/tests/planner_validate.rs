//! Suite (b), the validate rows: `CanonicalTests.md` § 3 P-01, P-02, P-05…P-12,
//! plus the two rules the adopted contract adds (`V-MODE`, `V-SPAWN-SELF-IMPORT`).

use super::corpus::*;
use crate::host::RawKey;
use crate::*;
use std::sync::Arc;

fn invalid<O, I>(r: Result<Plan<O, I>, Invalid>) -> Invalid {
    match r {
        Ok(p) => panic!("expected {p:?} to be rejected at build"),
        Err(inv) => inv,
    }
}

fn only(inv: &Invalid, rule: Rule) -> &Finding {
    let hits: Vec<&Finding> = inv.checks.iter().filter(|f| f.rule == rule).collect();
    assert_eq!(
        hits.len(),
        1,
        "expected exactly one {} in {:?}",
        rule.id(),
        inv
    );
    hits[0]
}

/// P-01 — a key of another plan named in this one.
#[test]
fn p01_foreign_key_is_rejected_naming_the_node_the_key_and_both_plans() {
    let mut other = Plan::builder("Conn");
    let session = other.step("Session").run(|_cx, ()| async move { Ok(()) });
    let other_id = other.id();
    let _ = other.build(Policy::FailFast, Shutdown::within(secs(1)), Mode::Finite);

    let mut p = Plan::builder("Process");
    p.service("Reporter")
        .needs(session)
        .stop_within(secs(1))
        .initialize(|_cx, _s: Arc<()>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident));
    let f = only(&inv, Rule::ForeignKey);
    assert_eq!(f.nodes, ["Reporter"]);
    assert_eq!(f.keys, [session.raw()]);
    assert!(
        f.detail.contains(&other_id.to_string()),
        "names the owning plan: {}",
        f.detail
    );
    assert!(!f.fix.is_empty());
}

/// P-02 — two nodes with one name in one scope.
#[test]
fn p02_duplicate_names_are_rejected() {
    let inv = invalid(i18(secs(3), true));
    let f = only(&inv, Rule::DupName);
    assert_eq!(f.nodes, ["Db", "Db"]);
}

/// P-05 — budget arithmetic.
#[test]
fn p05_stop_within_must_fit_the_shutdown_budget() {
    assert!(
        i18(secs(3), false).is_ok(),
        "3s inside a 10s budget is accepted"
    );
    let inv = invalid(i18(secs(30), false));
    let f = only(&inv, Rule::BudgetOrder);
    assert_eq!(f.nodes, ["Worker"]);
    assert!(
        f.detail.contains("30s") && f.detail.contains("10s"),
        "{}",
        f.detail
    );
}

/// P-05 variant — a component whose own budget exceeds its parent's.
#[test]
fn p05_a_components_budget_must_fit_its_parents() {
    assert!(i32(Shutdown::within(secs(5))).is_ok());
    let inv = invalid(i32(Shutdown::within(secs(20))));
    let f = only(&inv, Rule::BudgetOrder);
    assert_eq!(f.nodes, ["Net"]);
}

/// P-06 — re-execution and evidence-free compensation require `idempotent`.
#[test]
fn p06_idempotency_is_required_for_retry_restart_and_compensating_ambiguity() {
    let inv = invalid(i15_with(I15Opts {
        retry: true,
        ..I15Opts::default()
    }));
    let f = only(&inv, Rule::IdempotentRequired);
    assert_eq!(f.nodes, ["Registration"]);
    assert!(f.detail.contains("retry"), "{}", f.detail);

    let inv = invalid(i15_with(I15Opts {
        ambiguity: Ambiguity::Recover,
        ..I15Opts::default()
    }));
    assert!(only(&inv, Rule::IdempotentRequired)
        .detail
        .contains("on_ambiguous"));

    assert!(
        i15_with(I15Opts {
            retry: true,
            idempotent: true,
            ..I15Opts::default()
        })
        .is_ok(),
        "declaring idempotent satisfies the rule"
    );
}

/// P-06 (restart arm) — a restarted service must be idempotent too.
#[test]
fn p06_a_restarted_service_must_be_idempotent() {
    fn build(idempotent: bool) -> Result<Plan, Invalid> {
        let mut p = Plan::builder("Renew");
        let svc = p
            .service("Renew")
            .stop_within(secs(2))
            .restart(Restart::on_error(Backoff::fixed(secs(1))));
        let svc = if idempotent { svc.idempotent() } else { svc };
        svc.initialize(|_cx, ()| async move { Ok(()) })
            .serve(|_cx, _handle| async move { Ok(()) });
        p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
    }
    assert!(build(true).is_ok());
    let inv = invalid(build(false));
    assert!(only(&inv, Rule::IdempotentRequired)
        .detail
        .contains("restart"));
}

/// P-07 — a child plan importing a key of an unrelated plan.
#[test]
fn p07_import_scope_is_checked_at_registration() {
    let mut unrelated = Plan::builder("Other");
    let stray = unrelated
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });

    let mut p = Plan::builder("Mesh");
    let _own = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let tpl = link_plan(stray).expect("the child plan itself is valid");
    p.template("Link", &tpl);
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident));
    let f = only(&inv, Rule::ImportScope);
    assert_eq!(f.nodes, ["Link"]);
    assert_eq!(f.keys, [stray.raw()]);
}

/// P-07 variant — two levels of nesting, each declaring its own import.
#[test]
fn p07_a_two_level_nesting_is_accepted_when_each_level_imports() {
    let mut root = Plan::builder("Root");
    let endpoint = root
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });

    let mut mid = Plan::with_input::<PeerLink>("Mid");
    let mid_ep = mid.import(endpoint);
    let grandchild = {
        let mut g = Plan::with_input::<PeerLink>("Grandchild");
        let g_ep = g.import(mid_ep);
        g.resource("Entry")
            .needs(g_ep)
            .acquire(|cx, _e: Arc<Endpoint>| async move { Ok(cx.hold_value(LinkEntry)) })
            .release(|_cx, _e| async move { Ok(()) });
        g.build(Policy::Isolate, Shutdown::within(secs(1)), Mode::Finite)
            .expect("valid")
    };
    let gtpl = mid.template("Grandchild", &grandchild);
    mid.service("Spawner")
        .stop_within(secs(1))
        .spawns(&gtpl)
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let mid = mid
        .build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident)
        .expect("valid");

    let mtpl = root.template("Mid", &mid);
    root.service("AcceptLoop")
        .needs(endpoint)
        .spawns(&mtpl)
        .stop_within(secs(2))
        .initialize(|_cx, _e: Arc<Endpoint>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    assert!(root
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .is_ok());
}

/// P-08 — a lock on a resource the node does not need.
#[test]
fn p08_a_lock_must_name_a_declared_need() {
    assert!(i27(true).is_ok());
    let inv = invalid(i27(false));
    let f = only(&inv, Rule::LockNeeds);
    assert_eq!(f.nodes, ["MigB"]);
}

/// P-09 — resident holders that can starve a pool's other users.
#[test]
fn p09_pool_starvation_is_rejected() {
    assert!(i29(2, 0).is_ok(), "nobody else holds the pool");
    assert!(
        i29(3, 2).is_ok(),
        "two resident holders inside a pool of three"
    );
    let inv = invalid(i29(2, 2));
    let f = only(&inv, Rule::PoolStarve);
    assert!(
        f.detail.contains("cpu") && f.detail.contains("Verify"),
        "{}",
        f.detail
    );
}

/// P-09 variant — pools are per plan, so a child plan cannot take its
/// parent's pool at all: the mutant is caught one rule earlier.
#[test]
fn p09_a_child_plan_cannot_take_a_parent_pool() {
    let mut p = Plan::builder("Mesh");
    let cpu = p.pool("cpu", 3);
    let mut t = Plan::with_input::<PeerLink>("Link");
    t.service("Worker")
        .limit(cpu)
        .stop_within(secs(1))
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let inv = invalid(t.build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Resident));
    let f = only(&inv, Rule::ForeignKey);
    assert_eq!(f.nodes, ["Worker"]);
    assert!(f.detail.contains("pool"), "{}", f.detail);
}

/// P-09 variant, white box — when a template's instances *do* hold a pool of
/// this plan, the holder count is unbounded and every other user is starved.
#[test]
fn p09_a_template_holder_counts_as_unbounded() {
    let plan = i29(2, 0).expect("valid");
    let mut ir = (*plan.ir).clone();
    // Give the plan a template node whose instances take `cpu`.
    let pool = ir
        .nodes
        .iter()
        .find(|n| n.name == "Verify")
        .unwrap()
        .attrs
        .pool
        .unwrap();
    let tpl_key = RawKey {
        plan: ir.id,
        idx: ir.nodes.len() as u32,
    };
    let mut child = ir.clone();
    child.nodes.clear();
    let mut worker = plan.ir.nodes[0].clone();
    worker.name = "Worker".into();
    worker.kind = crate::plan::Kind::Service;
    worker.attrs.limit = Some(pool);
    child.nodes.push(worker);
    let mut tpl = plan.ir.nodes[0].clone();
    tpl.key = tpl_key;
    tpl.name = "Link".into();
    tpl.kind = crate::plan::Kind::Template;
    tpl.needs.clear();
    tpl.child = Some(std::sync::Arc::new(child));
    ir.nodes.push(tpl);
    let checks = crate::validate::validate(&ir);
    let starve = checks
        .iter()
        .find(|f| f.rule == Rule::PoolStarve)
        .expect("V-POOL-STARVE");
    assert!(starve.detail.contains("unbounded"), "{}", starve.detail);
    assert_eq!(starve.nodes, ["Verify"]);
}

/// P-10 — an unused pool, an unconsumed try-step, an empty plan.
#[test]
fn p10_unused_pool_unconsumed_try_step_and_empty_plan() {
    let mut p = Plan::builder("Unused");
    p.pool("cpu", 2);
    p.step("Only").run(|_cx, ()| async move { Ok(()) });
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(1)), Mode::Finite));
    assert!(only(&inv, Rule::UnusedPool).detail.contains("cpu"));

    let inv = invalid(i23(false));
    let f = only(&inv, Rule::TryUnconsumed);
    assert_eq!(f.nodes, ["Probe1", "Probe2", "Probe3", "Probe4", "Probe5"]);
    assert!(i23(true).is_ok());

    let empty = Plan::builder("Empty");
    let inv = invalid(empty.build(Policy::FailFast, Shutdown::within(secs(1)), Mode::Finite));
    assert_eq!(only(&inv, Rule::Empty).rule.id(), "V-EMPTY");
}

/// P-11 — an unbounded shutdown requires every service to bound its stop.
#[test]
fn p11_an_unbounded_shutdown_requires_stop_within_on_every_service() {
    assert!(i07(Shutdown::unbounded(), Some(secs(2))).is_ok());
    let inv = invalid(i07(Shutdown::unbounded(), None));
    let f = only(&inv, Rule::ServiceUnbounded);
    assert_eq!(f.nodes, ["Exporter"]);
}

/// P-12 — an attribute set twice.
#[test]
fn p12_a_duplicate_attribute_is_rejected() {
    assert!(i24(false).is_ok());
    let inv = invalid(i24(true));
    let f = only(&inv, Rule::DupAttr);
    assert_eq!(f.nodes, ["Conn"]);
    assert!(f.detail.contains("retry"), "{}", f.detail);
}

/// F3 — `Mode` is explicit, and `Finite` with a service or template is rejected.
#[test]
fn v_mode_rejects_a_finite_plan_that_contains_a_service_or_a_template() {
    let mut p = Plan::builder("Daemon");
    p.service("Loop")
        .stop_within(secs(1))
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite));
    let f = only(&inv, Rule::Mode);
    assert_eq!(f.nodes, ["Loop"]);
    assert!(f.detail.contains("finite"), "{}", f.detail);
}

/// F3 — a resources-only plan may still be `Resident`; nothing is derived.
#[test]
fn v_mode_allows_a_resident_plan_with_no_services() {
    let mut p = Plan::builder("Resident resources");
    p.resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    assert!(p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .is_ok());
}

/// F1 — a template a service spawns must not import that service's own key.
#[test]
fn v_spawn_self_import_rejects_a_readiness_deadlock() {
    let mut p = Plan::builder("Mesh");
    let endpoint = p
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let accept = p
        .service("AcceptLoop")
        .needs(endpoint)
        .stop_within(secs(2))
        .initialize(|_cx, _e: Arc<Endpoint>| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let tpl = {
        let mut t = Plan::with_input::<PeerLink>("Link");
        let a = t.import(accept);
        let conn = t.input();
        t.resource("LinkEntry")
            .needs((conn, a))
            .acquire(|cx, _d: (Arc<PeerLink>, Arc<()>)| async move { Ok(cx.hold_value(LinkEntry)) })
            .release(|_cx, _e| async move { Ok(()) });
        t.build(Policy::Isolate, Shutdown::within(secs(2)), Mode::Finite)
            .expect("valid")
    };
    let links = p.template("Link", &tpl);
    p.spawns(accept, &links);
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident));
    let f = only(&inv, Rule::SpawnSelfImport);
    assert_eq!(f.nodes, ["AcceptLoop", "Link"]);
    assert_eq!(f.keys, [accept.raw()]);
    // I-30 spawns a template that imports only the endpoint: accepted.
    assert!(i30().is_ok());
}

/// F1 — the late `spawns` form may only attach a template to a service.
#[test]
fn v_spawn_kind_rejects_a_late_spawns_on_a_node_that_is_not_a_service() {
    let mut p = Plan::builder("Mesh");
    let db = p
        .resource("Db")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let tpl = p.template("Link", &plain_link().expect("valid"));
    p.spawns(db, &tpl);
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident));
    let f = only(&inv, Rule::SpawnKind);
    assert_eq!(f.nodes, ["Db", "Link"]);
    assert!(
        f.detail.contains("resource") && f.detail.contains("Link"),
        "names the node's kind and the template: {}",
        f.detail
    );
    assert!(!f.fix.is_empty());
}

/// F1 — the chain form is unaffected: a service may still declare `spawns`,
/// in either form.
#[test]
fn a_service_may_spawn_in_the_chain_form_and_in_the_late_form() {
    assert!(i30().is_ok(), "the chain form is accepted");

    let mut p = Plan::builder("Mesh");
    let tpl = p.template("Link", &plain_link().expect("valid"));
    let accept = p
        .service("AcceptLoop")
        .stop_within(secs(2))
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    p.spawns(accept, &tpl);
    let plan = p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident)
        .expect("a service may spawn");
    assert_eq!(
        plan.inspect()
            .node("AcceptLoop")
            .expect("AcceptLoop")
            .spawns,
        vec![NodePath::root("Link")]
    );
}

/// A late `spawns` naming a key of another plan is a finding, not a silent
/// no-op.
#[test]
fn v_foreign_key_catches_a_late_spawns_whose_key_belongs_to_another_plan() {
    let mut other = Plan::builder("Other");
    let stray = other
        .service("Stray")
        .stop_within(secs(1))
        .initialize(|_cx, ()| async move { Ok(()) })
        .serve(|_cx, _handle| async move { Ok(()) });
    let other_id = other.id();
    let _ = other.build(Policy::FailFast, Shutdown::within(secs(2)), Mode::Resident);

    let mut p = Plan::builder("Mesh");
    let mine = p.id();
    let tpl = p.template("Link", &plain_link().expect("valid"));
    p.spawns(stray, &tpl);
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Resident));
    let f = only(&inv, Rule::ForeignKey);
    assert_eq!(f.nodes, ["Link"]);
    assert_eq!(f.keys, [stray.raw()]);
    assert!(
        f.detail.contains(&other_id.to_string()) && f.detail.contains(&mine.to_string()),
        "names both plans: {}",
        f.detail
    );
}

/// P-12 (locks) — one resource locked twice, or in two modes, on one node.
#[test]
fn p12_repeated_or_conflicting_lock_attributes_are_rejected() {
    fn build(twice: bool, both_modes: bool) -> Result<Plan, Invalid> {
        let mut p = Plan::builder("Locks");
        let db = p
            .resource("Db")
            .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
            .release(|_cx, _d| async move { Ok(()) });
        let mut mig = p.step("Mig").needs(db).exclusive(db);
        if twice {
            mig = mig.exclusive(db);
        }
        if both_modes {
            mig = mig.shared(db);
        }
        mig.run(|_cx, _d: Arc<Db>| async move { Ok(()) });
        p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
    }
    assert!(build(false, false).is_ok(), "one lock in one mode is fine");

    let inv = invalid(build(true, false));
    let f = only(&inv, Rule::DupAttr);
    assert_eq!(f.nodes, ["Mig"]);
    assert!(
        f.detail.contains("exclusive") && f.detail.contains("Db"),
        "names the attribute and the key: {}",
        f.detail
    );

    let inv = invalid(build(false, true));
    let f = only(&inv, Rule::DupAttr);
    assert!(
        f.detail.contains("exclusive") && f.detail.contains("shared") && f.detail.contains("Db"),
        "names both modes and the key: {}",
        f.detail
    );

    // Two different resources, one mode each, is not a duplicate.
    let mut p = Plan::builder("Two locks");
    let a = p
        .resource("A")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    let b = p
        .resource("B")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Db)) })
        .release(|_cx, _d| async move { Ok(()) });
    p.step("Mig")
        .needs((a, b))
        .exclusive(a)
        .exclusive(b)
        .run(|_cx, _d: (Arc<Db>, Arc<Db>)| async move { Ok(()) });
    assert!(p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .is_ok());
}

/// Every finding names its rule id, the node(s) and a fix.
#[test]
fn findings_are_values_that_name_the_rule_and_a_fix() {
    let inv = invalid(i18(secs(3), true));
    assert_eq!(inv.checks[0].rule.id(), "V-DUP-NAME");
    assert!(!inv.checks[0].fix.is_empty());
    let rendered = inv.to_string();
    assert!(
        rendered.contains("V-DUP-NAME") && rendered.contains("Db"),
        "{rendered}"
    );
}

/// L-IMPORTS — a child plan started as a root is refused before any effect.
#[test]
fn l_imports_lists_what_a_root_run_could_not_resolve() {
    let mut root = Plan::builder("Root");
    let endpoint = root
        .resource("Endpoint")
        .acquire(|cx, ()| async move { Ok(cx.hold_value(Endpoint)) })
        .release(|_cx, _e| async move { Ok(()) });
    let child = link_plan(endpoint).expect("valid");
    assert_eq!(
        child.unresolved_imports(),
        vec![NodePath::root("import#1")],
        "the import node of the child, named the way every other node is"
    );
    assert!(i01().expect("valid").unresolved_imports().is_empty());
}

/// `V-BLOCKING-CANCEL` (F-04) — the grace of `.cooperative(g)` on a blocking
/// step can never be spent: a thread is signalled and never aborted.
#[test]
fn v_blocking_cancel_rejects_a_cooperative_grace_on_a_blocking_step() {
    let mut p = Plan::builder("Blocking");
    let cpu = p.pool("cpu", 1);
    p.blocking_step("Verify")
        .on(cpu)
        .cooperative(secs(1))
        .run(|_cx, ()| Ok(()));
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite));
    let f = only(&inv, Rule::BlockingCancel);
    assert_eq!(f.nodes, ["Verify"]);
    assert!(f.fix.contains("is_stopping"), "{f}");
}

/// And the same step without the attribute is valid: `cx.is_stopping()` works
/// in a blocking body regardless, because the signal is unconditional.
#[test]
fn v_blocking_cancel_allows_a_blocking_step_with_no_cancel_attribute() {
    let mut p = Plan::builder("Blocking");
    let cpu = p.pool("cpu", 1);
    p.blocking_step("Verify").on(cpu).run(|_cx, ()| Ok(()));
    assert!(p
        .build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite)
        .is_ok());
}

/// `V-PERSIST-AMBIG` (F-09) — a persistent effect has nothing to compensate,
/// and the pair also forced a meaningless `.idempotent()` through
/// `V-IDEMPOTENT-REQUIRED`.
#[test]
fn v_persist_ambig_rejects_a_persistent_effect_that_compensates_an_ambiguity() {
    let mut p = Plan::builder("Persist");
    p.effect("Charge")
        .idempotent()
        .on_ambiguous(Ambiguity::Recover)
        .perform(|cx, ()| async move { Ok(cx.hold_value(Receipt("r"))) })
        .persistent();
    let inv = invalid(p.build(Policy::FailFast, Shutdown::within(secs(10)), Mode::Finite));
    let f = only(&inv, Rule::RecoveryMissing);
    assert_eq!(f.nodes, ["Charge"]);
    assert!(f.fix.contains("identified_by"), "{f}");
}
