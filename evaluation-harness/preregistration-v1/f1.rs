use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior() { for (n,failures) in [(3,vec![]),(11,vec![]),(3,vec!["slot-2"])] {let e=Env::failing(&failures);let r=finite(&candidate::build(e.clone()),n);let ev=e.events();for child in ["slot-2","slot-5"]{for parent in ["west","east"]{before(&ev,&format!("close:{child}"),&format!("close:{parent}"));}}assert_eq!(e.live_count(),0);if failures.is_empty(){assert_eq!(candidate::boundary(r).unwrap(),(11*n+16)*1000+(11*n+46));}else{assert!(candidate::boundary(r).unwrap_err().contains("slot-2 cleanup leaf"));}}}
