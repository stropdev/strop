//! Native mutation requires the Linux/macOS no-follow/no-replace implementation.
use strop_core::worker::CancelToken;
use strop_workspace::operation::*;

pub fn prepare(
    _: &OperationIntent,
    _: bool,
    _: &crate::Environment,
    _: &CancelToken,
) -> Result<PreparedOperation, FsFailure> {
    Err(FsFailure::new(
        FsFailureKind::Unsupported,
        "native filesystem mutation is supported on Linux and macOS",
    ))
}
pub fn execute(
    _: &PreparedOperation,
    _: Option<&ropey::Rope>,
    _: &[StepReceipt],
    _: &CancelToken,
) -> StepOutcome {
    StepOutcome::Refused(FsFailure::new(
        FsFailureKind::Unsupported,
        "native filesystem mutation is unsupported on this platform",
    ))
}
pub fn verify(_: &StepReceipt, _: &CancelToken) -> Result<VerifiedOutcome, FsFailure> {
    Err(FsFailure::new(
        FsFailureKind::Unsupported,
        "native operation verification is unsupported on this platform",
    ))
}
