//! The bookkeeping the plan generator needs: the one value type every
//! generated node produces, the keys a scope has declared so far, the needs
//! and attributes chosen for one node, and the code that turns those choices
//! into a call on the author surface.

use sdax::*;
use std::time::Duration;

/// The one value every generated node produces.
pub(crate) struct Unit;
pub(crate) fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}
/// The keys a scope has declared so far, by value type.
#[derive(Default)]
pub(crate) struct Keys {
    pub(crate) units: Vec<(Key<Unit>, bool)>,
    pub(crate) trys: Vec<(Key<Result<Unit, Error>>, bool)>,
    pub(crate) joins: Vec<Key<()>>,
}
/// The needs chosen for one node.
pub(crate) struct Needs {
    pub(crate) units: Vec<Key<Unit>>,
    pub(crate) try_: Option<Key<Result<Unit, Error>>>,
    pub(crate) join: Option<Key<()>>,
}
/// The attributes chosen for one node.
#[derive(Clone)]
pub(crate) struct Attrs {
    pub(crate) within: Option<Duration>,
    pub(crate) retry: Option<Retry>,
    pub(crate) idempotent: bool,
    pub(crate) exclusive: Option<Key<Unit>>,
    pub(crate) shared: Option<Key<Unit>>,
    pub(crate) limit: Option<Pool>,
    pub(crate) cooperative: Option<Duration>,
    pub(crate) stop_within: Option<Duration>,
    pub(crate) restart: Option<Restart>,
    pub(crate) terminal: bool,
    pub(crate) ambiguity: Ambiguity,
    pub(crate) persistent: bool,
    pub(crate) by_drop: bool,
    pub(crate) pool: Option<Pool>,
}

impl Default for Attrs {
    fn default() -> Attrs {
        Attrs {
            within: None,
            retry: None,
            idempotent: false,
            exclusive: None,
            shared: None,
            limit: None,
            cooperative: None,
            stop_within: None,
            restart: None,
            terminal: false,
            ambiguity: Ambiguity::Report,
            persistent: false,
            by_drop: false,
            pool: None,
        }
    }
}
impl Attrs {
    pub(crate) fn describe(&self) -> String {
        let mut s = String::new();
        if let Some(d) = self.within {
            s.push_str(&format!(" within {d:?}"));
        }
        if let Some(r) = self.retry {
            s.push_str(&format!(" retry {}", r.max_attempts()));
        }
        if self.idempotent {
            s.push_str(" idempotent");
        }
        if self.exclusive.is_some() {
            s.push_str(" exclusive");
        }
        if self.shared.is_some() {
            s.push_str(" shared");
        }
        if self.limit.is_some() || self.pool.is_some() {
            s.push_str(" pool");
        }
        if let Some(g) = self.cooperative {
            s.push_str(&format!(" cooperative {g:?}"));
        }
        if let Some(d) = self.stop_within {
            s.push_str(&format!(" stop_within {d:?}"));
        }
        if self.restart.is_some() {
            s.push_str(" restart");
        }
        if self.terminal {
            s.push_str(" terminal");
        }
        s
    }
}
pub(crate) fn common<'p, D: Deps, K>(mut n: Node<'p, D, K>, a: &Attrs) -> Node<'p, D, K> {
    if let Some(d) = a.within {
        n = n.within(d);
    }
    if let Some(r) = a.retry {
        n = n.retry(r);
    }
    if a.idempotent {
        n = n.idempotent();
    }
    if let Some(k) = a.exclusive {
        n = n.exclusive(k);
    }
    if let Some(k) = a.shared {
        n = n.shared(k);
    }
    if let Some(p) = a.limit {
        n = n.limit(p);
    }
    if let Some(g) = a.cooperative {
        n = n.cooperative(g);
    }
    n
}
fn add_one<D: Deps>(
    p: &mut PlanBuilder,
    kind: Kind,
    name: &str,
    deps: D,
    a: &Attrs,
    keys: &mut Keys,
) {
    match kind {
        Kind::Resource => {
            let n = common(p.resource(name).needs(deps), a)
                .acquire(|cx, _d| async move { Ok(cx.hold_value(Unit)) });
            let k = if a.by_drop {
                n.release(release::by_drop())
            } else {
                n.release(|_cx, _u| async move { Ok(()) })
            };
            keys.units.push((k, true));
        }
        Kind::Step => {
            let k = common(p.step(name).needs(deps), a).run(|_cx, _d| async move { Ok(Unit) });
            keys.units.push((k, false));
        }
        Kind::TryStep => {
            let k = common(p.try_step(name).needs(deps), a).run(|_cx, _d| async move { Ok(Unit) });
            keys.trys.push((k, false));
        }
        Kind::BlockingStep => {
            let k = common(p.blocking_step(name).needs(deps), a)
                .on(a.pool.expect("a blocking step has a pool"))
                .run(|_cx, _d| Ok(Unit));
            keys.units.push((k, false));
        }
        Kind::Service => {
            let mut n = common(p.service(name).needs(deps), a);
            if let Some(d) = a.stop_within {
                n = n.stop_within(d);
            }
            if let Some(r) = a.restart {
                n = n.restart(r);
            }
            if a.terminal {
                n = n.terminal();
            }
            let k = n.start(|_cx, _d| async move { Ok(Serving::new(Unit, async { Ok(()) })) });
            keys.units.push((k, false));
        }
        Kind::Effect => {
            let n = common(p.effect(name).needs(deps), a)
                .on_ambiguous(a.ambiguity)
                .perform(|cx, _d| async move { Ok(cx.hold_value(Unit)) });
            let k = if a.persistent {
                n.persistent()
            } else {
                n.compensate(|_cx, _u| async move { Ok(()) })
            };
            keys.units.push((k, false));
        }
        Kind::Join => keys.joins.push(p.join(name, deps)),
        _ => unreachable!("components are added by the caller"),
    }
}
pub(crate) fn add_node(
    p: &mut PlanBuilder,
    kind: Kind,
    name: &str,
    needs: &Needs,
    a: &Attrs,
    keys: &mut Keys,
) {
    let u = &needs.units;
    match (u.len(), needs.try_, needs.join) {
        (0, None, None) => add_one(p, kind, name, (), a, keys),
        (1, None, None) => add_one(p, kind, name, u[0], a, keys),
        (_, None, None) => add_one(p, kind, name, (u[0], u[1]), a, keys),
        (0, Some(t), None) => add_one(p, kind, name, t, a, keys),
        (1, Some(t), None) => add_one(p, kind, name, (u[0], t), a, keys),
        (_, Some(t), None) => add_one(p, kind, name, (u[0], u[1], t), a, keys),
        (0, None, Some(j)) => add_one(p, kind, name, j, a, keys),
        (1, None, Some(j)) => add_one(p, kind, name, (u[0], j), a, keys),
        (_, None, Some(j)) => add_one(p, kind, name, (u[0], u[1], j), a, keys),
        (0, Some(t), Some(j)) => add_one(p, kind, name, (t, j), a, keys),
        (1, Some(t), Some(j)) => add_one(p, kind, name, (u[0], t, j), a, keys),
        (_, Some(t), Some(j)) => add_one(p, kind, name, (u[0], u[1], t, j), a, keys),
    }
}
