//! Picker preview: bounded worker-prepared ropes; open documents use their live
//! snapshots. Syntax belongs to the display-analysis actor, not the UI. Reads run as
//! worker requests — registration precedes launch, every miss is a
//! typed failure (never an empty success), and a failure never poisons
//! the path forever.

use std::io::Read;
use std::path::Path;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};

use strop_core::worker::{self, CancelReason, FailureKind, Load, Outcome, Ticket};
use strop_picker::Payload;

use super::super::Editor;
use super::{PreviewKey, PreviewResult, PreviewSource, PreviewTitle};

mod watchdog;
pub(crate) use watchdog::{physical_complete, publish, PreviewWatchdog, PREVIEW_STALLED};

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

/// A Search hit's preview range no longer witnessed by its source.
const SOURCE_CHANGED: &str = "source changed — refresh Search";

/// How the selected Search hit's range resolves in a preview source: an
/// answer, or a fresh witness check the `&mut` preparation path caches.
enum RangeResolution {
    Answer(Option<strop_core::Range>),
    Checked(WitnessCheck),
}

impl WitnessCheck {
    fn answer(&self) -> Result<Option<strop_core::Range>, &'static str> {
        self.range.map(Some).ok_or(SOURCE_CHANGED)
    }
}

/// What the selected picker item previews, resolved without side
/// effects. `resource` is the location the selection reads, when it
/// reads one: in-flight reads of any other location are superseded.
struct PreviewPlan {
    resource: Option<strop_workspace::ResourceLocation>,
    preview: Option<(PreviewTitle, Option<usize>, PreviewSource)>,
}

impl Editor {
    pub(super) fn selected_preview_path(&self) -> Option<strop_workspace::ResourceLocation> {
        let item = self.picker.as_ref()?.picker.current()?;
        self.preview_item_source(item).map(|(path, _)| path)
    }

    /// Readonly paint query: the same resolution as [`Self::picker_preview`],
    /// but an uncached read reports loading instead of registering and
    /// launching it. Preparation calls [`Self::picker_preview`] first.
    pub fn picker_preview_cached(&self) -> Option<(PreviewTitle, Option<usize>, PreviewSource)> {
        self.preview_plan()?.preview
    }

    /// Preparation query: resolves like [`Self::picker_preview_cached`],
    /// supersedes other in-flight reads, and launches the bounded read a
    /// loading preview needs.
    pub fn picker_preview(&mut self) -> Option<(PreviewTitle, Option<usize>, PreviewSource)> {
        let picker = self.picker.as_ref()?.id;
        let plan = self.preview_plan()?;
        self.cancel_other_previews(picker, plan.resource.as_ref());
        match (plan.preview, plan.resource) {
            (Some((title, focus_line, PreviewSource::Loading)), Some(full)) => {
                if self.preview_ready(&full) {
                    Some((title, focus_line, PreviewSource::Cached(full)))
                } else {
                    Some((title, focus_line, PreviewSource::Loading))
                }
            }
            (preview, _) => preview,
        }
    }

    /// `None` without a picker or a current item; otherwise the plan.
    fn preview_plan(&self) -> Option<PreviewPlan> {
        let item = self.picker.as_ref()?.picker.current()?;
        let document = |document: strop_core::id::DocumentId, offset: Option<usize>| {
            let doc = self.docs.get(document)?;
            let line = offset.map(|offset| doc.buf.line_of(offset.min(doc.buf.len_bytes())) + 1);
            let title = document_title(doc, &self.cwd);
            Some((title, line, PreviewSource::Buffer(document)))
        };
        let (full, focus_line) = match &item.payload {
            Payload::Buffer(id) => {
                return Some(PreviewPlan {
                    resource: None,
                    preview: document(*id, None),
                })
            }
            Payload::Jump {
                document: id,
                offset,
            } => {
                return Some(PreviewPlan {
                    resource: None,
                    preview: document(*id, Some(*offset)),
                })
            }
            _ => match self.preview_item_source(item) {
                Some(source) => source,
                None => {
                    return Some(PreviewPlan {
                        resource: None,
                        preview: None,
                    })
                }
            },
        };
        // 0050: the header identifies the file; the renderer makes the
        // path workspace-relative and keeps the authority its own segment.
        let title = PreviewTitle::of(&full);
        let source = match crate::files::FileTarget::from_location(&full) {
            Err(error) => PreviewSource::Failed(error.to_string()),
            Ok(target) => match self
                .docs
                .iter()
                .find(|(_, document)| document.matches_target(&target))
            {
                Some((document, _)) => PreviewSource::Buffer(document),
                None => match self.preview_loads.get(&full) {
                    Some(Load::Failed { failure, .. }) => {
                        PreviewSource::Failed(failure.message.clone())
                    }
                    Some(Load::Cancelled { reason, .. }) => PreviewSource::Cancelled(*reason),
                    _ if self.preview_load_ready(&full) => PreviewSource::Cached(full.clone()),
                    _ => PreviewSource::Loading,
                },
            },
        };
        Some(PreviewPlan {
            preview: Some((title, focus_line, source)),
            resource: Some(full),
        })
    }

    /// The cache serves only a preview this picker instance owns (0050):
    /// a respawned search never paints the previous instance's rows.
    fn preview_load_ready(&self, full: &strop_workspace::ResourceLocation) -> bool {
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return false;
        };
        let key = PreviewKey {
            picker,
            path: full.clone(),
        };
        self.previews.contains_key(full)
            && matches!(
                self.preview_loads.get(full),
                Some(Load::Ready(owner)) if *owner == key
            )
    }

    /// The resource a file-backed item previews, with its focus line.
    /// Documents (`Buffer`, `Jump`) and non-file items have none.
    fn preview_item_source(
        &self,
        item: &strop_picker::Item,
    ) -> Option<(strop_workspace::ResourceLocation, Option<usize>)> {
        match &item.payload {
            Payload::WorkspaceRoot(_)
            | Payload::RemoteDirectory(_)
            | Payload::RemoteConnect
            | Payload::Buffer(_)
            | Payload::Jump { .. }
            | Payload::Action(_)
            | Payload::ProjectStatus(_)
            | Payload::IndentChoice(_)
            | Payload::ThemeChoice(_)
            | Payload::Branch(_) => None,
            Payload::File(path) => Some((
                strop_workspace::ResourceLocation::local(self.picker_path(path)),
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

    /// Readonly paint query: cached witness or a fresh checked range
    /// without writing the witness cache back (preparation owns that).
    pub fn picker_preview_range_cached(
        &self,
        source: &PreviewSource,
    ) -> Result<Option<strop_core::Range>, &'static str> {
        match self.resolve_preview_range(source)? {
            RangeResolution::Answer(range) => Ok(range),
            RangeResolution::Checked(check) => check.answer(),
        }
    }

    /// Preparation query: as [`Self::picker_preview_range_cached`], and a
    /// fresh check is kept as the witness for later frames.
    pub fn picker_preview_range(
        &mut self,
        source: &PreviewSource,
    ) -> Result<Option<strop_core::Range>, &'static str> {
        match self.resolve_preview_range(source)? {
            RangeResolution::Answer(range) => Ok(range),
            RangeResolution::Checked(check) => {
                let answer = check.answer();
                if let Some(glue) = self.picker.as_mut() {
                    glue.preview_witness = Some(check);
                }
                answer
            }
        }
    }

    fn resolve_preview_range(
        &self,
        source: &PreviewSource,
    ) -> Result<RangeResolution, &'static str> {
        let none = Ok(RangeResolution::Answer(None));
        let Some(glue) = self
            .picker
            .as_ref()
            .filter(|glue| glue.picker.kind == strop_picker::Kind::Search)
        else {
            return none;
        };
        let Some(context) = glue.search.as_ref() else {
            return none;
        };
        let Some(row) = glue.picker.rows.get(glue.picker.selected) else {
            return none;
        };
        let Payload::Grep {
            location,
            line,
            col,
            match_len,
            line_text,
        } = &glue.picker.items[row.item].payload
        else {
            return none;
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
                    return none;
                };
                (&entry.rope, None, Some(path))
            }
            _ => return none,
        };
        if let Some(cached) = glue.preview_witness.as_ref().filter(|cached| {
            cached.item == row.item
                && cached.dataset == context.stamp.dataset
                && witness_document_current(&cached.document, &document)
                && cached.path.as_ref() == path
        }) {
            return cached.answer().map(RangeResolution::Answer);
        }
        let range = super::checked_hit_range(
            rope,
            &super::ReplacementHit {
                line: *line,
                col: *col,
                match_len: *match_len,
                text: line_text.clone(),
            },
        );
        Ok(RangeResolution::Checked(WitnessCheck {
            item: row.item,
            dataset: context.stamp.dataset,
            document,
            path: path.cloned(),
            range,
        }))
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

/// A document's header: its file (remote authority split out) or its
/// label when it has none — a scratch buffer, a terminal.
fn document_title(doc: &crate::editor::Document, cwd: &Path) -> PreviewTitle {
    if let Some(remote) = doc.remote_metadata() {
        return PreviewTitle::remote(remote.file.endpoint(), remote.file.path());
    }
    match doc.buf.path.as_deref() {
        Some(path) if matches!(&doc.source, crate::editor::DocumentSource::File) => {
            PreviewTitle::local(strop_workspace::directory::display_path(path))
        }
        _ => PreviewTitle::local(doc.label(cwd)),
    }
}

impl Editor {
    fn cancel_other_previews(
        &mut self,
        picker: super::PickerId,
        current: Option<&strop_workspace::ResourceLocation>,
    ) {
        let stale: Vec<_> = self
            .preview_loads
            .iter()
            .filter_map(|(path, load)| match load {
                Load::Running(ticket) if ticket.key.picker == picker && current != Some(path) => {
                    Some((path.clone(), ticket.request))
                }
                _ => None,
            })
            .collect();
        for (path, request) in stale {
            self.preview_loads.remove(&path);
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::Superseded);
            }
        }
    }

    /// True when the preview is cached. Otherwise registers an owned
    /// request for this picker instance (cancelling any stale one) and
    /// launches the bounded read; the next tick picks the result up.
    fn preview_ready(&mut self, path: &strop_workspace::ResourceLocation) -> bool {
        self.preview_ready_with(path, read_resource_preview)
    }

    fn preview_ready_with(
        &mut self,
        path: &strop_workspace::ResourceLocation,
        read: impl FnOnce(
                &strop_workspace::ResourceLocation,
                &strop_remote::RemoteClient,
                &worker::CancelToken,
            ) -> Outcome<PreparedPreview>
            + Send
            + 'static,
    ) -> bool {
        let Some(picker) = self.picker.as_ref().map(|glue| glue.id) else {
            return false;
        };
        self.cancel_other_previews(picker, Some(path));
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
        self.admissions.previews += 1;
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
        let tx = self.preview_tx.clone();
        let terminal = Arc::new(AtomicBool::new(false));
        if self.preview_watchdog.is_stalled() {
            publish(
                &terminal,
                &tx,
                ticket,
                Outcome::failed(FailureKind::Unavailable, PREVIEW_STALLED),
            );
            return false;
        }
        if let Err(failure) =
            self.preview_watchdog
                .watch(ticket.clone(), tx.clone(), terminal.clone())
        {
            publish(
                &terminal,
                &tx,
                ticket,
                Outcome::Failed {
                    failure,
                    partial: None,
                },
            );
            return false;
        }
        if terminal.load(Ordering::Acquire) {
            return false;
        }

        let launch_path = path.clone();
        let client = self.remote_client();
        let emit_terminal = terminal.clone();
        let emit_tx = tx.clone();
        let emit_ticket = ticket.clone();
        let prepared = worker::prepare(move |outcome| {
            publish(&emit_terminal, &emit_tx, emit_ticket, outcome);
        });
        let handle = prepared.cancel_handle();
        let watchdog = self.preview_watchdog.shared.clone();
        let spawn = std::thread::Builder::new()
            .name("strop-preview".into())
            .spawn(move || {
                prepared.run(move |token| read(&launch_path, &client, &token));
                physical_complete(&watchdog, request.get());
            });
        if let Err(error) = spawn {
            physical_complete(&self.preview_watchdog.shared, request.get());
            publish(
                &terminal,
                &tx,
                ticket,
                Outcome::failed(FailureKind::ThreadStart, error.to_string()),
            );
        }
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
        Ok(crate::files::FileTarget::Local(path)) => read_preview(&path, token),
        Ok(crate::files::FileTarget::Remote(remote)) => {
            match client.read(&remote, 512 * 1024, token) {
                Ok(snapshot) => Outcome::Success(PreparedPreview {
                    rope: snapshot.buffer.text().clone(),
                }),
                Err(error) if error.kind() == strop_remote::RemoteFailureKind::TooLarge => {
                    Outcome::failed(FailureKind::Unavailable, "preview too large")
                }
                Err(_) if token.is_cancelled() => Outcome::Cancelled(CancelReason::Superseded),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            }
        }
        Err(error) => Outcome::failed(FailureKind::Protocol, error.to_string()),
    }
}

/// The bounded preview read: at most 512 KiB, real files only, valid
/// UTF-8 — every miss is a typed failure, never an empty success. The
/// read stays capped even if the file grows after metadata.
pub(crate) fn read_preview(path: &Path, token: &worker::CancelToken) -> Outcome<PreparedPreview> {
    const LIMIT: u64 = 512 * 1024;
    if token.is_cancelled() {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let meta = match std::fs::metadata(path) {
        Ok(meta) => meta,
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    if !meta.is_file() {
        return Outcome::failed(FailureKind::Unavailable, "preview target is not a file");
    }
    if meta.len() > LIMIT {
        return Outcome::failed(FailureKind::Unavailable, "preview too large");
    }
    if token.is_cancelled() {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let file = match std::fs::File::open(path) {
        Ok(file) => file,
        Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
    };
    if token.is_cancelled() {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    let mut file = file.take(LIMIT + 1);
    let mut bytes = Vec::with_capacity(meta.len() as usize);
    let mut chunk = [0; 16 * 1024];
    loop {
        if token.is_cancelled() {
            return Outcome::Cancelled(CancelReason::Superseded);
        }
        let read = match file.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => read,
            Err(error) => return Outcome::failed(FailureKind::Io, error.to_string()),
        };
        bytes.extend_from_slice(&chunk[..read]);
        if bytes.len() as u64 > LIMIT {
            return Outcome::failed(FailureKind::Unavailable, "preview too large");
        }
    }
    if token.is_cancelled() {
        return Outcome::Cancelled(CancelReason::Superseded);
    }
    match String::from_utf8(bytes) {
        Ok(text) => Outcome::Success(text.into()),
        Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
    }
}
