use sdax::prelude::*;
use sdax_tokio::{PlanStart, TokioRuntime};
use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

#[derive(Clone, Default)]
pub struct Env(Arc<Mutex<State>>);
#[derive(Default)]
struct State {
    events: Vec<String>,
    live: BTreeMap<String, Vec<String>>,
    fail: Vec<String>,
}
pub struct Resource {
    pub value: u32,
    name: String,
}
impl Env {
    pub fn failing(names: &[&str]) -> Self {
        let e = Self::default();
        e.0.lock().unwrap().fail = names.iter().map(|x| x.to_string()).collect();
        e
    }
    pub fn record(&self, event: impl Into<String>) {
        self.0.lock().unwrap().events.push(event.into());
    }
    pub fn events(&self) -> Vec<String> {
        self.0.lock().unwrap().events.clone()
    }
    pub async fn open(
        &self,
        name: &str,
        value: u32,
        parents: &[&Resource],
    ) -> Result<Resource, Error> {
        let mut s = self.0.lock().unwrap();
        for p in parents {
            assert!(
                s.live.contains_key(&p.name),
                "parent already closed: {}",
                p.name
            );
        }
        assert!(
            !s.live.contains_key(name),
            "duplicate live resource: {name}"
        );
        s.events.push(format!("open:{name}:{value}"));
        s.live.insert(
            name.to_string(),
            parents.iter().map(|p| p.name.clone()).collect(),
        );
        Ok(Resource {
            value,
            name: name.into(),
        })
    }
    pub async fn close(&self, r: &Resource) -> Result<(), Error> {
        let mut s = self.0.lock().unwrap();
        assert!(
            !s.live.values().any(|ps| ps.contains(&r.name)),
            "live child at close: {}",
            r.name
        );
        assert!(
            s.live.remove(&r.name).is_some(),
            "duplicate close: {}",
            r.name
        );
        s.events.push(format!("close:{}", r.name));
        if s.fail.contains(&r.name) {
            Err(format!("{} cleanup leaf", r.name).into())
        } else {
            Ok(())
        }
    }
    pub fn live_count(&self) -> usize {
        self.0.lock().unwrap().live.len()
    }
}
#[derive(Debug)]
pub struct Cause(pub &'static str, pub Option<Box<Cause>>);
impl std::fmt::Display for Cause {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0)
    }
}
impl std::error::Error for Cause {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        self.1
            .as_deref()
            .map(|x| x as &(dyn std::error::Error + 'static))
    }
}
pub fn runtime() -> (tokio::runtime::Runtime, Arc<TokioRuntime>) {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_time()
        .start_paused(true)
        .build()
        .unwrap();
    let host = Arc::new(TokioRuntime::current_thread_no_background_drain(
        rt.handle().clone(),
    ));
    (rt, host)
}
pub fn finite(plan: &Plan<u32, u32>, input: u32) -> Report<u32> {
    let (rt, host) = runtime();
    let r = rt.block_on(plan.start(host.clone(), input));
    assert_eq!(host.tracked(), 0);
    r
}
pub fn count(e: &[String], needle: &str) -> usize {
    e.iter().filter(|x| x.as_str() == needle).count()
}
pub fn before(e: &[String], a: &str, b: &str) {
    assert_eq!(count(e, a), 1, "{e:?}");
    assert_eq!(count(e, b), 1, "{e:?}");
    assert!(
        e.iter().position(|x| x == a) < e.iter().position(|x| x == b),
        "{a} before {b}: {e:?}"
    );
}
