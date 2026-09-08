//! T1 — start eligibility, atomic FIFO grants, and what follows a start:
//! steady-state detection and skip propagation (T4 under `Isolate`).

use super::state::{Cause, Machine, Purpose, St};
use super::{Effect, RunState};
use crate::plan::Kind;
use crate::report::{Phase, TraceKind};

impl Machine {
    /// T1 for one scope, to a fixpoint: a start can make a join ready, which
    /// can make another node need-ready in the same step.
    pub(super) fn admit(&mut self, scope: usize) {
        // A steady scope admits again when a restarted service comes out of
        // its backoff; `check_steady` returns it to Steady afterwards.
        if self.scopes[scope].st == RunState::Steady {
            self.scopes[scope].st = RunState::Admitting;
        }
        loop {
            if self.scopes[scope].st != RunState::Admitting {
                return;
            }
            // 0. A node can *lose* a need after it is queued: a service
            //    that was `Ready` fails its serve and restarts. It goes back
            //    to `Pending` — starting it would break INV-1, and leaving it
            //    queued would let it hold the lock its own need wants back.
            for n in self.t.scopes[scope].nodes.clone() {
                if self.slots[n].st == St::Waiting && !self.needs_ready(n) {
                    self.slots[n].st = St::Pending;
                    self.slots[n].queued = None;
                }
            }
            // 1. Newly need-ready nodes join the FIFO queue, the schedule's
            //    preference deciding among those that became ready together.
            let mut newly: Vec<usize> = self.t.scopes[scope]
                .nodes
                .iter()
                .copied()
                .filter(|&n| self.slots[n].st == St::Pending && self.needs_ready(n))
                .collect();
            newly.sort_by_key(|&n| (self.rank[n], n));
            for n in newly {
                self.seq += 1;
                self.slots[n].queued = Some(self.seq);
                self.slots[n].st = St::Waiting;
            }
            // 2. Grants in queue order, all or none, an earlier waiter's
            //    unmet wants blocking a later waiter for the same grant.
            let mut waiters: Vec<usize> = self.t.scopes[scope]
                .nodes
                .iter()
                .copied()
                .filter(|&n| self.slots[n].st == St::Waiting)
                .collect();
            waiters.sort_by_key(|&n| self.slots[n].queued);
            // Per grant, the first waiter that wanted it and did not get it.
            // One flag for every pool queued a waiter for a *free* pool behind
            // a waiter for a full one, which INV-1 forbids: arbitration is
            // "among nodes that declare the same lock or pool", and nothing
            // else orders starts.
            let mut blocked_locks: Vec<(usize, usize)> = Vec::new();
            let mut blocked_pools: Vec<(usize, usize)> = Vec::new();
            let mut started = false;
            for n in waiters {
                // The list above was taken before the first `start`, and a
                // `start` can re-enter `admit` for this same scope: a
                // component's inner graph comes up inside it, and the `Ready`
                // that follows admits the parent again. That nested pass may
                // already have granted and started a later waiter, so the
                // slot's state is the authority, not the list. Without this a
                // node was started twice with two attempts in flight at once
                // (INV-12), the first of them an orphan (INV-15).
                if self.slots[n].st != St::Waiting {
                    continue;
                }
                let wants: Vec<usize> = self.t.nodes[n]
                    .exclusive
                    .iter()
                    .chain(self.t.nodes[n].shared.iter())
                    .copied()
                    .collect();
                let pool = self.t.nodes[n].pool;
                let ahead = wants
                    .iter()
                    .find_map(|w| blocked_locks.iter().find(|(r, _)| r == w))
                    .or_else(|| pool.and_then(|p| blocked_pools.iter().find(|(q, _)| *q == p)))
                    .map(|(_, waiter)| *waiter);
                self.slots[n].blocked_by = ahead;
                if ahead.is_none() && self.grants_available(n) {
                    self.take_grants(n);
                    self.start(n);
                    started = true;
                } else {
                    blocked_locks.extend(wants.iter().map(|&w| (w, n)));
                    if let Some(p) = pool {
                        blocked_pools.push((p, n));
                    }
                }
            }
            if !started {
                return;
            }
        }
    }

    pub(super) fn needs_ready(&self, n: usize) -> bool {
        self.t.nodes[n].needs.iter().all(|&d| {
            (self.t.nodes[d].kind == Kind::Service && self.slots[d].initialized)
                || matches!(self.slots[d].st, St::Ready | St::Finished)
        })
    }

    fn grants_available(&self, n: usize) -> bool {
        let node = &self.t.nodes[n];
        let locks_ok = node
            .exclusive
            .iter()
            .all(|&r| self.locks[r].exclusive.is_none() && self.locks[r].shared.is_empty())
            && node
                .shared
                .iter()
                .all(|&r| self.locks[r].exclusive.is_none());
        let pool_ok = match node.pool {
            Some(p) => {
                let limit = self.t.scopes[node.scope].pools[p].limit;
                self.scopes[node.scope].pools[p] < limit
            }
            None => true,
        };
        locks_ok && pool_ok
    }

    fn take_grants(&mut self, n: usize) {
        for r in self.t.nodes[n].exclusive.clone() {
            self.locks[r].exclusive = Some(n);
        }
        for r in self.t.nodes[n].shared.clone() {
            self.locks[r].shared.push(n);
        }
        if let Some(p) = self.t.nodes[n].pool {
            self.scopes[self.t.nodes[n].scope].pools[p] += 1;
        }
        self.slots[n].granted = true;
    }

    pub(super) fn release_grants(&mut self, n: usize) {
        if !self.slots[n].granted {
            return;
        }
        self.slots[n].granted = false;
        for r in self.t.nodes[n].exclusive.clone() {
            if self.locks[r].exclusive == Some(n) {
                self.locks[r].exclusive = None;
            }
        }
        for r in self.t.nodes[n].shared.clone() {
            self.locks[r].shared.retain(|&h| h != n);
        }
        if let Some(p) = self.t.nodes[n].pool {
            let scope = self.t.nodes[n].scope;
            self.scopes[scope].pools[p] = self.scopes[scope].pools[p].saturating_sub(1);
        }
    }

    /// Every scope that can admit, to a fixpoint.
    ///
    /// A lock or a pool belongs to a scope, but an **imported** resource does
    /// not: `Table::flatten` resolves an import to the parent's node, so a
    /// child node's `exclusive`/`shared` names the same lock a parent node
    /// names. Re-admitting only the releasing node's own scope left a child
    /// waiting for a lock nobody held — for ever, and with an empty `why`,
    /// because the holder it would have named was gone.
    pub(super) fn admit_all(&mut self) {
        loop {
            let before = self.starts;
            for s in 0..self.scopes.len() {
                if matches!(self.scopes[s].st, RunState::Admitting | RunState::Steady) {
                    self.admit(s);
                    self.check_steady(s);
                }
            }
            if self.starts == before {
                return;
            }
        }
    }

    /// Spawn the next attempt of a need-ready, granted node.
    fn start(&mut self, n: usize) {
        self.starts += 1;
        let kind = self.t.nodes[n].kind;
        let slot = &mut self.slots[n];
        slot.queued = None;
        let recovery = kind == Kind::Service && slot.initialized;
        if !recovery {
            slot.attempt += 1;
        }
        slot.held = false;
        slot.cancelling = false;
        slot.signalled = false;
        slot.timing_out = false;
        slot.timed_out = false;
        slot.started = true;
        slot.st = St::Running;
        let attempt = slot.attempt;
        let key = self.t.nodes[n].key;
        if recovery {
            let episode = slot.restarts + 1;
            slot.st = St::Ready;
            slot.episode = episode;
            slot.faults.clear();
            self.emit(n, TraceKind::Start(Phase::Serve));
            self.fx.push(Effect::Serve { node: key, episode });
            return;
        }
        match kind {
            Kind::Join => {
                self.release_grants(n);
                self.request_publication(n);
            }
            // A template has no body and never becomes `Ready` (contract § 1):
            // it is live, admitting instances, from the moment its imports are
            // ready until its obligation opens. No grant is taken and nothing
            // is observed — an instance's `InstanceSpawned` on this node is
            // what the trace records instead.
            Kind::Template => {
                self.release_grants(n);
                self.slots[n].st = St::Live;
            }
            Kind::Component => {
                self.emit(n, TraceKind::Start(Phase::Prepare));
                let inner = self.t.nodes[n].inner.expect("a component has a scope");
                self.scopes[inner].st = RunState::Admitting;
                self.admit(inner);
                self.check_steady(inner);
            }
            Kind::BlockingStep => {
                self.emit(n, TraceKind::Start(Phase::Run));
                self.fx.push(Effect::SpawnBlocking { node: key, attempt });
                self.arm_within(n);
            }
            _ => {
                let phase = match kind {
                    Kind::Resource | Kind::Effect => Phase::Prepare,
                    _ => Phase::Run,
                };
                self.emit(n, TraceKind::Start(phase));
                self.fx.push(Effect::Spawn { node: key, attempt });
                self.arm_within(n);
            }
        }
    }

    fn arm_within(&mut self, n: usize) {
        if let Some(d) = self.t.nodes[n].attrs.within {
            let id = self.timer(Purpose::Within(n), self.now + d);
            self.slots[n].timer = Some(id);
        }
    }

    /// Whether the scope has anything left to admit or settle.
    ///
    /// A `Live` template is settled: it has no body to wait for, and an
    /// instance of it is a scope of its own whose readiness a body awaits
    /// through `Child::ready` (INV-17), never the scope's steady state.
    fn unsettled(&self, scope: usize) -> bool {
        self.t.scopes[scope].nodes.iter().any(|&n| {
            let recovering = self.t.nodes[n].kind == Kind::Service && self.slots[n].initialized;
            matches!(self.slots[n].st, St::Pending | St::Waiting | St::Backoff) && !recovering
                || matches!(
                    self.slots[n].st,
                    St::Running | St::Publishing | St::RetryRelease
                )
        })
    }

    /// `Admitting → Steady` when every node has settled. A finite root then
    /// settles; a component becomes Ready in its parent.
    pub(super) fn check_steady(&mut self, scope: usize) {
        if self.scopes[scope].st != RunState::Admitting || self.unsettled(scope) {
            return;
        }
        self.scopes[scope].st = RunState::Steady;
        // A child plan's `Mode` is a declaration only: an inner scope — a
        // component's or an instance's — stays `Steady` whatever its mode
        // until something opens it (contract § 2).
        if self.t.scopes[scope].instance.is_some() {
            return;
        }
        match self.t.scopes[scope].component {
            None => {
                if self.t.scopes[scope].mode == crate::policy::Mode::Finite {
                    self.settle(scope, Cause::Finite);
                }
            }
            Some(c) => {
                if self.slots[c].st == St::Running {
                    self.request_publication(c);
                }
            }
        }
    }

    /// T4 under `Isolate`: every transitive dependent that has not started
    /// is `Skipped{because}`.
    pub(super) fn skip_dependents(&mut self, n: usize, because: usize) {
        let mut stack = vec![n];
        while let Some(x) = stack.pop() {
            for d in self.t.nodes[x].dependents.clone() {
                self.keep_template_live(d);
                if matches!(self.slots[d].st, St::Pending | St::Waiting) {
                    self.slots[d].st = St::Skipped;
                    self.slots[d].queued = None;
                    self.slots[d].because = Some(self.t.nodes[because].key);
                    let path = self.t.nodes[because].path.clone();
                    self.emit(
                        d,
                        TraceKind::Skipped {
                            because: Some(path),
                        },
                    );
                    // A node waiting for its *next* attempt still owns the
                    // faults of the attempts that already failed (INV-9).
                    self.flush_faults(d);
                    self.settle_or_skip_inner(d, Some(self.t.nodes[because].key));
                    stack.push(d);
                }
            }
        }
    }

    /// After a node settles: dependents may start, the scope may be steady,
    /// a settling scope may reach cleanup.
    pub(super) fn after_settle(&mut self, n: usize) {
        let scope = self.t.nodes[n].scope;
        self.admit(scope);
        self.check_steady(scope);
        // The local admit above already reached a fixpoint. Only a second
        // scope can have another waiter for a grant reached through an import.
        // Preserve the global sweep in that case, including dynamic scopes.
        if self.scopes.len() > 1 {
            self.admit_all();
        }
        self.try_cleanup(scope);
    }
}

#[cfg(test)]
#[path = "admit_trace_tests.rs"]
mod trace_tests;
