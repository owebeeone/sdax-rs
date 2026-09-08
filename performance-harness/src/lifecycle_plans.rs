use crate::lifecycle::{Case, Evidence, FixtureError};
use sdax::{Mode, Plan, Policy, Shutdown};
use std::time::Duration;

const BUDGET: Duration = Duration::from_secs(30);

pub fn plan(case: Case, evidence: Evidence) -> Plan<u64, u64> {
    match case {
        Case::Normal => normal_plan(evidence),
        Case::StartupFailure => startup_plan(evidence),
        Case::CleanupFailure => cleanup_plan(evidence),
        Case::Cancellation => cancellation_plan(evidence),
    }
}

fn normal_plan(evidence: Evidence) -> Plan<u64, u64> {
    let mut plan = Plan::with_input::<u64>("lifecycle_normal");
    let input = plan.input();
    let acquire_evidence = evidence.clone();
    let release_evidence = evidence.clone();
    let resource = plan
        .resource("resource")
        .needs(input)
        .acquire(move |cx, value| {
            let evidence = acquire_evidence.clone();
            let value = *value;
            async move {
                evidence.record("acquire_resource");
                Ok(cx.hold_value(evidence.resource(value)))
            }
        })
        .release(move |_cx, _resource| {
            let evidence = release_evidence.clone();
            async move {
                evidence.record("release_resource");
                Ok(())
            }
        });
    let use_evidence = evidence;
    let output = plan.step("use").needs(resource).run(move |_cx, resource| {
        let evidence = use_evidence.clone();
        let value = resource.value;
        async move {
            evidence.record("use_resource");
            Ok(value + 1)
        }
    });
    plan.export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("normal lifecycle plan")
}

fn startup_plan(evidence: Evidence) -> Plan<u64, u64> {
    let mut plan = Plan::with_input::<u64>("lifecycle_startup_failure");
    let input = plan.input();
    let acquire_evidence = evidence.clone();
    let release_evidence = evidence.clone();
    let upstream = plan
        .resource("upstream")
        .needs(input)
        .acquire(move |cx, value| {
            let evidence = acquire_evidence.clone();
            let value = *value;
            async move {
                evidence.record("acquire_upstream");
                Ok(cx.hold_value(evidence.resource(value)))
            }
        })
        .release(move |_cx, _resource| {
            let evidence = release_evidence.clone();
            async move {
                evidence.record("release_upstream");
                Ok(())
            }
        });
    let fail_evidence = evidence;
    let failed = plan.step("failed").needs(upstream).run(move |_cx, _resource| {
        let evidence = fail_evidence.clone();
        async move {
            evidence.record("fail_downstream_run");
            Err::<u64, _>(FixtureError::startup().into())
        }
    });
    plan.export(failed)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("startup lifecycle plan")
}

fn cleanup_plan(evidence: Evidence) -> Plan<u64, u64> {
    let mut plan = Plan::with_input::<u64>("lifecycle_cleanup_failure");
    let input = plan.input();
    let acquire_up = evidence.clone();
    let release_up = evidence.clone();
    let upstream = plan
        .resource("upstream")
        .needs(input)
        .acquire(move |cx, value| {
            let evidence = acquire_up.clone();
            let value = *value;
            async move {
                evidence.record("acquire_upstream");
                Ok(cx.hold_value(evidence.resource(value)))
            }
        })
        .release(move |_cx, _resource| {
            let evidence = release_up.clone();
            async move {
                evidence.record("release_upstream");
                Ok(())
            }
        });
    let acquire_down = evidence.clone();
    let release_down = evidence.clone();
    let downstream = plan
        .resource("downstream")
        .needs(upstream)
        .acquire(move |cx, upstream| {
            let evidence = acquire_down.clone();
            let value = upstream.value;
            async move {
                evidence.record("acquire_downstream");
                Ok(cx.hold_value(evidence.resource(value)))
            }
        })
        .release(move |_cx, _resource| {
            let evidence = release_down.clone();
            async move {
                evidence.record("fail_downstream_release");
                Err(FixtureError::cleanup().into())
            }
        });
    let use_evidence = evidence;
    let output = plan.step("use").needs(downstream).run(move |_cx, resource| {
        let evidence = use_evidence.clone();
        let value = resource.value;
        async move {
            evidence.record("use_resource");
            Ok(value)
        }
    });
    plan.export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("cleanup lifecycle plan")
}

fn cancellation_plan(evidence: Evidence) -> Plan<u64, u64> {
    let mut plan = Plan::with_input::<u64>("lifecycle_cancellation");
    let input = plan.input();
    let acquire_evidence = evidence.clone();
    let release_evidence = evidence;
    let resource = plan
        .resource("resource")
        .needs(input)
        .acquire(move |cx, value| {
            let evidence = acquire_evidence.clone();
            let value = *value;
            async move {
                let held = cx.hold_value(evidence.resource(value));
                evidence.record("hold_resource");
                evidence.signal_ready();
                std::future::pending::<()>().await;
                #[allow(unreachable_code)]
                Ok(held)
            }
        })
        .release(move |_cx, _resource| {
            let evidence = release_evidence.clone();
            async move {
                evidence.record("release_resource");
                Ok(())
            }
        });
    let output = plan
        .step("completed")
        .needs(resource)
        .run(|_cx, resource| {
            let value = resource.value;
            async move { Ok(value) }
        });
    plan.export(output)
        .build(Policy::FailFast, Shutdown::within(BUDGET), Mode::Finite)
        .expect("cancellation lifecycle plan")
}

