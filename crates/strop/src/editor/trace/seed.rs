//! The forensic startup seed (R11): a pure, serializable image of the
//! editor captured AFTER authorized live startup (file open, config load,
//! session restore) and BEFORE any service start. Replay reconstructs
//! documents with identical arena identities, buffer revisions, undo
//! history and disk baselines — no Client, process, filesystem or network.
use std::io;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};
use strop_core::id::{Arena, ArenaSeed, DocumentId};
use strop_core::worker::{Load, WorkerId, WorkerIds};
use strop_core::{Buffer, BufferSeed};

use crate::editor::document::DocumentSource;
use crate::editor::{Document, Editor, LayoutDir, Pane};

/// One seeded document: its buffer plus whether it came from a file.
/// Surfaces (diff/log/output) are job-owned content, never startup state.
#[derive(Serialize, Deserialize)]
struct DocSeed {
    buffer: BufferSeed,
    file: bool,
}

#[derive(Serialize, Deserialize)]
pub struct Seed {
    documents: ArenaSeed<DocSeed>,
    panes: Vec<Pane>,
    mru: Vec<DocumentId>,
    active: usize,
    layout: LayoutDir,
    #[serde(with = "strop_core::path_serde")]
    cwd: PathBuf,
    #[serde(with = "strop_core::path_serde::option")]
    state_dir: Option<PathBuf>,
    session_policy: crate::session::SessionPolicy,
    config: crate::config::Config,
    git: Option<strop_git::GitContext>,
    git_view: WorkerId,
    git_discovery: Load<crate::editor::git_memory::ContextKey>,
    worker_ids: WorkerIds,
    focus_epoch: u64,
    generation: u64,
    message: String,
}

impl Seed {
    /// Capture the startup state. This is only legal at the seed boundary:
    /// pickers, LSP servers, git surfaces and in-flight discovery mean
    /// services already started and the recording is not a full replay.
    pub fn capture(editor: &Editor) -> io::Result<Self> {
        if editor.picker.is_some()
            || !editor.lsp_servers.is_empty()
            || matches!(editor.git_discovery, Load::Running(_))
            || editor.docs.iter().any(|(_, document)| {
                !matches!(
                    document.source,
                    DocumentSource::File | DocumentSource::Scratch
                )
            })
        {
            return Err(io::Error::other("seed must precede service startup"));
        }
        Ok(Self {
            documents: editor.docs.seed_with(|document| DocSeed {
                buffer: document.buf.seed(),
                file: matches!(document.source, DocumentSource::File),
            }),
            panes: editor.panes.clone(),
            mru: editor.mru.clone(),
            active: editor.active_pane,
            layout: editor.layout,
            cwd: editor.cwd.clone(),
            state_dir: editor.state_dir.clone(),
            session_policy: editor.session_policy,
            config: editor.config.clone(),
            git: editor.git.clone(),
            git_view: editor.git_view,
            git_discovery: editor.git_discovery.clone(),
            worker_ids: editor.worker_ids.clone(),
            focus_epoch: editor.focus_epoch,
            generation: editor.generation,
            message: editor.message.clone(),
        })
    }

    /// Input-only extraction begins with the active startup document, not
    /// a lazily emitted diagnostic snapshot after the first key.
    pub fn input_text(&self) -> io::Result<&str> {
        let pane = self
            .panes
            .get(self.active)
            .ok_or_else(|| io::Error::other("seed has no active pane"))?;
        let document = self
            .documents
            .slots
            .get(pane.doc.index())
            .filter(|(generation, _)| *generation == pane.doc.generation())
            .and_then(|(_, document)| document.as_ref())
            .ok_or_else(|| io::Error::other("seed has no active document"))?;
        Ok(&document.buffer.text)
    }

    /// Rebuild the editor: same identities, same revisions, same history,
    /// pure `new_in` construction, the given tape installed.
    pub fn into_editor(self, tape: std::rc::Rc<strop_trace::replay::Tape>) -> io::Result<Editor> {
        let mut slots = Vec::with_capacity(self.documents.slots.len());
        for (generation, value) in self.documents.slots {
            let document = value
                .map(|seed| {
                    let buffer = seed.buffer.into_buffer().map_err(io::Error::other)?;
                    Ok::<_, io::Error>(if seed.file {
                        Document::new(buffer)
                    } else {
                        Document::scratch(buffer)
                    })
                })
                .transpose()?;
            slots.push((generation, document));
        }
        let docs = Arena::from_seed(ArenaSeed {
            slots,
            free: self.documents.free,
        })
        .map_err(io::Error::other)?;
        if docs.is_empty()
            || self.active >= self.panes.len()
            || self.panes.iter().any(|pane| docs.get(pane.doc).is_none())
            || self.mru.iter().any(|id| docs.get(*id).is_none())
        {
            return Err(io::Error::other("invalid initial document references"));
        }
        for pane in &self.panes {
            let buffer = &docs
                .get(pane.doc)
                .ok_or_else(|| io::Error::other("invalid initial document reference"))?
                .buf;
            for selection in
                std::iter::once(pane.sels.primary()).chain(pane.sels.extra_heads().iter().copied())
            {
                if selection.anchor > buffer.len_bytes()
                    || selection.head > buffer.len_bytes()
                    || !buffer.is_boundary(selection.anchor)
                    || !buffer.is_boundary(selection.head)
                {
                    return Err(io::Error::other("invalid initial selection"));
                }
            }
        }
        let mut editor = Editor::new_in(Buffer::from_text(""), self.cwd);
        editor.docs = docs;
        editor.panes = self.panes;
        editor.mru = self.mru;
        editor.active_pane = self.active;
        editor.layout = self.layout;
        editor.state_dir = self.state_dir;
        editor.session_policy = self.session_policy;
        editor.config = self.config;
        editor.git = self.git;
        editor.git_view = self.git_view;
        editor.git_discovery = self.git_discovery;
        editor.worker_ids = self.worker_ids;
        editor.focus_epoch = self.focus_epoch;
        editor.generation = self.generation;
        editor.message = self.message;
        editor.tape = tape;
        Ok(editor)
    }
}
