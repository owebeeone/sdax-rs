//! `build`-time validation: everything the type system cannot catch, decided
//! before any effect.
//!
//! Every rule below is a decision procedure over the recorded declaration, and
//! each one's rustdoc states it. `build` reports **all** findings at once as
//! values, never a string; a `Finding` names the rule id, the node(s), the
//! key(s), what was found and what to do about it.
//!
//! Findings are emitted rule by rule in the order of [`Rule::ALL`], and within
//! a rule in node declaration order, so two builds of one program produce the
//! same list.

mod budgets;
mod rules;

use crate::key::RawKey;
use crate::plan::PlanIr;

/// The validate-stage rules, in the order findings are emitted.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Rule {
    /// `V-INCOMPLETE-DECLARATION`: every reserved builder chain must reach a
    /// terminal before build, even when its intermediate value is forgotten.
    IncompleteDeclaration,
    /// `V-LIVE-EXPORT`: finite plans export completed data, not a resource or service handle.
    LiveExport,
    /// `V-EMPTY`: a plan with no nodes to run.
    Empty,
    /// `V-FOREIGN-KEY`: a key or pool of another plan named in this one.
    ForeignKey,
    /// `V-DUP-NAME`: two nodes of one scope share a name.
    DupName,
    /// `V-DUP-ATTR`: an attribute set twice on one node, or one resource
    /// locked twice or in two modes.
    DupAttr,
    /// `V-IMPORT-SCOPE`: a child plan imports a key the registering plan does
    /// not own.
    ImportScope,
    /// `V-SPAWN-SELF-IMPORT`: a template a service spawns imports that
    /// service's own key — a readiness deadlock (F1).
    SpawnSelfImport,
    /// `V-SPAWN-KIND`: `spawns` attached to a node that is not a service.
    SpawnKind,
    /// `V-LOCK-NEEDS`: `exclusive`/`shared` names a resource the node does not
    /// need.
    LockNeeds,
    /// `V-IDEMPOTENT-REQUIRED`: re-execution or evidence-free compensation
    /// without `idempotent`.
    IdempotentRequired,
    /// `V-POOL-STARVE`: resident holders of a pool can starve its other users.
    PoolStarve,
    /// `V-UNUSED-POOL`: a declared pool nobody uses.
    UnusedPool,
    /// `V-SERVICE-UNBOUNDED`: `Shutdown::unbounded()` with a service that does
    /// not bound its stop, anywhere in the component/template declaration tree.
    ServiceUnbounded,
    /// `V-TRY-UNCONSUMED`: a try-step nothing depends on, so its failure would
    /// vanish.
    TryUnconsumed,
    /// `V-BUDGET-ORDER`: a stop or child budget larger than the budget that
    /// contains it.
    BudgetOrder,
    /// `V-MODE`: `Mode::Finite` on a plan that has a service or a template (F3).
    Mode,
    /// `V-BLOCKING-CANCEL`: `cooperative` on a blocking step, whose grace can
    /// never be spent — a thread is signalled and never aborted.
    BlockingCancel,
    /// `V-BLOCKING-LIMIT`: a blocking node declares both its execution pool and a general limit.
    BlockingLimit,
    /// `V-RECOVERY-MISSING`: an effect requests recovery without binding
    /// a typed operation identity and installing a recovery handler.
    RecoveryMissing,
}

impl Rule {
    /// Every rule, in emission order.
    pub const ALL: [Rule; 20] = [
        Rule::IncompleteDeclaration,
        Rule::LiveExport,
        Rule::Empty,
        Rule::ForeignKey,
        Rule::DupName,
        Rule::DupAttr,
        Rule::ImportScope,
        Rule::SpawnSelfImport,
        Rule::SpawnKind,
        Rule::LockNeeds,
        Rule::IdempotentRequired,
        Rule::PoolStarve,
        Rule::UnusedPool,
        Rule::ServiceUnbounded,
        Rule::TryUnconsumed,
        Rule::BudgetOrder,
        Rule::Mode,
        Rule::BlockingCancel,
        Rule::BlockingLimit,
        Rule::RecoveryMissing,
    ];

    /// The rule's stable identifier, as it appears in the gate inventory.
    pub fn id(&self) -> &'static str {
        match self {
            Rule::IncompleteDeclaration => "V-INCOMPLETE-DECLARATION",
            Rule::LiveExport => "V-LIVE-EXPORT",
            Rule::Empty => "V-EMPTY",
            Rule::ForeignKey => "V-FOREIGN-KEY",
            Rule::DupName => "V-DUP-NAME",
            Rule::DupAttr => "V-DUP-ATTR",
            Rule::ImportScope => "V-IMPORT-SCOPE",
            Rule::SpawnSelfImport => "V-SPAWN-SELF-IMPORT",
            Rule::SpawnKind => "V-SPAWN-KIND",
            Rule::LockNeeds => "V-LOCK-NEEDS",
            Rule::IdempotentRequired => "V-IDEMPOTENT-REQUIRED",
            Rule::PoolStarve => "V-POOL-STARVE",
            Rule::UnusedPool => "V-UNUSED-POOL",
            Rule::ServiceUnbounded => "V-SERVICE-UNBOUNDED",
            Rule::TryUnconsumed => "V-TRY-UNCONSUMED",
            Rule::BudgetOrder => "V-BUDGET-ORDER",
            Rule::Mode => "V-MODE",
            Rule::BlockingCancel => "V-BLOCKING-CANCEL",
            Rule::BlockingLimit => "V-BLOCKING-LIMIT",
            Rule::RecoveryMissing => "V-RECOVERY-MISSING",
        }
    }
}

impl std::fmt::Display for Rule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.id())
    }
}

/// One violated rule, as a value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// Which rule was violated.
    pub rule: Rule,
    /// The node names involved, in declaration order. Nested service findings
    /// use root-relative paths in depth-first declaration order.
    pub nodes: Vec<String>,
    /// The keys involved.
    pub keys: Vec<RawKey>,
    /// What was found.
    pub detail: String,
    /// What to do about it.
    pub fix: String,
}

impl std::fmt::Display for Finding {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}: {} — {}", self.rule.id(), self.detail, self.fix)
    }
}

/// Everything wrong with a plan, reported at once.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Invalid {
    /// The findings.
    pub checks: Vec<Finding>,
}

impl std::fmt::Display for Invalid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        writeln!(f, "invalid plan: {} finding(s)", self.checks.len())?;
        for c in &self.checks {
            writeln!(f, "  {c}")?;
        }
        Ok(())
    }
}

impl std::error::Error for Invalid {}

pub(crate) struct Ctx<'a> {
    pub(crate) ir: &'a PlanIr,
    pub(crate) out: Vec<Finding>,
}

impl Ctx<'_> {
    pub(crate) fn name(&self, k: RawKey) -> String {
        match self.ir.node(k) {
            Some(n) => n.name.clone(),
            None => format!("{}/{}", k.plan, k.idx),
        }
    }

    fn add(
        &mut self,
        rule: Rule,
        nodes: Vec<String>,
        keys: Vec<RawKey>,
        detail: String,
        fix: &str,
    ) {
        self.out.push(Finding {
            rule,
            nodes,
            keys,
            detail,
            fix: fix.to_string(),
        });
    }
}

/// Run every rule over a plan's declaration.
#[cfg(test)]
pub(crate) fn validate(ir: &PlanIr) -> Vec<Finding> {
    validate_build(ir, &[])
}

/// Validate reserved authoring chains as well as the completed graph.
pub(crate) fn validate_build(ir: &PlanIr, unfinished: &[RawKey]) -> Vec<Finding> {
    let mut c = Ctx {
        ir,
        out: Vec::new(),
    };
    for rule in Rule::ALL {
        match rule {
            Rule::IncompleteDeclaration => {
                for key in unfinished {
                    if let Some(node) = ir.node(*key) {
                        let fix = match node.kind {
                            crate::plan::Kind::Resource => {
                                "finish the resource with acquire and release"
                            }
                            crate::plan::Kind::Effect => {
                                "finish the effect with perform and compensate or persistent"
                            }
                            crate::plan::Kind::Service => {
                                "finish the service with initialize and serve"
                            }
                            _ => "finish the node with run",
                        };
                        c.add(
                            Rule::IncompleteDeclaration,
                            vec![node.name.clone()],
                            vec![*key],
                            format!("node {:?} has an unfinished declaration", node.name),
                            fix,
                        );
                    }
                }
            }
            Rule::LiveExport => rules::live_export(&mut c),
            Rule::Empty => rules::empty(&mut c),
            Rule::ForeignKey => rules::foreign_key(&mut c),
            Rule::DupName => rules::dup_name(&mut c),
            Rule::DupAttr => rules::dup_attr(&mut c),
            Rule::ImportScope => rules::import_scope(&mut c),
            Rule::SpawnSelfImport => rules::spawn_self_import(&mut c),
            Rule::SpawnKind => rules::spawn_kind(&mut c),
            Rule::LockNeeds => rules::lock_needs(&mut c),
            Rule::IdempotentRequired => budgets::idempotent_required(&mut c),
            Rule::PoolStarve => budgets::pool_starve(&mut c),
            Rule::UnusedPool => budgets::unused_pool(&mut c),
            Rule::ServiceUnbounded => budgets::service_unbounded(&mut c),
            Rule::TryUnconsumed => budgets::try_unconsumed(&mut c),
            Rule::BudgetOrder => budgets::budget_order(&mut c),
            Rule::Mode => budgets::mode(&mut c),
            Rule::BlockingCancel => rules::blocking_cancel(&mut c),
            Rule::BlockingLimit => rules::blocking_limit(&mut c),
            Rule::RecoveryMissing => rules::persist_ambig(&mut c),
        }
    }
    c.out
}
