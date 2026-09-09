//! Owned per-project snapshots and transactional restoration.
//! Scratch and readonly documents never persist. Trust storage is independent.

use crate::editor::{Document, Editor};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use strop_core::{history::History, Buffer};

mod trust;
pub use trust::{is_trusted, is_trusted_remote, trust, trust_remote};
mod persistence;
pub(crate) mod remotes;
#[cfg(test)]
mod tests;

const UNDO_CAP: usize = 200;
const UNDO_BYTES: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionPolicy {
    Automatic,
    Disabled,
}

/// Resolve once at live startup. Replay receives this value in the seed;
/// trust access does not imply automatic session restoration or persistence.
pub fn state_root() -> Option<PathBuf> {
    std::env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
}

#[derive(Debug, thiserror::Error)]
pub enum SessionError {
    #[error("session I/O at {path:?}: {source}")]
    Io {
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
    #[error("session JSON: {0}")]
    Json(#[from] serde_json::Error),
    #[error("invalid session: {0}")]
    Invalid(String),
    #[error(transparent)]
    History(#[from] strop_core::history::HistoryError),
    #[error(transparent)]
    Path(#[from] strop_core::path_serde::PathError),
    #[error("{original}; temporary cleanup also failed: {cleanup}")]
    Cleanup {
        original: Box<SessionError>,
        cleanup: Box<SessionError>,
    },
}

fn io(path: &Path, source: std::io::Error) -> SessionError {
    SessionError::Io {
        path: path.to_owned(),
        source,
    }
}

#[derive(Debug, Default, Serialize, Deserialize)]
pub struct Session {
    buffers: Vec<BufferState>,
    current: usize,
    #[serde(skip)]
    captured: Vec<ropey::Rope>,
}

#[derive(Debug, Serialize, Deserialize)]
struct BufferState {
    #[serde(with = "strop_core::path_serde")]
    path: PathBuf,
    line: usize,
    col: usize,
    view_top: usize,
    undo: Option<History>,
    #[serde(default)]
    undo_hash: u64,
}

/// Fully owned work item: move this to a blocking persistence worker.
#[derive(Debug)]
pub struct SaveRequest {
    path: PathBuf,
    session: Session,
}

fn session_path(base_dir: Option<&Path>, cwd: &Path) -> Option<PathBuf> {
    let base = base_dir?;
    let mut hasher = std::hash::DefaultHasher::new();
    std::hash::Hash::hash(cwd, &mut hasher);
    let key = format!("{:016x}", std::hash::Hasher::finish(&hasher));
    Some(
        base.join("strop")
            .join("sessions")
            .join(format!("{key}.json")),
    )
}

/// Capture owns all paths and history; it never borrows the editor afterward.
pub fn capture(editor: &Editor) -> Option<Session> {
    let mut buffers = Vec::new();
    let mut current = 0;
    let mut captured = Vec::new();
    for (id, doc) in editor.docs.iter() {
        let buf = &doc.buf;
        if buf.readonly {
            continue;
        }
        let Some(path) = &buf.path else {
            continue;
        };
        let active = id == editor.current();
        if active {
            current = buffers.len();
        }
        let (undo, undo_hash) = if buf.history().depth() > 0 {
            let h = buf.history().snapshot(UNDO_CAP, UNDO_BYTES);
            (Some(h), 0)
        } else {
            (None, 0)
        };
        captured.push(buf.snapshot());
        buffers.push(BufferState {
            path: path.clone(),
            line: if active {
                buf.line_of(editor.head())
            } else {
                0
            },
            col: if active { buf.col_of(editor.head()) } else { 0 },
            view_top: if active { editor.view_top() } else { 0 },
            undo,
            undo_hash,
        });
    }
    if buffers.is_empty() {
        None
    } else {
        Some(Session {
            buffers,
            current,
            captured,
        })
    }
}

pub fn capture_save(editor: &Editor) -> Option<SaveRequest> {
    if editor.session_policy == SessionPolicy::Disabled {
        return None;
    }
    let path = session_path(editor.state_dir.as_deref(), &editor.cwd)?;
    Some(SaveRequest {
        path,
        session: capture(editor)?,
    })
}

impl SaveRequest {
    /// Blocking I/O; schedule on a worker, not the async executor thread.
    pub fn persist(mut self) -> Result<(), SessionError> {
        self.session.finish_capture();
        self.session.validate()?;
        persistence::write(&self.path, &self.session)
    }
}

fn content_hash(text: &ropey::Rope) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for b in text.chunks().flat_map(str::bytes) {
        h ^= u64::from(b);
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

impl Session {
    fn finish_capture(&mut self) {
        for (buffer, text) in self.buffers.iter_mut().zip(self.captured.drain(..)) {
            buffer.undo_hash = content_hash(&text);
        }
    }
}

/// Read/decode without an editor, suitable for a blocking worker.
/// Missing state and disabled persistence are ordinary absence, not errors.
pub fn load(base_dir: Option<&Path>, cwd: &Path) -> Result<Option<Session>, SessionError> {
    let Some(path) = session_path(base_dir, cwd) else {
        return Ok(None);
    };
    let bytes = match persistence::read(&path) {
        Ok(bytes) => bytes,
        Err(SessionError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None)
        }
        Err(e) => return Err(e),
    };
    let session: Session = serde_json::from_slice(&bytes)?;
    session.validate()?;
    Ok(Some(session))
}

impl Session {
    fn validate(&self) -> Result<(), SessionError> {
        if self.buffers.is_empty() || self.current >= self.buffers.len() {
            return Err(SessionError::Invalid(
                "empty buffers or invalid current index".into(),
            ));
        }
        for b in &self.buffers {
            strop_core::path_serde::validate(&b.path)?;
            if let Some(h) = &b.undo {
                h.validate()?;
            }
        }
        Ok(())
    }

    /// Open and validate every buffer before changing any live editor state.
    pub fn restore(mut self, editor: &mut Editor) -> Result<(), SessionError> {
        self.finish_capture();
        self.validate()?;
        let mut documents = Vec::with_capacity(self.buffers.len());
        let mut warning = None;
        for b in &self.buffers {
            let path = if b.path.is_absolute() {
                b.path.clone()
            } else {
                editor.cwd.join(&b.path)
            };
            let mut buf = Buffer::open(&path).map_err(|e| io(&path, e))?;
            if let Some(h) = &b.undo {
                if content_hash(buf.text()) == b.undo_hash {
                    buf.restore_history(h.clone())?;
                } else {
                    warning = Some(format!(
                        "{}: changed since capture — undo history dropped",
                        b.path.display()
                    ));
                }
            }
            documents.push(Document::new(buf));
        }
        let b = &self.buffers[self.current];
        let selected = &documents[self.current].buf;
        let last_line = selected.len_lines().saturating_sub(1);
        let line = b.line.min(last_line);
        let start = selected.line_start(line);
        let end = if line < last_line {
            selected.line_start(line + 1).saturating_sub(1)
        } else {
            selected.len_bytes()
        };
        let head = selected.clamp_boundary(start.saturating_add(b.col).min(end));
        let top = b.view_top.min(last_line);
        // All fallible work is complete. Insert first, then remove old IDs, so
        // arena generations cannot accidentally alias an outstanding old ID.
        let old: Vec<_> = editor.docs.iter().map(|(id, _)| id).collect();
        for &id in &old {
            editor.lsp_close_document(id);
        }
        let mut ids = Vec::with_capacity(documents.len());
        for doc in documents {
            ids.push(editor.docs.insert(doc));
        }
        let current = ids[self.current];
        for pane in &mut editor.panes {
            pane.doc = current;
            pane.view_top = 0;
            pane.sels = Default::default();
        }
        editor.view_mut().doc = current;
        for id in old {
            editor.docs.remove(id);
        }
        editor.mru = ids;
        editor.touch_mru(current);
        editor.set_head(head);
        editor.view_mut().view_top = top;
        editor.clamp_cursor();
        editor.generation += 1;
        editor.focus_epoch += 1;
        if let Some(message) = warning {
            editor.message = message;
        }
        Ok(())
    }
}

pub fn restore(editor: &mut Editor) -> Result<bool, SessionError> {
    let Some(session) = load(editor.state_dir.as_deref(), &editor.cwd)? else {
        return Ok(false);
    };
    session.restore(editor)?;
    Ok(true)
}
