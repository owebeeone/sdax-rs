//! The catalogue of declaration mistakes the generator makes on purpose, so
//! the validator is under test on the same walk as the machine. Each one
//! names the [`Rule`] the build must be refused by.

use sdax::Rule;

/// Which declaration mistake to make on purpose.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mutation {
    None,
    DupName,
    LockNeeds,
    IdempotentRequired,
    BudgetOrder,
    Mode,
    TryUnconsumed,
    UnusedPool,
    ServiceUnbounded,
    PoolStarve,
    SpawnKind,
    SpawnSelfImport,
}

impl Mutation {
    pub(crate) fn rule(self) -> Option<Rule> {
        Some(match self {
            Mutation::None => return None,
            Mutation::DupName => Rule::DupName,
            Mutation::LockNeeds => Rule::LockNeeds,
            Mutation::IdempotentRequired => Rule::IdempotentRequired,
            Mutation::BudgetOrder => Rule::BudgetOrder,
            Mutation::Mode => Rule::Mode,
            Mutation::TryUnconsumed => Rule::TryUnconsumed,
            Mutation::UnusedPool => Rule::UnusedPool,
            Mutation::ServiceUnbounded => Rule::ServiceUnbounded,
            Mutation::PoolStarve => Rule::PoolStarve,
            Mutation::SpawnKind => Rule::SpawnKind,
            Mutation::SpawnSelfImport => Rule::SpawnSelfImport,
        })
    }

    pub(crate) fn name(self) -> &'static str {
        match self {
            Mutation::None => "none",
            Mutation::DupName => "duplicate name",
            Mutation::LockNeeds => "lock on a non-need",
            Mutation::IdempotentRequired => "retry on an effect without idempotent",
            Mutation::BudgetOrder => "stop_within over the budget",
            Mutation::Mode => "finite with a service",
            Mutation::TryUnconsumed => "unconsumed try_step",
            Mutation::UnusedPool => "unused pool",
            Mutation::ServiceUnbounded => "unbounded without stop_within",
            Mutation::PoolStarve => "pool starved by a resident holder",
            Mutation::SpawnKind => "spawns on a node that is not a service",
            Mutation::SpawnSelfImport => "a template importing the service that spawns it",
        }
    }
}
