//! What is kept, and the one rule for accepting a build's result (0099
//! §4.1, §5.4). Pure: no editor, no channels — the race exploration in
//! `race_tests.rs` drives exactly this.

use std::sync::Arc;

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::WorkerId;
use strop_lsp::{ProtoSymbol, RequestStamp, ServerId};

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

/// The one build in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Build {
    pub id: BuildId,
    pub document: DocumentId,
    pub revision: BufferRevision,
    pub producer: Producer,
    /// `Space s` is waiting for this build: its result opens the picker.
    pub open: bool,
}

/// The document a result is for, as it is when the result arrives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Current {
    pub revision: BufferRevision,
    /// The server the document is open on now, if any.
    pub server: Option<ServerId>,
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
    Fresh(Build, Arc<Vec<ProtoSymbol>>),
}

pub(crate) struct SymbolStore {
    cache: Cache<Arc<Vec<ProtoSymbol>>>,
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
    ) -> Option<Arc<Vec<ProtoSymbol>>> {
        self.cache.get(key(document, producer, revision)).cloned()
    }

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
        symbols: Option<Vec<ProtoSymbol>>,
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
        let served = match build.producer {
            Producer::Outline => true,
            Producer::Server(server) => current.server == Some(server),
        };
        if current.revision != build.revision || !served {
            return Delivery::Stale(build);
        }
        let Some(symbols) = symbols else {
            return Delivery::Failed(build);
        };
        let symbols = Arc::new(symbols);
        self.cache.put(
            key(build.document, build.producer, build.revision),
            symbols.len(),
            symbols.clone(),
        );
        Delivery::Fresh(build, symbols)
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
