use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test]
fn behavior() {
    for failures in [vec![], vec!["sleeve"]] {
        let e = Env::failing(&failures);
        let p = candidate::build(e.clone());
        for n in [4, 13] {
            let start = e.events().len();
            let r = finite(&p, n);
            let ev = e.events()[start..].to_vec();
            before(&ev, "close:lens", "close:sleeve");
            before(&ev, "close:sleeve", "close:vault");
            assert_eq!(e.live_count(), 0);
            if failures.is_empty() {
                assert_eq!(candidate::boundary(r).unwrap(), 9 * n + 9);
            } else {
                assert!(candidate::boundary(r)
                    .unwrap_err()
                    .contains("sleeve cleanup leaf"));
            }
        }
    }
}

#[test]
fn diagnostics() {
    let e = Env::failing(&["sleeve"]);
    let r = finite(&candidate::build(e), 5);
    let s = candidate::boundary(r).unwrap_err();
    for word in ["sleeve cleanup leaf", "release"] {
        assert!(s.contains(word), "missing {word}: {s}");
    }
}
