//! The local filesystem, in process (plan 0090 §2): observation, listing,
//! reads and the checked operation kernel of `strop-fs`, called directly
//! on jobs. One execution context per editor binds every prepared plan to
//! this process; nothing here runs on input or render.
use std::collections::HashMap;
use std::sync::Arc;

use strop_core::worker::CancelToken;
use strop_fs::batch::PreparedBatch;
use strop_fs::{Environment, ExecutionContext};
use strop_workspace::operation::{
    FsFailure, FsFailureKind, LocatedObservation, OperationIntent, PreparedOperation, StepOutcome,
    StepReceipt, VerifiedOutcome,
};
use strop_workspace::{DirectorySnapshot, EntryKind, ResourceLocation};

/// The editor's one local execution context. Cloning shares it.
#[derive(Clone)]
pub(crate) struct LocalFs {
    context: Arc<ExecutionContext>,
}

impl Default for LocalFs {
    fn default() -> Self {
        Self {
            context: Arc::new(ExecutionContext::native()),
        }
    }
}

fn io_failure(error: std::io::Error) -> FsFailure {
    FsFailure::new(FsFailureKind::Io, error.to_string())
}

/// Bytes read from the file per system call while a rope is built.
const READ_BUFFER: usize = 1 << 20;

/// A reader that stops at the owner's cancellation: a large file's load
/// ends at the next read, not after the whole file.
struct Cancellable<'a, R> {
    inner: R,
    token: &'a CancelToken,
}

impl<R: std::io::Read> std::io::Read for Cancellable<'_, R> {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        if self.token.is_cancelled() {
            return Err(std::io::Error::other("cancelled"));
        }
        self.inner.read(buf)
    }
}

fn cancelled(token: &CancelToken) -> Result<(), FsFailure> {
    if token.is_cancelled() {
        return Err(FsFailure::new(FsFailureKind::Cancelled, "cancelled"));
    }
    Ok(())
}

impl LocalFs {
    /// Each location's observation. A valid symlink reports its target's
    /// observation; a broken one reports the link itself, so the opener
    /// keeps its typed "target unavailable" outcome instead of a new file.
    pub(crate) fn observe(
        &self,
        token: &CancelToken,
        locations: &[ResourceLocation],
    ) -> Result<Vec<LocatedObservation>, FsFailure> {
        let mut observations = Vec::with_capacity(locations.len());
        for location in locations {
            self.context.admit(location)?;
            let value = strop_fs::observe(&location.path, false, false, token)?;
            let value = match value {
                Some(observation) if observation.kind == EntryKind::SymbolicLink => {
                    match std::fs::canonicalize(&location.path) {
                        Ok(target) => {
                            strop_fs::observe(&target, false, false, token)?.or(Some(observation))
                        }
                        Err(_) => Some(observation),
                    }
                }
                other => other,
            };
            observations.push(LocatedObservation {
                location: location.clone(),
                value,
            });
        }
        Ok(observations)
    }

    pub(crate) fn list(
        &self,
        token: &CancelToken,
        location: &ResourceLocation,
    ) -> Result<DirectorySnapshot, FsFailure> {
        Ok(strop_fs::list(&self.context, location, token)?.snapshot)
    }

    /// The whole file as text, built straight from the file: no second
    /// copy of its bytes is held while the rope grows. Bytes that are not
    /// UTF-8 refuse the read. A directory is listed, never read.
    pub(crate) fn read_rope(
        &self,
        token: &CancelToken,
        location: &ResourceLocation,
    ) -> Result<ropey::Rope, FsFailure> {
        self.context.admit(location)?;
        cancelled(token)?;
        if !location.path.is_absolute() {
            return Err(FsFailure::new(
                FsFailureKind::InvalidPath,
                "read scope must be absolute",
            ));
        }
        let file = std::fs::File::open(&location.path).map_err(io_failure)?;
        if file.metadata().map_err(io_failure)?.is_dir() {
            return Err(FsFailure::new(
                FsFailureKind::Unsupported,
                "a directory is listed, never read",
            ));
        }
        let reader = Cancellable {
            inner: std::io::BufReader::with_capacity(READ_BUFFER, file),
            token,
        };
        let rope = ropey::Rope::from_reader(reader).map_err(|error| {
            if token.is_cancelled() {
                FsFailure::new(FsFailureKind::Cancelled, "cancelled")
            } else {
                io_failure(error)
            }
        })?;
        cancelled(token)?;
        Ok(rope)
    }

    /// Prepare one reviewed batch under `environment` (its trash roots).
    pub(crate) fn prepare(
        &self,
        token: &CancelToken,
        intents: &[OperationIntent],
        environment: &Environment,
    ) -> Result<PreparedBatch, FsFailure> {
        strop_fs::batch::prepare(&*self.context, intents, environment, token)
    }

    /// Execute one prepared batch; `contents` are the frozen buffer texts
    /// of the steps that carry one.
    pub(crate) fn execute(
        &self,
        token: &CancelToken,
        plan: &PreparedBatch,
        contents: &HashMap<usize, ropey::Rope>,
    ) -> Vec<StepReceipt> {
        strop_fs::batch::execute(&*self.context, plan, contents, token)
    }

    pub(crate) fn verify(
        &self,
        token: &CancelToken,
        receipt: &StepReceipt,
    ) -> Result<VerifiedOutcome, FsFailure> {
        strop_fs::batch::verify(&*self.context, receipt, token)
    }

    /// Prepare one document Store: the admitted step (its destination
    /// observation is the editor's evidence) or the typed refusal.
    pub(crate) fn prepare_store(
        &self,
        token: &CancelToken,
        intent: OperationIntent,
    ) -> Result<PreparedOperation, FsFailure> {
        let PreparedBatch { mut steps, refused } =
            self.prepare(token, &[intent], &Environment::capture())?;
        if let Some(refusal) = refused.into_iter().next() {
            return Err(refusal.failure);
        }
        if steps.len() != 1 {
            return Err(FsFailure::new(
                FsFailureKind::Protocol,
                "a document store prepares exactly one step",
            ));
        }
        Ok(steps.remove(0))
    }

    /// Apply one prepared Store step with its frozen text: exactly one
    /// receipt — committed, refused, cancelled, or unconfirmed with its
    /// frozen evidence.
    pub(crate) fn apply_store(
        &self,
        token: &CancelToken,
        operation: PreparedOperation,
        text: ropey::Rope,
    ) -> StepReceipt {
        let plan = PreparedBatch {
            steps: vec![operation.clone()],
            refused: Vec::new(),
        };
        let contents = HashMap::from([(0, text)]);
        match <[StepReceipt; 1]>::try_from(self.execute(token, &plan, &contents)) {
            Ok([receipt]) => receipt,
            Err(_) => StepReceipt {
                step: 0,
                operation,
                outcome: StepOutcome::Unconfirmed {
                    detail: "a store produced the wrong receipt count".into(),
                    observed_destination: None,
                    recovery: None,
                    publication: None,
                },
            },
        }
    }

    /// Verify a frozen unconfirmed Store without reviving its prepared
    /// mutation capability: observation and durable synchronization only.
    pub(crate) fn verify_recovered(
        &self,
        token: &CancelToken,
        receipt: &StepReceipt,
    ) -> Result<VerifiedOutcome, FsFailure> {
        strop_fs::batch::verify_recovered(&self.context, receipt, token)
    }
}
