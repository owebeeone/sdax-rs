use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior(){let e=Env::default();let p=candidate::build(e.clone());for n in [6,19]{let start=e.events().len();let r=finite(&p,n);let ev=e.events()[start..].to_vec();for event in [format!("ack:{}",2*n),format!("unknown:{}",2*n+1),format!("undo:{}",2*n+900),format!("reconcile:{}",2*n+1)]{assert_eq!(count(&ev,&event),1,"{ev:?}");}assert_eq!(ev.len(),4,"{ev:?}");assert_eq!(r.ambiguous.len(),1,"{r}");assert!(r.cleanup_failures.is_empty(),"{r}");let s=candidate::boundary(r).unwrap_err();for word in ["pending","dispatch","ambiguous"]{assert!(s.contains(word),"{word}: {s}");}}}
