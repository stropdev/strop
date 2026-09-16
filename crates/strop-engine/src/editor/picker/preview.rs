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

/// One exact witness check per source revision/dataset/selected hit. Replacement
/// typing and repaints must not compare a megabyte source line again.
pub(super) struct WitnessCheck {
    item: usize,
    dataset: u64,
    document: Option<(strop_core::id::DocumentId, strop_core::id::BufferRevision)>,
    path: Option<strop_workspace::ResourceLocation>,
    range: Option<strop_core::Range>,
}

impl Editor {
    /// Readonly paint query: identical resolution, but an uncached preview
    /// reports loading instead of registering/launching the bounded read.
    /// Preparation calls [`Self::picker_preview`] first.
    pub fn picker_preview_cached(&self) -> Option<(String, Option<usize>, PreviewSource)> {
        let mut probe = PreviewProbe { editor: self };
        probe.resolve()
    }

    /// Readonly paint query: cached witness or a fresh checked range
    /// without writing the witness cache back (preparation owns that).
    pub fn picker_preview_range_cached(
        &self,
        source: &PreviewSource,
    ) -> Result<Option<strop_core::Range>, &'static str> {
        let probe = RangeProbe { editor: self };
        probe.resolve(source)
    }
    pub fn picker_preview_range(
        &mut self,
        source: &PreviewSource,
    ) -> Result<Option<strop_core::Range>, &'static str> {
        let Some(glue) = self
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::Search)
        else {
            return Ok(None);
        };
        let Some(context) = glue.search.as_ref() else {
            return Ok(None);
        };
        let Some(row) = glue.picker.rows.get(glue.picker.selected) else {
            return Ok(None);
        };
        let Payload::Grep { location, .. } = &glue.picker.items[row.item].payload else {
            return Ok(None);
        };
        let target = crate::files::FileTarget::from_location(location)
            .map_err(|_| "invalid source identity")?;
        let (rope, document, path) = match source {
            PreviewSource::Buffer(id) => {
                let Some(doc) = self.docs.get(*id) else {
                    return Err("source closed — refresh Search");
                };
                if !doc.matches_target(&target) {
                    return Err("preview source belongs to another resource");
                }
                (doc.buf.text(), Some((*id, doc.buf.revision())), None)
            }
            PreviewSource::Cached(path) => {
                if path != location {
                    return Err("preview source belongs to another resource");
                }
                let Some(entry) = self.previews.get(path) else {
                    return Ok(None);
                };
                (&entry.rope, None, Some(path))
            }
            _ => return Ok(None),
        };
        let result = |range: Option<strop_core::Range>| {
            range.map(Some).ok_or("source changed — refresh Search")
        };
        if let Some(cached) = glue.preview_witness.as_ref().filter(|cached| {
            cached.item == row.item
                && cached.dataset == context.stamp.dataset
                && witness_document_current(&cached.document, &document)
                && cached.path.as_ref() == path
        }) {
            return result(cached.range);
        }
        let Payload::Grep {
            line,
            col,
            match_len,
            line_text,
            ..
        } = &glue.picker.items[row.item].payload
        else {
            return Ok(None);
        };
        let range = super::checked_hit_range(
            rope,
            &super::ReplacementHit {
                line: *line,
                col: *col,
                match_len: *match_len,
                text: line_text.clone(),
            },
        );
        let checked = WitnessCheck {
            item: row.item,
            dataset: context.stamp.dataset,
            document,
            path: path.cloned(),
            range,
        };
        if let Some(glue) = self.picker.as_mut() {
            glue.preview_witness = Some(checked);
        }
        result(range)
    }
}

/// `&self` mirror of picker_preview_range for the paint path.
struct RangeProbe<'a> {
    editor: &'a Editor,
}
impl RangeProbe<'_> {
    fn resolve(&self, source: &PreviewSource) -> Result<Option<strop_core::Range>, &'static str> {
        let Some(glue) = self
            .editor
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::Search)
        else {
            return Ok(None);
        };
        let Some(context) = glue.search.as_ref() else {
            return Ok(None);
        };
        let Some(row) = glue.picker.rows.get(glue.picker.selected) else {
            return Ok(None);
        };
        let Payload::Grep { location, .. } = &glue.picker.items[row.item].payload else {
            return Ok(None);
        };
        let target = crate::files::FileTarget::from_location(location)
            .map_err(|_| "invalid source identity")?;
        let (rope, document, path) = match source {
            PreviewSource::Buffer(id) => {
                let Some(doc) = self.editor.docs.get(*id) else {
                    return Err("source closed — refresh Search");
                };
                if !doc.matches_target(&target) {
                    return Err("preview source belongs to another resource");
                }
                (doc.buf.text(), Some((*id, doc.buf.revision())), None)
            }
            PreviewSource::Cached(path) => {
                if path != location {
                    return Err("preview source belongs to another resource");
                }
                let Some(entry) = self.editor.previews.get(path) else {
                    return Ok(None);
                };
                (&entry.rope, None, Some(path))
            }
            _ => return Ok(None),
        };
        if let Some(cached) = glue.preview_witness.as_ref().filter(|cached| {
            cached.item == row.item
                && cached.dataset == context.stamp.dataset
                && witness_document_current(&cached.document, &document)
                && cached.path.as_ref() == path
        }) {
            return cached
                .range
                .map(Some)
                .ok_or("source changed — refresh Search");
        }
        let Payload::Grep {
            line,
            col,
            match_len,
            line_text,
            ..
        } = &glue.picker.items[row.item].payload
        else {
            return Ok(None);
        };
        let range = super::checked_hit_range(
            rope,
            &super::ReplacementHit {
                line: *line,
                col: *col,
                match_len: *match_len,
                text: line_text.clone(),
            },
        );
        range.map(Some).ok_or("source changed — refresh Search")
    }
}

/// StaleAcceptsNever (0063 §6.6): a cached witness stands only while
/// the source document is still observed at the revision the witness
/// was checked against — the verified kernel's revision decision; a
/// moved source recomputes (and refuses when the hit no longer
/// witnesses) instead of consuming stale evidence.
fn witness_document_current(
    cached: &Option<(strop_core::id::DocumentId, strop_core::id::BufferRevision)>,
    current: &Option<(strop_core::id::DocumentId, strop_core::id::BufferRevision)>,
) -> bool {
    match (cached, current) {
        (None, None) => true,
        (Some((cached_id, cached_rev)), Some((id, rev))) => {
            cached_id == id
                && strop_core::searchguard::revision_is_current(cached_rev.get(), rev.get())
        }
        _ => false,
    }
}

/// `&self` mirror of the picker_preview resolution for the paint path.
struct PreviewProbe<'a> {
    editor: &'a Editor,
}
impl PreviewProbe<'_> {
    fn resolve(&mut self) -> Option<(String, Option<usize>, PreviewSource)> {
        let item = self.editor.picker.as_ref()?.picker.current()?;
        if let Payload::Buffer(document) = &item.payload {
            let name = self.editor.docs.get(*document)?.label(&self.editor.cwd);
            return Some((name, None, PreviewSource::Buffer(*document)));
        }
        let (full, focus_line) = self.item_source(item)?;
        let title = self.title(&full, focus_line);
        let target = match crate::files::FileTarget::from_location(&full) {
            Ok(target) => target,
            Err(error) => {
                return Some((title, focus_line, PreviewSource::Failed(error.to_string())))
            }
        };
        if let Some((document, _)) = self
            .editor
            .docs
            .iter()
            .find(|(_, document)| document.matches_target(&target))
        {
            return Some((title, focus_line, PreviewSource::Buffer(document)));
        }
        match self.editor.preview_loads.get(&full) {
            Some(Load::Failed { failure, .. }) => Some((
                title,
                focus_line,
                PreviewSource::Failed(failure.message.clone()),
            )),
            Some(Load::Cancelled { reason, .. }) => {
                Some((title, focus_line, PreviewSource::Cancelled(*reason)))
            }
            _ if self.load_ready(&full) => Some((title, focus_line, PreviewSource::Cached(full))),
            _ => Some((title, focus_line, PreviewSource::Loading)),
        }
    }

    /// The cache serves only a preview this picker instance owns (0050):
    /// a respawned search never paints the previous instance's rows.
    fn load_ready(&self, full: &strop_workspace::ResourceLocation) -> bool {
        let Some(picker) = self.editor.picker.as_ref().map(|glue| glue.id) else {
            return false;
        };
        let key = PreviewKey {
            picker,
            path: full.clone(),
        };
        self.editor.previews.contains_key(full)
            && matches!(
                self.editor.preview_loads.get(full),
                Some(Load::Ready(owner)) if *owner == key
            )
    }
    fn title(&self, full: &strop_workspace::ResourceLocation, focus_line: Option<usize>) -> String {
        let path = &full.path;
        let name = path
            .file_name()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| path.display().to_string());
        let parent = path
            .parent()
            .map(|p| p.display().to_string())
            .unwrap_or_default();
        let title = match focus_line {
            Some(line) => format!("{name}:{line}  {parent}"),
            None => format!("{name}  {parent}"),
        }
        .trim_end()
        .to_string();
        if full.local_path().is_none() {
            format!("{} · {title}", full.filesystem.label())
        } else {
            title
        }
    }

    fn item_source(
        &self,
        item: &strop_picker::Item,
    ) -> Option<(strop_workspace::ResourceLocation, Option<usize>)> {
        match &item.payload {
            Payload::RemoteDirectory(_)
            | Payload::RemoteConnect
            | Payload::Jump { .. }
            | Payload::SearchOption(_)
            | Payload::CodeAction(_)
            | Payload::Container(_)
            | Payload::FilesystemAction(_)
            | Payload::ProjectStatus(_)
            | Payload::IndentChoice(_) => None,
            Payload::Buffer(_) => None, // handled via label in the admitting path
            Payload::File(path) => Some((
                strop_workspace::ResourceLocation::local(self.editor.picker_path(path)),
                None,
            )),
            Payload::Grep { location, line, .. } => Some((location.clone(), Some(*line))),
            Payload::Remote {
                endpoint,
                path,
                line,
                ..
            } => Some((
                strop_workspace::ResourceLocation::remote(endpoint.clone(), path.clone()),
                Some(*line),
            )),
        }
    }
}
impl Editor {
    pub fn picker_preview(&mut self) -> Option<(String, Option<usize>, PreviewSource)> {
        let item = self.picker.as_ref()?.picker.current()?;
        let (full, focus_line) = match &item.payload {
            Payload::RemoteDirectory(_)
            | Payload::RemoteConnect
            | Payload::Jump { .. }
            | Payload::SearchOption(_)
            | Payload::CodeAction(_)
            | Payload::Container(_)
            | Payload::FilesystemAction(_)
            | Payload::ProjectStatus(_)
            | Payload::IndentChoice(_) => return None,
            Payload::Buffer(document) => {
                let name = self.docs.get(*document)?.label(&self.cwd);
                return Some((name, None, PreviewSource::Buffer(*document)));
            }
            Payload::File(path) => (
                strop_workspace::ResourceLocation::local(self.picker_path(path)),
                None,
            ),
            Payload::Grep { location, line, .. } => (location.clone(), Some(*line)),
            Payload::Remote {
                endpoint,
                path,
                line,
                ..
            } => (
                strop_workspace::ResourceLocation::remote(endpoint.clone(), path.clone()),
                Some(*line),
            ),
        };
        let path = &full.path;
        // 0050: the header identifies filename + line first; the parent
        // directory follows only while it fits the card.
        let title = {
            let name = path
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.display().to_string());
            let parent = path
                .parent()
                .map(|p| p.display().to_string())
                .unwrap_or_default();
            match focus_line {
                Some(line) => format!("{name}:{line}  {parent}"),
                None => format!("{name}  {parent}"),
            }
            .trim_end()
            .to_string()
        };
        let title = if full.local_path().is_none() {
            format!("{} · {title}", full.filesystem.label())
        } else {
            title
        };
        let target = match crate::files::FileTarget::from_location(&full) {
            Ok(target) => target,
            Err(error) => {
                return Some((title, focus_line, PreviewSource::Failed(error.to_string())))
            }
        };
        if let Some((document, _)) = self
            .docs
            .iter()
            .find(|(_, document)| document.matches_target(&target))
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
    fn preview_ready(&mut self, path: &strop_workspace::ResourceLocation) -> bool {
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return false;
        };
        let key = PreviewKey {
            picker,
            path: path.clone(),
        };
        if self.previews.contains_key(path)
            && matches!(self.preview_loads.get(path), Some(Load::Ready(owner)) if owner == &key)
        {
            return true;
        }
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
            .insert(path.clone(), Load::Running(ticket.clone()));
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"preview","request":request.get(),
                "picker":picker.0.get(),"location":path,
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
        let launch_path = path.clone();
        let client = self.remote_client();
        let tx = self.preview_tx.clone();
        let handle = worker::spawn(
            "strop-preview",
            move |outcome| {
                let _ = tx.send(PreviewResult { ticket, outcome });
            },
            move |token| read_resource_preview(&launch_path, &client, &token),
        );
        self.worker_handles.insert(request, handle);
        false
    }
}

fn read_resource_preview(
    location: &strop_workspace::ResourceLocation,
    client: &strop_remote::RemoteClient,
    token: &worker::CancelToken,
) -> Outcome<PreparedPreview> {
    match crate::files::FileTarget::from_location(location) {
        Ok(crate::files::FileTarget::Local(path)) => read_preview(&path),
        Ok(crate::files::FileTarget::Remote(remote)) => {
            let length = match strop_remote::ReadLimit::new(512 * 1024) {
                Ok(length) => length,
                Err(error) => return Outcome::failed(FailureKind::Protocol, error.to_string()),
            };
            let selection = strop_remote::ReadSelection::Range {
                start: strop_remote::RemoteOffset::new(0),
                length,
            };
            match client.read(&remote, selection, token) {
                Ok(snapshot) if snapshot.window.is_complete() => {
                    Outcome::Success(PreparedPreview {
                        rope: snapshot.buffer.text().clone(),
                    })
                }
                Ok(_) => Outcome::failed(FailureKind::Unavailable, "preview too large"),
                Err(_) if token.is_cancelled() => Outcome::Cancelled(CancelReason::Superseded),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            }
        }
        Ok(crate::files::FileTarget::Container { .. }) => Outcome::failed(
            FailureKind::Unavailable,
            "container search preview is not supported",
        ),
        Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
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
