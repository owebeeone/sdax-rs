use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior(){let e=Env::failing(&["red","blue"]);let r=finite(&candidate::build(e.clone()),5);let ev=e.events();before(&ev,"close:red","close:trunk");before(&ev,"close:blue","close:trunk");assert_eq!(e.live_count(),0);assert_eq!(r.faults.len(),1,"{r}");assert_eq!(r.cleanup_failures.len(),2,"{r}");match &r.faults[0].kind{FaultKind::Error(err)=>assert!(err.downcast_ref::<Cause>().is_some()),_=>panic!("typed cause lost")};let s=candidate::boundary(r).unwrap_err();for word in ["session","shard","aggregate","run","aggregation refused","checksum mismatch","sector 19","red cleanup leaf","blue cleanup leaf","release"]{assert!(s.contains(word),"missing {word}: {s}");}}
