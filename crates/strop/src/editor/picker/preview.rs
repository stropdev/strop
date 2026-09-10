//! Picker preview: bounded worker-prepared ropes; open documents use their live
//! snapshots. Syntax belongs to the display-analysis actor, not the UI. Reads run as
//! worker requests — registration precedes launch, every miss is a
//! typed failure (never an empty success), and a failure never poisons
//! the path forever.

use std::io::Read;
use std::path::Path;

use strop_core::worker::{self, CancelReason, Failure, FailureKind, Load, Outcome, Ticket};
use strop_picker::Payload;

use super::super::Editor;
use super::{PreviewKey, PreviewResult, PreviewSource};

/// Live delivery already owns a rope. Replay reconstructs the same pure snapshot
/// from the recorded text; no parser or filesystem access crosses this boundary.
#[derive(Debug, Clone)]
pub struct PreparedPreview {
    pub rope: ropey::Rope,
}
impl From<String> for PreparedPreview {
    fn from(text: String) -> Self {
        Self {
            rope: ropey::Rope::from_str(&text),
        }
    }
}
impl serde::Serialize for PreparedPreview {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.rope)
    }
}
impl<'de> serde::Deserialize<'de> for PreparedPreview {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer).map(Self::from)
    }
}

impl Editor {
    pub fn picker_preview(&mut self) -> Option<(String, Option<usize>, PreviewSource)> {
        let item = self.picker.as_ref()?.picker.current()?;
        let (path, focus_line) = match &item.payload {
            Payload::RemoteDirectory(_)
            | Payload::RemoteConnect
            | Payload::CodeAction(_)
            | Payload::Container(_) => return None,
            Payload::Buffer(document) => {
                let name = self
                    .docs
                    .get(*document)?
                    .buf
                    .path
                    .as_ref()
                    .map(|path| path.to_string_lossy().into_owned())
                    .unwrap_or_else(|| "[scratch]".into());
                return Some((name, None, PreviewSource::Buffer(*document)));
            }
            Payload::File(path) => (path.clone(), None),
            Payload::Grep { path, line, .. } => (path.clone(), Some(*line)),
            // A remote hit never previews from the local disk: the
            // analogous path is another machine's file (0036). The
            // endpoint-labelled title says where it lives; accepting
            // opens it remotely.
            Payload::Remote {
                endpoint,
                path,
                line,
                ..
            } => {
                return Some((
                    format!("{endpoint}{}", path.display()),
                    Some(*line),
                    PreviewSource::Failed("remote hit — accept to open".into()),
                ));
            }
        };
        let full = self.cwd.join(&path);
        let title = path.display().to_string();
        if let Some((document, _)) = self
            .docs
            .iter()
            .find(|(_, document)| document.buf.path.as_ref() == Some(&full))
        {
            return Some((title, focus_line, PreviewSource::Buffer(document)));
        }
        match self.preview_loads.get(&full) {
            Some(Load::Failed { failure, .. }) => {
                return Some((
                    title,
                    focus_line,
                    PreviewSource::Failed(failure.message.clone()),
                ));
            }
            Some(Load::Cancelled { reason, .. }) => {
                return Some((title, focus_line, PreviewSource::Cancelled(*reason)));
            }
            _ => {}
        }
        if !self.preview_ready(&full) {
            return Some((title, focus_line, PreviewSource::Loading));
        }
        Some((title, focus_line, PreviewSource::Cached(full)))
    }

    /// True when the preview is cached. Otherwise registers an owned
    /// request for this picker instance (cancelling any stale one) and
    /// launches the bounded read; the next tick picks the result up.
    fn preview_ready(&mut self, path: &Path) -> bool {
        if self.previews.contains_key(path) {
            return true;
        }
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return false;
        };
        let key = PreviewKey {
            picker,
            path: path.to_path_buf(),
        };
        // a request from this instance — running, failed or cancelled —
        // owns the path: frames never silently retry
        if self
            .preview_loads
            .get(path)
            .is_some_and(|load| load.covers(&key))
        {
            return false;
        }
        // a stale Running ticket (older instance) must not block
        if let Some(Load::Running(old)) = self.preview_loads.get(path).cloned() {
            if let Some(handle) = self.worker_handles.remove(&old.request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return false;
            }
        };
        let ticket = Ticket {
            request,
            key: key.clone(),
        };
        // registration precedes launch — replay mode stops here and
        // only injected results populate the cache
        self.preview_loads
            .insert(path.to_path_buf(), Load::Running(ticket.clone()));
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"preview","request":request.get(),
                "picker":picker.0.get(),"path":path.to_string_lossy(),
            })
        });
        match self
            .tape
            .request("strop-preview", &serde_json::json!({"ticket":ticket}))
        {
            Ok(false) => return false,
            Ok(true) => {}
            Err(error) => {
                self.handle_preview(PreviewResult {
                    ticket,
                    outcome: Outcome::failed(FailureKind::Protocol, error.to_string()),
                });
                return false;
            }
        }
        let launch_path = path.to_path_buf();
        let tx = self.preview_tx.clone();
        let handle = worker::spawn(
            "strop-preview",
            move |outcome| {
                let _ = tx.send(PreviewResult { ticket, outcome });
            },
            move |_| read_preview(&launch_path),
        );
        self.worker_handles.insert(request, handle);
        false
    }
}

/// The bounded preview read: at most 512 KiB, real files only, valid
/// UTF-8 — every miss is a typed failure, never an empty success. The
/// read stays capped even if the file grows after metadata.
pub(crate) fn read_preview(path: &Path) -> Outcome<PreparedPreview> {
    const LIMIT: u64 = 512 * 1024;
    let read = || -> Result<String, Failure> {
        let meta =
            std::fs::metadata(path).map_err(|e| Failure::new(FailureKind::Io, e.to_string()))?;
        if !meta.is_file() {
            return Err(Failure::new(
                FailureKind::Unavailable,
                "preview target is not a file",
            ));
        }
        if meta.len() > LIMIT {
            return Err(Failure::new(FailureKind::Unavailable, "preview too large"));
        }
        let file =
            std::fs::File::open(path).map_err(|e| Failure::new(FailureKind::Io, e.to_string()))?;
        let mut bytes = Vec::new();
        file.take(LIMIT + 1)
            .read_to_end(&mut bytes)
            .map_err(|e| Failure::new(FailureKind::Io, e.to_string()))?;
        if bytes.len() as u64 > LIMIT {
            return Err(Failure::new(FailureKind::Unavailable, "preview too large"));
        }
        String::from_utf8(bytes).map_err(|e| Failure::new(FailureKind::Io, e.to_string()))
    };
    match read() {
        Ok(text) => Outcome::Success(text.into()),
        Err(failure) => Outcome::Failed {
            failure,
            partial: None,
        },
    }
}
