//! Random scripts and schedules for a generated plan: per node ok / fail /
//! panic / pending / held-then-fail at random ticks, serve outcomes,
//! cleanup outcomes, external requests at random ticks (a cancel before the
//! first poll, a second cancel during cleanup), and a random preference
//! among simultaneously ready nodes.

use super::prng::SplitMix64;
use sdax::*;
use std::time::Duration;

fn secs(g: &mut SplitMix64, lo: u64, hi: u64) -> Duration {
    Duration::from_secs(g.range(lo, hi))
}

fn at(g: &mut SplitMix64) -> At {
    if g.chance(0.7) {
        At::After(secs(g, 0, 3))
    } else {
        At::Tick(secs(g, 0, 8))
    }
}

fn body(g: &mut SplitMix64, kind: Kind, blocking_may_pend: bool) -> Body {
    let r = g.below(100);
    let holds = matches!(kind, Kind::Resource | Kind::Effect);
    match r {
        0..=54 => {
            let b = Body::ok(at(g));
            if holds && g.chance(0.3) {
                b.held(At::After(secs(g, 0, 1)))
            } else {
                b
            }
        }
        55..=69 => Body::fail(at(g), "mc"),
        70..=76 => Body::panic(at(g)),
        77..=86 => {
            if kind == Kind::BlockingStep && !blocking_may_pend {
                Body::ok(at(g))
            } else {
                Body::pending()
            }
        }
        _ => {
            if holds {
                Body::fail(At::After(secs(g, 1, 3)), "after-hold").held(At::After(secs(g, 0, 1)))
            } else {
                Body::fail(at(g), "mc")
            }
        }
    }
}

fn serve(g: &mut SplitMix64) -> Serve {
    match g.below(100) {
        0..=49 => Serve::StopsAfter(secs(g, 0, 2)),
        50..=64 => Serve::IgnoreStop,
        65..=79 => Serve::Ok(At::After(secs(g, 1, 6))),
        _ => Serve::Err(At::After(secs(g, 1, 6)), "serve".into()),
    }
}

fn cleanup(g: &mut SplitMix64, bounded: bool) -> Cleanup {
    match g.below(100) {
        0..=69 => Cleanup::Ok(secs(g, 0, 3)),
        70..=79 => Cleanup::Fail(secs(g, 0, 2), "cleanup".into()),
        80..=84 => Cleanup::Panic(secs(g, 0, 2)),
        _ if bounded => Cleanup::IgnoreStop,
        _ => Cleanup::Ok(secs(g, 0, 3)),
    }
}

/// A random script for `view`. `bounded` says whether the root budget is
/// bounded; an unbounded run never gets a body that can neither complete nor
/// be aborted, so it cannot get stuck.
pub fn generate(g: &mut SplitMix64, view: &PlanView, bounded: bool) -> Script {
    let mut s = Script::new();
    for n in &view.nodes {
        let path = n.path.to_string();
        match n.kind {
            Kind::Join | Kind::Component => {}
            Kind::Service => {
                let attempts = g.range(1, 3) as usize;
                let bodies: Vec<Body> = (0..attempts).map(|_| body(g, n.kind, bounded)).collect();
                s = s.body(&path, bodies);
                let episodes = g.range(1, 3) as usize;
                let serves: Vec<Serve> = (0..episodes).map(|_| serve(g)).collect();
                s = s.serve(&path, serves);
            }
            _ => {
                let attempts = g.range(1, 3) as usize;
                let bodies: Vec<Body> = (0..attempts).map(|_| body(g, n.kind, bounded)).collect();
                s = s.body(&path, bodies);
            }
        }
        if matches!(n.kind, Kind::Resource | Kind::Effect) {
            s = s.cleanup(&path, cleanup(g, bounded));
        }
    }
    let requests = g.below(3);
    let mut shutdown_at: Option<f64> = None;
    for _ in 0..requests {
        let t = g.range(0, 15) as f64 + if g.chance(0.3) { 0.5 } else { 0.0 };
        if g.chance(0.5) {
            s = s.at(t, Request::Shutdown);
            shutdown_at = Some(t);
        } else {
            s = s.at(t, Request::Cancel);
        }
    }
    if let Some(t) = shutdown_at {
        if g.chance(0.4) {
            s = s.at(t + g.range(1, 3) as f64, Request::Cancel);
        }
    }
    // A backstop, well past the window the random requests are drawn from.
    // A resident plan with a service that restarts on error and no limit
    // never ends by itself — correctly — and a walk of runs that never end is
    // a walk that tests nothing. The backstop lands after almost every run has
    // already finished, where it is refused like any other event after `End`.
    s = s.at(20.0 + g.range(0, 5) as f64, Request::Shutdown);
    if g.chance(0.5) {
        let paths: Vec<String> = view.nodes.iter().map(|n| n.path.to_string()).collect();
        let perm = g.permutation(paths.len());
        s = s.schedule(Schedule::order(perm.into_iter().map(|i| paths[i].clone())));
    }
    s
}
