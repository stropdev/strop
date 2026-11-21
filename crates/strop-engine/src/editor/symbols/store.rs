//! What is kept, and the one rule for accepting a build's result (0099
//! §4.1, §5.4). Pure: no editor, no channels — the race exploration in
//! `race_tests.rs` drives exactly this.

use std::sync::Arc;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::WorkerId;
use strop_lsp::{PositionEncoding, ProtoSymbol, RequestStamp, ServerId};

use crate::editor::cache::{Cache, Key};

/// Symbol lists kept at once.
const ENTRIES: usize = 64;
/// The most symbols one kept list may have; a longer one is shown and not
/// kept. Together: at most 1.28 million kept symbols.
pub(crate) const SYMBOL_LIMIT: usize = 20_000;

/// Who computed a list.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Producer {
    /// The linked tree-sitter grammar.
    Outline,
    Server(ServerId),
}

impl Producer {
    fn id(self) -> u64 {
        match self {
            Producer::Outline => 0,
            // server ids count up from zero and are never reused
            Producer::Server(server) => server.get().saturating_add(1),
        }
    }
}

fn subject(document: DocumentId) -> u64 {
    ((document.index() as u64) << 32) | u64::from(document.generation())
}

fn key(document: DocumentId, producer: Producer, revision: BufferRevision) -> Key {
    Key {
        subject: subject(document),
        producer: producer.id(),
        version: revision.get(),
    }
}

/// The identity a build's result must carry to be the awaited one.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BuildId {
    Outline(WorkerId),
    Server(RequestStamp),
}

/// Registry identity plus this filesystem connection's incarnation.
pub(crate) type Workspace = Option<(strop_core::id::WorkspaceId, u64)>;

/// The one build in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Build {
    pub id: BuildId,
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub producer: Producer,
    pub workspace: Workspace,
    /// `Space s` is waiting for this build: its result opens the picker.
    pub open: bool,
}

/// The document a result is for, as it is when the result arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Current {
    pub revision: BufferRevision,
    pub document: DocumentId,
    pub producer: Option<Producer>,
    pub workspace: Workspace,
}

/// Columns in a kept list must retain the encoding of the original reply.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct SymbolList {
    pub(crate) rows: Arc<Vec<ProtoSymbol>>,
    pub(crate) catalog: strop_picker::Catalog,
    pub(crate) listing: Arc<Vec<strop_picker::Row>>,
    pub(crate) root: Arc<strop_workspace::ResourceLocation>,
    pub(crate) location: Arc<strop_workspace::ResourceLocation>,
    /// Set only from the owned build's dispatch stamp at admission.
    #[serde(skip)]
    pub(crate) workspace: Workspace,
}

impl SymbolList {
    pub(super) fn prepare(
        mut rows: Vec<ProtoSymbol>,
        root: &std::path::Path,
        location: &strop_workspace::ResourceLocation,
        server: Option<(ServerId, PositionEncoding)>,
    ) -> Self {
        rows.retain(|row| &row.location.doc == location);
        let catalog = super::super::lsp::document_symbol_items(&rows, root, server).into();
        let listing = (0..rows.len()).map(strop_picker::Row::listing).collect();
        let root = Arc::new(strop_workspace::ResourceLocation {
            filesystem: location.filesystem.clone(),
            path: root.to_owned(),
        });
        Self {
            rows: Arc::new(rows),
            catalog,
            listing: Arc::new(listing),
            root,
            location: Arc::new(location.clone()),
            workspace: None,
        }
    }
}

pub(crate) enum Delivery {
    /// Not the build being waited for: nothing changes.
    Unowned,
    /// The awaited build, but the document, its text or its server moved on
    /// while it ran: the result is dropped.
    Stale(Build),
    /// The awaited build failed or was cancelled.
    Failed(Build),
    /// Accepted, and kept unless it is over the size limit.
    Fresh(Build, SymbolList),
}

pub(crate) struct SymbolStore {
    cache: Cache<SymbolList>,
    build: Option<Build>,
}

impl Default for SymbolStore {
    fn default() -> Self {
        Self {
            cache: Cache::new(ENTRIES, SYMBOL_LIMIT),
            build: None,
        }
    }
}

impl SymbolStore {
    /// The kept list for this document, producer and revision.
    pub fn lookup(
        &mut self,
        document: DocumentId,
        producer: Producer,
        revision: BufferRevision,
    ) -> Option<SymbolList> {
        self.cache.get(key(document, producer, revision)).cloned()
    }

    #[cfg(test)]
    pub fn has(&self, document: DocumentId, producer: Producer, revision: BufferRevision) -> bool {
        self.cache.contains(key(document, producer, revision))
    }

    pub fn building(&self) -> Option<&Build> {
        self.build.as_ref()
    }

    pub fn building_mut(&mut self) -> Option<&mut Build> {
        self.build.as_mut()
    }

    /// Own a new build. At most one is in flight (0099 §5.3): the caller
    /// has abandoned the previous one.
    pub fn begin(&mut self, build: Build) {
        debug_assert!(self.build.is_none(), "one symbols build at a time");
        self.build = Some(build);
    }

    /// Stop waiting for the build in flight; its result will be `Unowned`.
    pub fn abandon(&mut self) -> Option<Build> {
        self.build.take()
    }

    /// A build's result arrived (0099 §5.4). It is accepted only when it is
    /// the build being waited for, its document still exists at the revision
    /// the build read, and — for a server — that server still serves the
    /// document. It is stored under the revision it was computed for.
    pub fn deliver(
        &mut self,
        id: BuildId,
        current: Option<Current>,
        symbols: Option<SymbolList>,
    ) -> Delivery {
        if self.build.map(|build| build.id) != Some(id) {
            return Delivery::Unowned;
        }
        let Some(build) = self.build.take() else {
            return Delivery::Unowned;
        };
        let Some(current) = current else {
            return Delivery::Stale(build);
        };
        if current.document != build.document
            || current.revision != build.revision
            || current.workspace != build.workspace
            || current.producer != Some(build.producer)
        {
            return Delivery::Stale(build);
        }
        let Some(mut list) = symbols else {
            return Delivery::Failed(build);
        };
        list.workspace = build.workspace;
        self.cache.put(
            key(build.document, build.producer, build.revision),
            list.rows.len(),
            list.clone(),
        );
        Delivery::Fresh(build, list)
    }

    /// The document closed: its lists are gone and its build is unowned.
    pub fn close(&mut self, document: DocumentId) {
        self.cache.retire_subject(subject(document));
        if self.build.is_some_and(|build| build.document == document) {
            self.build = None;
        }
    }

    /// The server failed or was retired: its lists are gone and its build
    /// is unowned.
    pub fn retire_server(&mut self, server: ServerId) {
        let producer = Producer::Server(server);
        self.cache.retire_producer(producer.id());
        if self.build.is_some_and(|build| build.producer == producer) {
            self.build = None;
        }
    }
}
