//! Monotonic operation evidence; no new authorization, accounting or replay rights.
use anyhow::Result;
/// Monotonic evidence set before durable signed intent or a static paid send.
/// It is deliberately independent of response decoding and error text.
#[derive(Clone, Default)]
pub(crate) struct SubmissionProgress(std::sync::Arc<std::sync::atomic::AtomicBool>);
impl SubmissionProgress {
    pub(crate) fn mark_possible(&self) {
        self.0.store(true, std::sync::atomic::Ordering::Release);
    }
    pub(crate) fn possible(&self) -> bool {
        self.0.load(std::sync::atomic::Ordering::Acquire)
    }
}

/// Apply the acquisition's existing fee fact through final publication/output.
pub(crate) fn with_payment_context<T>(possible: bool, result: Result<T>) -> Result<T> {
    result.map_err(|error| {
        if possible && !error.is::<PossiblePayment>() {
            error.context(PossiblePayment)
        } else {
            error
        }
    })
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct PossiblePayment;
impl std::fmt::Display for PossiblePayment {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("A payment may already have settled; an authorization may still charge or execute; do not automatically repeat this logical call")
    }
}
impl std::error::Error for PossiblePayment {}
