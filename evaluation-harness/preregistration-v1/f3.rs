use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior(){let e=Env::default();let p=candidate::build(e.clone());for n in [2,7]{let start=e.events().len();let r=finite(&p,n);let ev=e.events()[start..].to_vec();let sends:Vec<_>=ev.iter().filter(|x|x.starts_with("send:")).map(|x|x.split(':').collect::<Vec<_>>()).collect();assert_eq!(sends.len(),3,"{ev:?}");for(i,s)in sends.iter().enumerate(){assert_eq!(s[1],(i+1).to_string());assert_eq!(s[2],(100*n+17).to_string());assert_eq!(s[3],n.to_string());assert_eq!(s[4],sends[0][4]);}before(&ev,&format!("recover:3:{}",100*n+17),"close:permit");assert_eq!(count(&ev,"permit-live-at-recovery"),1);assert!(!ev.iter().any(|x|x.starts_with("compensate:")));assert!(r.ambiguous.is_empty(),"{r}");assert!(r.cleanup_failures.is_empty(),"{r}");assert_eq!(e.live_count(),0);let s=candidate::boundary(r).unwrap_err();for word in ["send","prepare","timeout"]{assert!(s.contains(word),"{word}: {s}");}}}
