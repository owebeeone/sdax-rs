use crate::candidate;
use crate::support::*;
use sdax::prelude::*;
use sdax_tokio::PlanStart;
use std::time::Duration;
#[test] fn behavior(){for (wait,expected)in [(50,2),(90,3)]{let e=Env::default();let p=candidate::build(e.clone());let(rt,host)=runtime();let r=rt.block_on(async{let mut run=p.start(host.clone(),12);run.ready().await.unwrap();tokio::time::sleep(Duration::from_millis(wait)).await;run.shutdown();run.await});assert_eq!(host.tracked(),0);assert!(r.is_clean(),"{r}");assert_eq!(candidate::boundary(r).unwrap(),18);let ev=e.events();assert_eq!(count(&ev,"init:1"),1);assert_eq!(count(&ev,"init:2"),1);let episodes:Vec<_>=ev.iter().filter(|x|x.starts_with("serve:")).map(|x|x.split(':').collect::<Vec<_>>()).collect();assert_eq!(episodes.len(),expected,"{ev:?}");for(i,x)in episodes.iter().enumerate(){assert_eq!(x[1],(i+1).to_string());assert_eq!(x[2],"18");assert_eq!(x[3],episodes[0][3]);}assert_eq!(count(&ev,&format!("observer:18:{}",episodes[0][3])),1);assert_eq!(count(&ev,"cooperatively-stopped"),if expected==3{1}else{0});assert_eq!(count(&ev,"close:cable"),1);if expected==3{before(&ev,"cooperatively-stopped","close:cable");}assert_eq!(e.live_count(),0);}}
