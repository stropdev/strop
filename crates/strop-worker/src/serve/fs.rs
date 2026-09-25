//! Filesystem request handlers (0058 WK04): observe/list/read and
//! prepare/apply/verify dispatched to the in-process strop-fs kernel
//! under this session's one admitted [`ExecutionContext`]. Kernel
//! failures cross as the domain `FsFailure` taxonomy
//! (`ResultOutcome::Failed`), never blurred into admission refusals.

use std::collections::HashMap;
use std::io::{self, Read, Write};

use std::sync::atomic::Ordering;
use std::sync::Arc;
use strop_core::worker::CancelToken;
use strop_fs::batch::PreparedBatch;
use strop_fs::ExecutionContext;

use strop_worker_protocol::codec::StreamChunk;
use strop_worker_protocol::{Refusal, RequestId, ResultOutcome, WorkerMessage};
use strop_workspace::operation::{FsFailure, FsFailureKind, LocatedObservation, StepReceipt};
use strop_workspace::ResourceLocation;

use super::{failure, io_failure, Inbound, SessionState};

/// Read payloads stream in 64 KiB chunks (the wire ceiling is 256 KiB).
const READ_CHUNK: usize = 64 * 1024;
/// Accumulated upload bytes are bounded before their digest is checked;
/// an apply's declared length is verified against exactly what arrived.
pub(super) const MAX_CONTENT_BYTES: usize = 256 * 1024 * 1024;

pub(super) fn observe<W: Write>(
    shared: &Arc<SessionState<W>>,
    token: &CancelToken,
    locations: &[ResourceLocation],
) -> ResultOutcome {
    let mut observations = Vec::with_capacity(locations.len());
    for location in locations {
        match observe_one(&shared.context, token, location) {
            Ok(observation) => observations.push(observation),
            Err(failure) => return ResultOutcome::Failed { failure },
        }
    }
    ResultOutcome::Observations { observations }
}

/// The observation the open/list flows need (0058 WK04): lstat through
/// the kernel, with a valid symlink reported as its target's observation
/// (canonicalized, still kernel-observed) and a broken link reported as
/// the link itself so the client keeps its typed "target unavailable"
/// outcome instead of a new-file fallback.
fn observe_one(
    context: &ExecutionContext,
    token: &CancelToken,
    location: &ResourceLocation,
) -> Result<LocatedObservation, FsFailure> {
    context.admit(location)?;
    let value = strop_fs::observe(&location.path, false, token)?;
    let value = match value {
        Some(observation) if observation.kind == strop_workspace::EntryKind::SymbolicLink => {
            match std::fs::canonicalize(&location.path) {
                Ok(target) => strop_fs::observe(&target, false, token)?.or(Some(observation)),
                Err(_) => Some(observation),
            }
        }
        other => other,
    };
    Ok(LocatedObservation {
        location: location.clone(),
        value,
    })
}

pub(super) fn list<W: Write>(
    shared: &Arc<SessionState<W>>,
    token: &CancelToken,
    location: ResourceLocation,
    cursor: Option<String>,
) -> ResultOutcome {
    if cursor.is_some() {
        return ResultOutcome::Refused {
            refusal: Refusal::UnknownHandle {
                message: "listings are complete bounded snapshots; no cursor is ever issued".into(),
            },
        };
    }
    match strop_fs::list(&shared.context, &location, token) {
        Ok(listed) => ResultOutcome::Listing {
            snapshot: listed.snapshot,
            cursor: None,
        },
        Err(failure) => ResultOutcome::Failed { failure },
    }
}

/// A read streams: the `ReadOpened` reply first, then bounded chunks. A
/// cancelled or failed read ends the stream early WITHOUT announcing
/// completion — the client's payload errors on the short body, never
/// silently truncating (the wire has no stream-error frame; the announced
/// `size` is the integrity anchor).
pub(super) fn read_streaming<W: Write + Send + 'static>(
    shared: &Arc<SessionState<W>>,
    id: RequestId,
    token: &CancelToken,
    location: ResourceLocation,
    offset: u64,
    length: Option<u64>,
) {
    let reply = |outcome| shared.send(&WorkerMessage::Result { id, outcome });
    use std::io::Seek as _;
    let file = (|| -> Result<std::fs::File, FsFailure> {
        shared.context.admit(&location)?;
        if !location.path.is_absolute() {
            return Err(failure(
                FsFailureKind::InvalidPath,
                "read scope must be absolute",
            ));
        }
        let file = std::fs::File::open(&location.path).map_err(io_failure)?;
        let metadata = file.metadata().map_err(io_failure)?;
        if metadata.is_dir() {
            return Err(failure(
                FsFailureKind::Unsupported,
                "a directory is listed, never read",
            ));
        }
        Ok(file)
    })();
    let mut file = match file {
        Ok(file) => file,
        Err(failure) => return reply(ResultOutcome::Failed { failure }),
    };
    // Seek before announcing: the announced size is exactly what the
    // stream will deliver — a ranged read announces the range (clamped
    // to what remains), never the whole file, so the payload's
    // short-stream detection means "torn read", never "range shorter
    // than the file".
    if let Err(error) = file.seek(std::io::SeekFrom::Start(offset)) {
        return reply(ResultOutcome::Failed {
            failure: io_failure(error),
        });
    }
    let size = file.metadata().ok().map(|metadata| metadata.len());
    let announced = size.map(|total| {
        let available = total.saturating_sub(offset);
        length.map_or(available, |left| left.min(available))
    });
    let stream = shared.mint_stream();
    reply(ResultOutcome::ReadOpened {
        stream,
        size: announced,
    });
    let mut buffer = vec![0_u8; READ_CHUNK];
    let mut sequence = 0_u64;
    let mut remaining = length;
    loop {
        let want = remaining.map_or(READ_CHUNK, |left| left.min(READ_CHUNK as u64) as usize);
        if token.is_cancelled() || want == 0 || shared.stop.load(Ordering::Acquire) {
            shared.send_chunk(&StreamChunk {
                stream,
                sequence,
                last: true,
                bytes: Vec::new(),
            });
            return;
        }
        match file.read(&mut buffer[..want]) {
            Ok(0) => {
                shared.send_chunk(&StreamChunk {
                    stream,
                    sequence,
                    last: true,
                    bytes: Vec::new(),
                });
                return;
            }
            Ok(count) => {
                let last = remaining == Some(count as u64);
                shared.send_chunk(&StreamChunk {
                    stream,
                    sequence,
                    last,
                    bytes: buffer[..count].to_vec(),
                });
                sequence += 1;
                if let Some(left) = remaining.as_mut() {
                    *left -= count as u64;
                }
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => {
                shared.note(format_args!("read: {error}"));
                shared.send_chunk(&StreamChunk {
                    stream,
                    sequence,
                    last: true,
                    bytes: Vec::new(),
                });
                return;
            }
        }
    }
}

pub(super) fn prepare<W: Write>(
    shared: &Arc<SessionState<W>>,
    token: &CancelToken,
    intents: &[strop_workspace::operation::OperationIntent],
    environment: Option<strop_worker_protocol::request::EnvironmentOverride>,
) -> ResultOutcome {
    // A client's admitted environment override wins (per-session trash
    // roots); absent fields keep this worker's captured context.
    let environment = environment.map_or_else(
        || shared.environment.clone(),
        |override_| strop_fs::Environment {
            home: override_.home.or(shared.environment.home.clone()),
            data_home: override_.data_home.or(shared.environment.data_home.clone()),
        },
    );
    match strop_fs::batch::prepare(&shared.context, intents, &environment, token) {
        Ok(batch) => ResultOutcome::Prepared {
            steps: batch.steps,
            refused: batch.refused,
        },
        Err(failure) => ResultOutcome::Failed { failure },
    }
}

pub(super) fn apply<W: Write>(
    shared: &Arc<SessionState<W>>,
    token: &CancelToken,
    steps: Vec<strop_workspace::operation::PreparedOperation>,
    content: Option<strop_worker_protocol::StreamRef>,
    binding: Option<strop_worker_protocol::DocumentStamp>,
) -> ResultOutcome {
    let mut contents = HashMap::new();
    if let Some(reference) = content {
        let bytes = match take_content(shared, reference) {
            Ok(bytes) => bytes,
            Err(outcome) => return *outcome,
        };
        let Ok(text) = std::str::from_utf8(&bytes) else {
            return ResultOutcome::Failed {
                failure: failure(
                    FsFailureKind::Unsupported,
                    "frozen content is not UTF-8 text",
                ),
            };
        };
        let rope = ropey::Rope::from_str(text);
        for (step, operation) in steps.iter().enumerate() {
            if operation.intent.kind == strop_workspace::operation::OperationKind::Copy
                && operation.intent.copy_version == strop_workspace::operation::CopyVersion::Buffer
            {
                contents.insert(step, rope.clone());
            }
        }
    }
    let plan = PreparedBatch {
        steps,
        refused: Vec::new(),
    };
    let receipts = strop_fs::batch::execute(&shared.context, &plan, &contents, token);
    ResultOutcome::Applied { receipts, binding }
}

/// Consume one completed upload stream: it must exist, be closed by its
/// `last` chunk, and match the declared length and digest exactly.
fn take_content<W: Write>(
    shared: &Arc<SessionState<W>>,
    reference: strop_worker_protocol::StreamRef,
) -> Result<Vec<u8>, Box<ResultOutcome>> {
    let taken = shared.inbound.lock().remove(&reference.stream);
    let Some(Inbound::Content {
        bytes,
        closed: true,
        ..
    }) = taken
    else {
        return Err(Box::new(ResultOutcome::Refused {
            refusal: Refusal::UnknownHandle {
                message: format!("content stream {} is not complete", reference.stream.0),
            },
        }));
    };
    use sha2::Digest;
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    if bytes.len() as u64 != reference.bytes || digest != reference.digest {
        return Err(Box::new(ResultOutcome::Failed {
            failure: failure(
                FsFailureKind::Protocol,
                "content stream does not match its declared length and digest",
            ),
        }));
    }
    Ok(bytes)
}

pub(super) fn verify<W: Write>(
    shared: &Arc<SessionState<W>>,
    token: &CancelToken,
    attempt: &StepReceipt,
    binding: Option<strop_worker_protocol::DocumentStamp>,
) -> ResultOutcome {
    match strop_fs::batch::verify(&shared.context, attempt, token) {
        Ok(verified) => ResultOutcome::Verified { verified, binding },
        Err(failure) => ResultOutcome::Failed { failure },
    }
}
