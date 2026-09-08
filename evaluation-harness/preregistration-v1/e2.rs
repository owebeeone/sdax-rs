use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior() { for failures in [vec![],vec!["derived"]] { let e=Env::failing(&failures); let p=candidate::build(e.clone()); let r=finite(&p,9); let ev=e.events(); assert_eq!(count(&ev,"attempt:rates:1"),1); assert_eq!(count(&ev,"attempt:rates:2"),1); assert_eq!(ev.iter().filter(|x|x.starts_with("attempt:rates:")).count(),2); before(&ev,"close:derived","close:rates");before(&ev,"close:rates","close:base");assert_eq!(e.live_count(),0);if failures.is_empty(){assert_eq!(candidate::boundary(r).unwrap(),16);}else{assert!(candidate::boundary(r).unwrap_err().contains("derived cleanup leaf"));}} }
