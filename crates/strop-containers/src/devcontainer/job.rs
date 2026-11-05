use super::model::{EnvironmentContext, OwnershipReceipt, PartialResource, Reconciliation};
use super::process::{
    process_error, CliProcess, LogStream, ProcessFailure, ProcessLog, ProcessOutput,
    ProcessRequest, StreamRedactor,
};
use super::DevcontainerError;
use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};
use strop_core::worker::{CancelHandle, CancelReason, CancelToken};

const MAX_RETAINED_LOG_BYTES: usize = 512 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub struct JobId(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum JobState {
    Created,
    Running,
    CancellationRequested,
    Succeeded,
    Failed,
    Cancelled,
}
impl JobState {
    pub fn is_terminal(self) -> bool {
        matches!(self, Self::Succeeded | Self::Failed | Self::Cancelled)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CliOperation {
    Attach,
    Start,
    Rebuild,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JobLog {
    pub stream: LogStream,
    /// Redacted, lossy UTF-8 text. Raw process bytes never enter snapshots.
    pub text: String,
}

#[derive(Debug, Clone)]
pub struct JobSnapshot {
    pub id: JobId,
    pub operation: CliOperation,
    pub state: JobState,
    pub logs: Vec<JobLog>,
    pub dropped_log_bytes: usize,
}

struct JobRecord {
    id: JobId,
    operation: CliOperation,
    state: JobState,
    logs: Vec<JobLog>,
    retained_bytes: usize,
    dropped_log_bytes: usize,
}

struct SharedJob {
    record: Mutex<JobRecord>,
    token: CancelToken,
    cancellation: Mutex<Option<CancelHandle>>,
}
impl SharedJob {
    fn lock(&self) -> MutexGuard<'_, JobRecord> {
        self.record
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }
    fn append(&self, stream: LogStream, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }
        let mut record = self.lock();
        let available = MAX_RETAINED_LOG_BYTES.saturating_sub(record.retained_bytes);
        let retained = available.min(bytes.len());
        if retained > 0 {
            let text = String::from_utf8_lossy(&bytes[..retained]);
            record.retained_bytes += retained;
            if let Some(last) = record.logs.last_mut().filter(|last| last.stream == stream) {
                last.text.push_str(&text);
            } else {
                record.logs.push(JobLog {
                    stream,
                    text: text.into_owned(),
                });
            }
        }
        record.dropped_log_bytes += bytes.len() - retained;
    }
}

/// Cloneable control/observe handle. Cancellation is a request, not a claim
/// that owned work has settled; the state becomes Cancelled only when run returns.
#[derive(Clone)]
pub struct JobControl {
    shared: Arc<SharedJob>,
}
impl fmt::Debug for JobControl {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("JobControl")
            .field("snapshot", &self.snapshot())
            .finish()
    }
}
impl JobControl {
    pub fn snapshot(&self) -> JobSnapshot {
        let record = self.shared.lock();
        JobSnapshot {
            id: record.id,
            operation: record.operation,
            state: record.state,
            logs: record.logs.clone(),
            dropped_log_bytes: record.dropped_log_bytes,
        }
    }
    pub fn cancel(&self) {
        {
            let mut record = self.shared.lock();
            if record.state.is_terminal() || record.state == JobState::CancellationRequested {
                return;
            }
            record.state = JobState::CancellationRequested;
        }
        let handle = self
            .shared
            .cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
        if let Some(handle) = handle {
            handle.cancel(CancelReason::Dismissed);
        }
    }
}

pub(crate) struct OperationSuccess {
    pub context: EnvironmentContext,
    pub receipt: Option<OwnershipReceipt>,
    pub reconciliation: Reconciliation,
}

pub(crate) struct OperationFailure {
    pub error: Box<DevcontainerError>,
    pub reconciliation: Reconciliation,
    pub partial_resource: Option<PartialResource>,
}

#[derive(Debug)]
pub struct JobOutcome {
    pub id: JobId,
    pub operation: CliOperation,
    pub state: JobState,
    pub context: Option<EnvironmentContext>,
    pub receipt: Option<OwnershipReceipt>,
    /// A CLI-reported possible resource after failure/cancellation. Its
    /// presence is evidence to reconcile, not permission to stop or remove it.
    pub partial_resource: Option<PartialResource>,
    pub reconciliation: Reconciliation,
    pub error: Option<DevcontainerError>,
}
impl JobOutcome {
    pub fn is_ready(&self) -> bool {
        self.state == JobState::Succeeded && self.context.is_some() && self.error.is_none()
    }
}

type Completion =
    dyn FnOnce(&ProcessOutput, JobId) -> Result<OperationSuccess, OperationFailure> + Send;

/// Explicitly owned invocation. `run` is synchronous so an engine worker can
/// own it; another thread may retain `JobControl` for logs and cancellation.
/// Dropping an unrun job cancels it. There is no implicit stop/remove on Drop.
#[must_use]
pub struct OwnedJob {
    shared: Arc<SharedJob>,
    runner: Arc<dyn CliProcess>,
    request: ProcessRequest,
    secrets: Vec<String>,
    process_failure_reconciliation: Reconciliation,
    completion: Option<Box<Completion>>,
}
impl fmt::Debug for OwnedJob {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("OwnedJob")
            .field("request", &self.request)
            .finish_non_exhaustive()
    }
}
impl OwnedJob {
    pub(crate) fn new(
        id: JobId,
        operation: CliOperation,
        runner: Arc<dyn CliProcess>,
        request: ProcessRequest,
        secrets: Vec<String>,
        process_failure_reconciliation: Reconciliation,
        completion: Box<Completion>,
    ) -> Self {
        let (token, cancellation) = CancelToken::standalone();
        Self {
            shared: Arc::new(SharedJob {
                record: Mutex::new(JobRecord {
                    id,
                    operation,
                    state: JobState::Created,
                    logs: Vec::new(),
                    retained_bytes: 0,
                    dropped_log_bytes: 0,
                }),
                token,
                cancellation: Mutex::new(Some(cancellation)),
            }),
            runner,
            request,
            secrets,
            process_failure_reconciliation,
            completion: Some(completion),
        }
    }

    pub fn control(&self) -> JobControl {
        JobControl {
            shared: self.shared.clone(),
        }
    }

    pub fn run(mut self) -> JobOutcome {
        let id = self.shared.lock().id;
        let operation = self.shared.lock().operation;
        if !self.shared.token.is_cancelled() {
            self.shared.lock().state = JobState::Running;
        }
        let mut stdout_redactor = StreamRedactor::new(self.secrets.clone());
        let mut stderr_redactor = StreamRedactor::new(self.secrets.clone());
        let shared = self.shared.clone();
        let result = self.runner.run(
            &self.request,
            &self.shared.token,
            &mut |event: ProcessLog| {
                let redactor = match event.stream {
                    LogStream::Stdout => &mut stdout_redactor,
                    LogStream::Stderr => &mut stderr_redactor,
                };
                let bytes = redactor.push(&event.bytes);
                shared.append(event.stream, &bytes);
            },
        );
        self.shared
            .append(LogStream::Stdout, &stdout_redactor.finish());
        self.shared
            .append(LogStream::Stderr, &stderr_redactor.finish());
        let final_redactor = StreamRedactor::new(self.secrets.clone());
        let outcome = match result {
            Err(failure) => {
                let cancelled = self.shared.token.is_cancelled()
                    || matches!(&failure, ProcessFailure::Cancelled);
                let error = process_error(&self.request, failure, &final_redactor);
                self.finish_error(
                    cancelled,
                    error,
                    self.process_failure_reconciliation.clone(),
                    None,
                )
            }
            Ok(output) if self.shared.token.is_cancelled() => {
                let Some(completion) = self.completion.take() else {
                    panic!("job completion runs once");
                };
                let partial_resource = match completion(&output, id) {
                    Ok(success) => success.receipt.map(|receipt| PartialResource {
                        container_id: receipt.container_id().to_owned(),
                        cli_reported_stopped: None,
                        coverage: receipt.coverage(),
                        ownership_label: Some(receipt.label().to_owned()),
                    }),
                    Err(failure) => failure.partial_resource,
                };
                self.finish_error(
                    true,
                    DevcontainerError::Cancelled,
                    self.process_failure_reconciliation.clone(),
                    partial_resource,
                )
            }
            Ok(output) => {
                let Some(completion) = self.completion.take() else {
                    panic!("job completion runs once");
                };
                match completion(&output, id) {
                    Ok(success) => {
                        self.shared.lock().state = JobState::Succeeded;
                        JobOutcome {
                            id,
                            operation,
                            state: JobState::Succeeded,
                            context: Some(success.context),
                            receipt: success.receipt,
                            partial_resource: None,
                            reconciliation: success.reconciliation,
                            error: None,
                        }
                    }
                    Err(failure) => self.finish_error(
                        false,
                        *failure.error,
                        failure.reconciliation,
                        failure.partial_resource,
                    ),
                }
            }
        };
        outcome
    }

    fn finish_error(
        &self,
        cancelled: bool,
        error: DevcontainerError,
        reconciliation: Reconciliation,
        partial_resource: Option<PartialResource>,
    ) -> JobOutcome {
        let (id, operation, state) = {
            let mut record = self.shared.lock();
            record.state = if cancelled {
                JobState::Cancelled
            } else {
                JobState::Failed
            };
            (record.id, record.operation, record.state)
        };
        JobOutcome {
            id,
            operation,
            state,
            context: None,
            receipt: None,
            partial_resource,
            reconciliation,
            error: Some(error),
        }
    }
}
impl Drop for OwnedJob {
    fn drop(&mut self) {
        let state = self.shared.lock().state;
        if !state.is_terminal() {
            self.control().cancel();
            self.shared.lock().state = JobState::Cancelled;
        }
        self.shared
            .cancellation
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .take();
    }
}
