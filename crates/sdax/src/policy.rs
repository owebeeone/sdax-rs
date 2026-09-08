//! Scope-level and node-level policy. None of it is defaulted: policy,
//! shutdown budget, run mode and an effect's ambiguity handling are necessary
//! intent and are required arguments or required typestate steps.

use std::time::Duration;

/// What the scope does about the first fault.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Policy {
    /// Stop admitting starts, cancel in-flight non-service nodes, settle.
    FailFast,
    /// Skip the fault's transitive dependents and let the rest of the run
    /// proceed; every fault is aggregated.
    Isolate,
}

/// How long the run has, from entering `Settling` to `Ended`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Shutdown {
    within: Option<Duration>,
}

impl Shutdown {
    /// Bound the whole settle-and-cleanup phase by `d` on the engine clock
    /// (INV-8).
    pub fn within(d: Duration) -> Self {
        Shutdown { within: Some(d) }
    }

    /// No bound. Every service, including those in components and templates,
    /// must then declare `stop_within`
    /// ([`Rule::ServiceUnbounded`](crate::Rule::ServiceUnbounded)).
    pub fn unbounded() -> Self {
        Shutdown { within: None }
    }

    /// The budget, or `None` when unbounded.
    pub fn budget(&self) -> Option<Duration> {
        self.within
    }
}

impl std::fmt::Display for Shutdown {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self.within {
            Some(d) => write!(f, "within {}", crate::view::human(d)),
            None => f.write_str("unbounded"),
        }
    }
}

/// How the run ends by itself (F3, `AdversarialReview.md` F-B5).
///
/// Required at `build`. There is no structure-derived default: a plan of
/// resources that was meant to stay up used to end silently at `Steady`, and
/// that is exactly the kind of default this design rejects.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Mode {
    /// The run ends when every node has settled: `Steady` leads straight to
    /// `Cleanup`. Rejected for a plan with a service or a template
    /// ([`Rule::Mode`](crate::Rule::Mode)) — a finite run would stop them the
    /// moment they became ready.
    Finite,
    /// The run stays at `Steady` until `shutdown()`, `cancel()`, a fault under
    /// `FailFast`, or a `terminal` service finishing. Allowed with no services.
    Resident,
}

impl std::fmt::Display for Mode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Mode::Finite => "finite",
            Mode::Resident => "resident",
        })
    }
}

/// What to do about an effect whose outcome is unknown: it started, and it was
/// interrupted or timed out before `hold` registered a receipt.
///
/// Required on every effect (typestate): this is the most expensive footgun in
/// the domain, so it has no default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Ambiguity {
    /// List it under `ambiguous` and do nothing else.
    Report,
    /// Reconcile using the typed identity and explicit recovery handler. Requires `.idempotent()`.
    Recover,
    /// Perform it again. Requires `.idempotent()`.
    Retry,
}

impl std::fmt::Display for Ambiguity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Ambiguity::Report => "report",
            Ambiguity::Recover => "recover",
            Ambiguity::Retry => "retry",
        })
    }
}

/// Backoff between attempts, measured on the injected clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Backoff {
    /// The same wait every time.
    Fixed(Duration),
    /// `initial`, then multiplied by `factor` each attempt, capped at `cap`.
    Exponential {
        /// Wait before the second attempt.
        initial: Duration,
        /// Multiplier applied per attempt.
        factor: u32,
        /// Upper bound on the wait.
        cap: Duration,
    },
}

impl Backoff {
    /// A fixed wait.
    pub fn fixed(d: Duration) -> Self {
        Backoff::Fixed(d)
    }
    /// An exponential wait.
    pub fn exponential(initial: Duration, factor: u32, cap: Duration) -> Self {
        Backoff::Exponential {
            initial,
            factor,
            cap,
        }
    }
}

/// Re-execution of a prepare body.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Retry {
    attempts: u32,
    backoff: Option<Backoff>,
}

impl Retry {
    /// At most `n` attempts in total (not `n` retries after the first).
    pub fn attempts(n: u32) -> Self {
        Retry {
            attempts: n,
            backoff: None,
        }
    }
    /// Wait between attempts.
    pub fn backoff(mut self, b: Backoff) -> Self {
        self.backoff = Some(b);
        self
    }
    /// Total attempts allowed.
    pub fn max_attempts(&self) -> u32 {
        self.attempts
    }
    /// The declared backoff, if any.
    pub fn backoff_policy(&self) -> Option<Backoff> {
        self.backoff
    }
}

/// Re-running a service whose serve future returned `Err`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Restart {
    backoff: Backoff,
    max: Option<u32>,
}

impl Restart {
    /// Restart after an error, waiting per `b`.
    pub fn on_error(b: Backoff) -> Self {
        Restart {
            backoff: b,
            max: None,
        }
    }
    /// Give up after `n` restarts.
    pub fn max(mut self, n: u32) -> Self {
        self.max = Some(n);
        self
    }
    /// The declared backoff.
    pub fn backoff(&self) -> Backoff {
        self.backoff
    }
    /// The restart limit, if any.
    pub fn limit(&self) -> Option<u32> {
        self.max
    }
}

/// How a node's in-flight body is cancelled.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CancelMode {
    /// Abort at the next await point. The default for resources, steps and
    /// effects; an interrupted effect that had not yet held is `Ambiguous`.
    Drop,
    /// Signal `cx.stop()`, keep polling for the grace, then abort — so an
    /// in-flight `hold` can still complete and register.
    Cooperative(Duration),
}

impl std::fmt::Display for CancelMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CancelMode::Drop => f.write_str("drop"),
            CancelMode::Cooperative(d) => write!(f, "cooperative({})", crate::view::human(*d)),
        }
    }
}
