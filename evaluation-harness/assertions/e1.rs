use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test]
fn behavior() {
    for failures in [vec![], vec!["derived"]] {
        let e = Env::failing(&failures);
        let p = candidate::build(e.clone());
        let r = finite(&p, 8);
        let ev = e.events();
        before(&ev, "close:derived", "close:base");
        assert_eq!(e.live_count(), 0);
        if failures.is_empty() {
            assert_eq!(candidate::boundary(r).unwrap(), 26);
        } else {
            let s = candidate::boundary(r).unwrap_err();
            assert!(s.contains("derived cleanup leaf"), "{s}");
        }
    }
}

#[test]
fn diagnostics() {
    let e = Env::failing(&["derived"]);
    let r = finite(&candidate::build(e), 5);
    let s = candidate::boundary(r).unwrap_err();
    for word in ["derived cleanup leaf", "release"] {
        assert!(s.contains(word), "missing {word}: {s}");
    }
}
