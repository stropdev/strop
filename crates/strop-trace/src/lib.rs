//! One opt-in diagnostic sink for all strop crates. Producers never perform file
//! I/O or wait for the writer; an incomplete trace is always reported as such.
mod event;
mod writer;

pub use event::{preview, ContentPolicy, EventKind, TraceOptions, SCHEMA_VERSION};

use parking_lot::Mutex;
use serde::Serialize;
use std::fs::OpenOptions;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{sync_channel, SyncSender};
use std::sync::{Arc, LazyLock};
use std::thread::JoinHandle;
use std::time::Instant;

const QUEUE_CAPACITY: usize = 4096;
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
    message: Mutex<Option<String>>,
    reported: AtomicBool,
}
impl Failure {
    fn set(&self, message: impl FnOnce() -> String) {
        let mut failure = self.message.lock();
        if failure.is_none() {
            *failure = Some(message());
        }
    }
}

struct Record {
    kind: EventKind,
    elapsed_us: u128,
    fields: Vec<u8>,
}
struct Recorder {
    sender: Mutex<Option<SyncSender<Record>>>,
    failure: Arc<Failure>,
    started: Instant,
}

/// Owning lifetime of a trace. Explicit finish reports errors; Drop still drains.
pub struct TraceSession {
    recorder: Arc<Recorder>,
    worker: Option<JoinHandle<()>>,
}

pub fn start(path: &Path, options: TraceOptions) -> Result<TraceSession, TraceError> {
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
    let (sender, receiver) = sync_channel(QUEUE_CAPACITY);
    let failure = Arc::new(Failure::default());
    let writer_failure = Arc::clone(&failure);
    let worker = std::thread::Builder::new()
        .name("strop-trace".into())
        .spawn(move || writer::run(file, receiver, writer_failure))
        .map_err(TraceError::Spawn)?;
    let recorder = Arc::new(Recorder {
        sender: Mutex::new(Some(sender)),
        failure,
        started: Instant::now(),
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
#[inline]
pub fn capture_content() -> bool {
    enabled() && CONTENT.load(Ordering::Relaxed)
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
    let fields = match serde_json::to_vec(fields) {
        Ok(fields) => fields,
        Err(error) => {
            recorder
                .failure
                .set(|| format!("event serialization failed: {error}"));
            return;
        }
    };
    // This lock protects queue admission only, never disk writes. Stamping under
    // the same lock makes timestamps nondecreasing in the writer's receive order.
    let sender = recorder.sender.lock();
    if let Some(sender) = sender.as_ref() {
        let record = Record {
            kind,
            elapsed_us: recorder.started.elapsed().as_micros(),
            fields,
        };
        if let Err(error) = sender.try_send(record) {
            recorder
                .failure
                .set(|| format!("event queue admission failed: {error}"));
        }
    }
}

/// Report once to the editor's status line; finish still returns the failure.
pub fn take_failure() -> Option<String> {
    let recorder = ACTIVE.lock().clone()?;
    let message = recorder.failure.message.lock().clone()?;
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
        self.recorder.sender.lock().take();
        if worker.join().is_err() {
            self.recorder
                .failure
                .set(|| "writer thread panicked".into());
        }
        match self.recorder.failure.message.lock().clone() {
            Some(error) => Err(TraceError::Incomplete(error)),
            None => Ok(()),
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
