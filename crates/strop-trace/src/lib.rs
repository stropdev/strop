//! One opt-in diagnostic sink for all strop crates. Producers never perform file
//! I/O or wait for the writer; an incomplete trace is always reported as such.
//! Capture is bounded (total bytes, total events, per-record bytes) and every
//! finished file ends with an explicit terminal `TraceEnd` marker, so a capped
//! or failed capture can never be mistaken for a complete one. Forensic values
//! over the per-record cap travel as ordered `replay_chunk` runs (schema 3)
//! that the reader reassembles strictly — or refuses loudly.
mod bounded;
mod chunk;
mod event;
pub mod export;
mod privacy;
pub mod replay;
pub use privacy::without_content;
mod writer;

pub use event::{
    preview, ContentPolicy, EventKind, Limits, TraceOptions, MAX_CAPTURE_BYTES, MAX_CAPTURE_EVENTS,
    MAX_RECORD_BYTES, SCHEMA_VERSION, TERMINAL_RESERVE,
};

use parking_lot::Mutex;
use serde::Serialize;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Sender};
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;
use std::time::Instant;

static ACTIVE: LazyLock<Mutex<Option<Arc<Recorder>>>> = LazyLock::new(|| Mutex::new(None));
static ENABLED: AtomicBool = AtomicBool::new(false);
static CONTENT: AtomicBool = AtomicBool::new(false);

#[derive(Debug, thiserror::Error)]
pub enum TraceError {
    #[error("a trace session is already active")]
    AlreadyActive,
    #[error("cannot create trace {path}: {source}")]
    Open {
        path: PathBuf,
        source: std::io::Error,
    },
    #[error("cannot start trace writer: {0}")]
    Spawn(std::io::Error),
    #[error("incomplete trace: {0}")]
    Incomplete(String),
}

#[derive(Default)]
struct Failure {
    message: Mutex<Option<(Severity, String)>>,
    reported: AtomicBool,
}

/// How a capture ends short of complete. `Degraded` means the session
/// continued honestly — the file's terminal marker says incomplete — so
/// `finish` reports it but does not fail the editor's exit; `Fatal` means
/// durability was lost (writer I/O or a vanished writer) and the session's
/// reporting boundary must propagate the failure.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Severity {
    Degraded,
    Fatal,
}

impl Failure {
    fn degraded(&self, message: impl FnOnce() -> String) {
        self.set(Severity::Degraded, message);
    }
    fn fatal(&self, message: impl FnOnce() -> String) {
        self.set(Severity::Fatal, message);
    }
    fn set(&self, severity: Severity, message: impl FnOnce() -> String) {
        let mut failure = self.message.lock();
        if failure.is_none() {
            *failure = Some((severity, message()));
        }
    }
}

struct Record {
    kind: EventKind,
    elapsed_us: u128,
    fields: Vec<u8>,
}

/// Lifetime budgets bound retained records even if the writer never runs.
/// Field bytes exclude envelope overhead, so exhausting either budget proves
/// the file cannot fit. Keep one overflow witness for the writer's cap marker.
struct Admission {
    sender: Option<Sender<Record>>,
    remaining_events: u64,
    remaining_fields: usize,
}

impl Admission {
    fn new(sender: Sender<Record>, limits: Limits) -> Self {
        Self {
            sender: Some(sender),
            remaining_events: limits.events,
            remaining_fields: limits.bytes - TERMINAL_RESERVE,
        }
    }

    fn send(&mut self, record: Record, failure: &Failure) -> bool {
        let Some(sender) = self.sender.as_ref() else {
            return false;
        };
        let fields = record.fields.len();
        let capped = self.remaining_events == 0 || fields > self.remaining_fields;
        if sender.send(record).is_err() {
            failure.fatal(|| "capture writer unavailable".into());
            self.sender.take();
            return false;
        }
        self.remaining_events = self.remaining_events.saturating_sub(1);
        self.remaining_fields = self.remaining_fields.saturating_sub(fields);
        if capped {
            self.sender.take();
        }
        !capped
    }
}

struct Recorder {
    admission: Mutex<Admission>,
    failure: Arc<Failure>,
    started: Instant,
    max_record: usize,
    /// Capture-owned output, resolved once outside the editor input path.
    path: Option<PathBuf>,
}

/// Owning lifetime of a trace. Explicit finish reports errors; Drop still drains.
pub struct TraceSession {
    recorder: Arc<Recorder>,
    worker: Option<JoinHandle<()>>,
}

pub fn start(path: &Path, options: TraceOptions) -> Result<TraceSession, TraceError> {
    if !options.limits.valid() {
        return Err(TraceError::Incomplete("invalid capture limits".into()));
    }
    let mut active = ACTIVE.lock();
    if active.is_some() {
        return Err(TraceError::AlreadyActive);
    }
    let mut open = OpenOptions::new();
    open.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        open.mode(0o600);
    }
    let file = open.open(path).map_err(|source| TraceError::Open {
        path: path.to_path_buf(),
        source,
    })?;
    let capture_path = path
        .canonicalize()
        .ok()
        .or_else(|| path.is_absolute().then(|| path.to_path_buf()));
    let (sender, receiver) = channel();
    let failure = Arc::new(Failure::default());
    let writer_failure = Arc::clone(&failure);
    let limits = options.limits;
    let worker = std::thread::Builder::new()
        .name("strop-trace".into())
        .spawn(move || writer::run(file, receiver, writer_failure, limits))
        .map_err(TraceError::Spawn)?;
    let recorder = Arc::new(Recorder {
        admission: Mutex::new(Admission::new(sender, limits)),
        failure,
        started: Instant::now(),
        max_record: limits.record_bytes,
        path: capture_path,
    });
    *active = Some(Arc::clone(&recorder));
    CONTENT.store(options.content == ContentPolicy::Full, Ordering::Release);
    ENABLED.store(true, Ordering::Release);
    Ok(TraceSession {
        recorder,
        worker: Some(worker),
    })
}

#[inline]
pub fn enabled() -> bool {
    ENABLED.load(Ordering::Relaxed)
}

/// The active capture's own output file. Watch consumers suppress only
/// self-generated hints for this resource; user file changes keep their
/// ordinary freshness semantics. No filesystem work occurs at query time.
pub fn active_path() -> Option<PathBuf> {
    if !enabled() {
        return None;
    }
    ACTIVE
        .lock()
        .as_ref()
        .and_then(|capture| capture.path.clone())
}
#[inline]
pub fn capture_content() -> bool {
    enabled() && CONTENT.load(Ordering::Relaxed) && privacy::content_allowed()
}

/// A declared private boundary ends content collection, not diagnostic recording.
/// The producer emits its opaque-scope marker before calling this.
pub(crate) fn stop_content_capture() {
    CONTENT.store(false, Ordering::Release);
}

/// Diagnostic text excerpts stay well below the per-record cap even under
/// the worst-case six-fold JSON escaping of control scalars, so an honest
/// record of any edit still fits one line; the true byte size always
/// travels in a sibling field and the excerpt says when it was cut.
pub const MAX_EXCERPT_BYTES: usize = 32 * 1024;

/// Bound a diagnostic payload to [`MAX_EXCERPT_BYTES`] on a char boundary.
/// Replay values never take excerpts — they chunk instead, because replay
/// equality is byte-exact; this is for diagnostic streams only.
pub fn excerpt(text: &str) -> (&str, bool) {
    if text.len() <= MAX_EXCERPT_BYTES {
        return (text, false);
    }
    let mut end = MAX_EXCERPT_BYTES;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    (&text[..end], true)
}

/// Lazy producer: no payload construction or allocation when disabled.
pub fn record_with<T: Serialize>(kind: EventKind, fields: impl FnOnce() -> T) {
    if enabled() {
        record(kind, &fields());
    }
}

pub fn record<T: Serialize>(kind: EventKind, fields: &T) {
    if !enabled() {
        return;
    }
    let Some(recorder) = ACTIVE.lock().clone() else {
        return;
    };
    record_to(&recorder, kind, fields);
}

fn record_to<T: Serialize>(recorder: &Recorder, kind: EventKind, fields: &T) {
    // Serialize/stamp under admission ownership, never under a disk-write lock.
    let mut admission = recorder.admission.lock();
    if admission.sender.is_none() {
        return;
    }
    if recorder.failure.message.lock().is_some() {
        admission.sender.take();
        return;
    }
    let mut bytes = bounded::Bytes::new(recorder.max_record);
    if serde_json::to_writer(&mut bytes, fields).is_ok() {
        admission.send(
            Record {
                kind,
                elapsed_us: recorder.started.elapsed().as_micros(),
                fields: bytes.into_vec(),
            },
            &recorder.failure,
        );
        return;
    }
    // Only the explicitly enabled forensic substream may carry chunked values.
    if kind == EventKind::Replay && CONTENT.load(Ordering::Relaxed) {
        if let Some(chunks) = chunk::serialize(kind, fields, recorder.max_record) {
            for fields in chunks {
                if !admission.send(
                    Record {
                        kind: EventKind::ReplayChunk,
                        elapsed_us: recorder.started.elapsed().as_micros(),
                        fields,
                    },
                    &recorder.failure,
                ) {
                    break;
                }
            }
            return;
        }
    }
    recorder
        .failure
        .degraded(|| "record exceeds cap or cannot serialize".into());
    admission.sender.take();
}

/// End the capture visibly when honest continuation is impossible (a value
/// beyond even the assembled-value bound, a writer failure): no further
/// records are admitted and the terminal marker reports the capture
/// incomplete instead of silently shrinking.
pub fn mark_incomplete(message: &'static str) {
    let Some(recorder) = ACTIVE.lock().clone() else {
        return;
    };
    recorder.failure.degraded(|| message.into());
    recorder.admission.lock().sender.take();
    CONTENT.store(false, Ordering::Release);
}

/// Report once to the editor's status line; `finish` still fails on the
/// fatal kind, while a degraded capture is only marked in the file.
pub fn take_failure() -> Option<String> {
    let recorder = ACTIVE.lock().clone()?;
    let (_, message) = recorder.failure.message.lock().clone()?;
    (!recorder.failure.reported.swap(true, Ordering::Relaxed)).then_some(message)
}

impl TraceSession {
    pub fn finish(mut self) -> Result<(), TraceError> {
        self.close()
    }

    fn close(&mut self) -> Result<(), TraceError> {
        let Some(worker) = self.worker.take() else {
            return Ok(());
        };
        {
            let mut active = ACTIVE.lock();
            if active
                .as_ref()
                .is_some_and(|value| Arc::ptr_eq(value, &self.recorder))
            {
                ENABLED.store(false, Ordering::Release);
                CONTENT.store(false, Ordering::Release);
                *active = None;
            }
        }
        self.recorder.admission.lock().sender.take();
        if worker.join().is_err() {
            self.recorder
                .failure
                .fatal(|| "writer thread panicked".into());
        }
        match self.recorder.failure.message.lock().clone() {
            Some((Severity::Fatal, error)) => Err(TraceError::Incomplete(error)),
            Some((Severity::Degraded, _)) | None => Ok(()),
        }
    }
}
impl Drop for TraceSession {
    fn drop(&mut self) {
        // Explicit finish is the reporting boundary. Drop guarantees durability
        // during unwinding without risking a second panic or corrupting the TUI.
        let _ = self.close();
    }
}

#[cfg(test)]
mod tests;
