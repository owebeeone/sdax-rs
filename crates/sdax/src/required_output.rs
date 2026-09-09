//! Required completed output with the original report retained on error.
use crate::Report;
use std::sync::Arc;

/// Why a run could not supply required completed output.
/// Both variants retain the original report, including typed causes and trace.
pub enum RequiredOutputError<Out = ()> {
    /// The run was unclean, whether or not it produced output.
    Failed(Report<Out>),
    /// The run was clean but produced no completed output.
    MissingOutput(Report<Out>),
}

impl<Out> RequiredOutputError<Out> {
    /// Borrow the complete original report without reducing its diagnostics to text.
    pub fn report(&self) -> &Report<Out> {
        match self {
            Self::Failed(report) | Self::MissingOutput(report) => report,
        }
    }

    /// Recover the complete original report and its original error objects.
    pub fn into_report(self) -> Report<Out> {
        match self {
            Self::Failed(report) | Self::MissingOutput(report) => report,
        }
    }
}

impl<Out> std::fmt::Display for RequiredOutputError<Out> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        if matches!(self, Self::MissingOutput(_)) {
            writeln!(f, "missing completed output")?;
        }
        std::fmt::Display::fmt(self.report(), f)
    }
}

impl<Out> std::fmt::Debug for RequiredOutputError<Out> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = match self {
            Self::Failed(_) => "Failed",
            Self::MissingOutput(_) => "MissingOutput",
        };
        f.debug_tuple(name)
            .field(&format_args!("{}", self.report()))
            .finish()
    }
}

impl<Out> std::error::Error for RequiredOutputError<Out> {}

impl<Out> Report<Out> {
    /// Return required completed output, or a typed error retaining the full report.
    /// Failed runs take precedence over missing output. Clean runs without output
    /// return `MissingOutput`; no default output is fabricated. Use
    /// [`into_result`](Self::into_result) when missing output is acceptable.
    #[allow(clippy::result_large_err)]
    pub fn into_required_output(mut self) -> Result<Arc<Out>, RequiredOutputError<Out>> {
        if !self.is_clean() {
            return Err(RequiredOutputError::Failed(self));
        }
        match self.output.take() {
            Some(output) => Ok(output),
            None => Err(RequiredOutputError::MissingOutput(self)),
        }
    }
}
