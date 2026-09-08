//! Performing the machine's effects against a [`Script`](super::Script): what
//! a run driver would do with tasks, timers and threads, done instead by
//! queueing the events the script says would come back.

use super::script::{Cleanup, Ending, Serve};
use super::simulator::{Item, Scripted, Simulator};
use crate::host::engine::{Effect, Event};
use crate::plan::Kind;
use crate::report::{FaultKind, TraceEvent, TraceKind};

impl Simulator {
    /// Perform one effect.
    pub(super) fn perform(&mut self, effect: Effect) {
        let now = self.now;
        match effect {
            Effect::PublishReady { node } => {
                // Scripts model lifecycle only: there are no typed slots to
                // transfer. Acknowledge after the complete effects batch.
                self.publications.push_back(Event::ReadyPublished {
                    node,
                    result: Ok(()),
                });
            }
            Effect::Spawn { node, .. } | Effect::SpawnBlocking { node, .. } => {
                // A run driver holds one task handle per node: when the
                // machine gives up on an attempt and starts the next, the old
                // handle's result no longer maps to anything and the driver
                // drops it. The simulator stands in for one, so it drops the
                // superseded attempt's queued events here. A blocking body is
                // the case that needs it — it cannot be aborted (T7), so
                // `on_within_timer` fails the attempt and says the thread's
                // later outcome is ignored — and nothing else did, so the
                // stale outcome ended the attempt that had just started, which
                // then reached `Ready` before its own `Started` arrived.
                // A cleanup outcome is never pending here: INV-12 finishes a
                // held attempt's release before attempt k+1 starts. A serve
                // episode is not an attempt and is left alone.
                self.queue.retain(|q| match &q.item {
                    Item::Body(k, ev) if *k == node => !matches!(
                        ev,
                        Event::Started(_)
                            | Event::Held(_)
                            | Event::NodeOk(_)
                            | Event::NodeErr(..)
                            | Event::NodeCancelled { .. }
                    ),
                    Item::Spawn(k, _) if *k == node => false,
                    _ => true,
                });
                self.parked.retain(|(k, _)| *k != node);
                let ns = self.node_mut(node);
                let body = ns.bodies[ns.attempts.min(ns.bodies.len() - 1)].clone();
                ns.attempts += 1;
                ns.held = false;
                ns.stopped = false;
                let kind = ns.kind;
                self.push(now, Item::Body(node, Event::Started(node)));
                let holds = matches!(kind, Kind::Resource | Kind::Effect);
                let end_at = match &body.ending {
                    Ending::Ok(at) | Ending::Fail(at, _) | Ending::Panic(at) => {
                        Some(self.resolve(*at, now))
                    }
                    Ending::Pending => None,
                };
                self.arm_spawns(node, now, end_at);
                if holds {
                    let held_at = match (body.held, &body.ending) {
                        (Some(at), _) => Some(self.resolve(at, now)),
                        (None, Ending::Ok(_)) => end_at,
                        _ => None,
                    };
                    if let Some(t) = held_at {
                        // A body registers its value *inside* itself, so a
                        // hold can never be later than the body's own ending.
                        // A script that says otherwise holds at the ending
                        // instant instead, and is delivered first (it was
                        // queued first, so its sequence number is lower).
                        // Without this the machine is fed a `Held` for a body
                        // that has already returned, and rejects it.
                        let t = match end_at {
                            Some(e) => t.min(e),
                            None => t,
                        };
                        self.push(t, Item::Body(node, Event::Held(node)));
                    }
                }
                if let Some(t) = end_at {
                    let ev = match body.ending {
                        Ending::Ok(_) => Event::NodeOk(node),
                        // A try-step's `Err` is its value, not a fault
                        // (OD-5): the body the engine sees returns `Ok`.
                        Ending::Fail(_, _) if kind == Kind::TryStep => Event::NodeOk(node),
                        Ending::Fail(_, msg) => {
                            Event::NodeErr(node, FaultKind::Error(Box::new(Scripted(msg))))
                        }
                        Ending::Panic(_) => {
                            Event::NodeErr(node, FaultKind::Panic(Box::new("scripted panic")))
                        }
                        Ending::Pending => unreachable!(),
                    };
                    self.push(t, Item::Body(node, ev));
                }
            }
            Effect::Serve { node, .. } => self.serve_begins(node),
            Effect::Abort(node) => {
                // Outcomes already due stand; later ones are dropped, and the
                // join reports the cancellation.
                let due_now = self.queue.iter().any(|q| {
                    q.at <= now && matches!(&q.item, Item::Body(k, ev) if *k == node && matches!(ev, Event::NodeOk(_) | Event::NodeErr(..)))
                });
                self.queue.retain(|q| {
                    !(q.at > now
                        && matches!(&q.item, Item::Body(k, _) | Item::Spawn(k, _) if *k == node))
                });
                // A body the engine aborted is not waiting for an instance any
                // more: its `Child::ready()` went with it, and the outcome it
                // was holding back is not "already due" either.
                let parked = self.parked.iter().any(|(k, _)| *k == node);
                self.parked.retain(|(k, _)| *k != node);
                if !due_now || parked {
                    let held = self.node_mut(node).held;
                    self.push(now, Item::Body(node, Event::NodeCancelled { node, held }));
                }
            }
            Effect::Signal(node) | Effect::StopService(node) => {
                let ns = self.node_mut(node);
                if ns.kind == Kind::Service && ns.episodes > 0 && !ns.stopped {
                    ns.stopped = true;
                    let serve = ns.serves[(ns.episodes - 1).min(ns.serves.len() - 1)].clone();
                    if let Serve::StopsAfter(d) = serve {
                        self.push(
                            now + d,
                            Item::Body(node, Event::ServeEnded { node, fault: None }),
                        );
                    }
                }
            }
            // Scripted bodies do not expose a live `Cx`; the Tokio host uses
            // this metadata-only effect to refresh one without cancellation.
            Effect::RefreshDeadline(_) => {}
            Effect::Release(node) | Effect::Compensate(node) | Effect::Recover(node) => {
                let cleanup = self.node_mut(node).cleanup.clone();
                match cleanup {
                    Cleanup::Ok(d) => self.push(now + d, Item::Body(node, Event::NodeOk(node))),
                    Cleanup::Fail(d, msg) => self.push(
                        now + d,
                        Item::Body(
                            node,
                            Event::NodeErr(node, FaultKind::Error(Box::new(Scripted(msg)))),
                        ),
                    ),
                    Cleanup::Panic(d) => self.push(
                        now + d,
                        Item::Body(
                            node,
                            Event::NodeErr(node, FaultKind::Panic(Box::new("scripted panic"))),
                        ),
                    ),
                    Cleanup::IgnoreStop => {}
                }
            }
            Effect::Timer { id, at } => self.push(at, Item::Event(Event::Timer(id))),
            Effect::CancelTimer(id) => self
                .queue
                .retain(|q| !matches!(&q.item, Item::Event(Event::Timer(t)) if *t == id)),
            Effect::Emit(ev) => self.observe(*ev),
            Effect::End(_) => self.ended = true,
            Effect::Reject(r) => self.rejections.push(format!("{} — {}", r.reason, r.event)),
            // The instance's nodes exist now: give each a script of its own,
            // by the path its declaration has in the view.
            Effect::SpawnInstance { id, .. } => self.open_instance(id),
        }
    }

    pub(super) fn observe(&mut self, ev: TraceEvent) {
        if let (TraceKind::Held, Some(_)) = (&ev.kind, &ev.node) {
            if let Some(key) = self.observed_key(&ev) {
                self.node_mut(key).held = true;
            }
        }
        self.trace.events.push(ev);
    }

    /// The run key of the node an observation is about: its path plus, for an
    /// instance's node, the instance its record order names.
    fn observed_key(&self, ev: &TraceEvent) -> Option<crate::key::RawKey> {
        let path = ev.node.as_ref()?.to_string();
        let inst = ev
            .order
            .as_ref()
            .and_then(|o| o.steps.iter().find_map(|(_, i)| *i));
        self.nodes
            .iter()
            .find(|n| {
                n.path == path
                    && match inst {
                        None => self.machine.origin(n.key).map(|(_, i)| i.is_none()) != Some(false),
                        Some(id) => self.machine.origin(n.key).and_then(|(_, i)| i) == Some(id),
                    }
            })
            .map(|n| n.key)
    }

    fn serve_begins(&mut self, key: crate::key::RawKey) {
        let now = self.now;
        let ns = self.node_mut(key);
        if ns.kind != Kind::Service {
            return;
        }
        let serve = ns.serves[ns.episodes.min(ns.serves.len() - 1)].clone();
        ns.episodes += 1;
        ns.stopped = false;
        self.arm_stops(key, now);
        match serve {
            Serve::Ok(at) => {
                let t = self.resolve(at, now);
                self.push(
                    t,
                    Item::Body(
                        key,
                        Event::ServeEnded {
                            node: key,
                            fault: None,
                        },
                    ),
                );
            }
            Serve::Err(at, msg) => {
                let t = self.resolve(at, now);
                self.push(
                    t,
                    Item::Body(
                        key,
                        Event::ServeEnded {
                            node: key,
                            fault: Some(FaultKind::Error(Box::new(Scripted(msg)))),
                        },
                    ),
                );
            }
            Serve::IgnoreStop | Serve::StopsAfter(_) => {}
        }
    }
}
