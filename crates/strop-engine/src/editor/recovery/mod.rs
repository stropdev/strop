//! Real dirty/scratch draft checkpoint/recovery (0056 AR04 §5) — a
//! different product from session restore. Session metadata reopens files;
//! this module persists the unsaved BYTES of ordinary local dirty documents
//! and scratch drafts in private state storage, as coalesced immutable
//! cohort snapshots on a bounded worker queue (one in flight, one queued,
//! newest replaces the unwritten). Checkpointing never marks text clean;
//! the guarantee is the last COMPLETED durable checkpoint, and the three
//! clocks — applied revision, captured checkpoint, durable checkpoint —
//! stay separate. Remote/sensitive drafts persist only with explicit
//! session consent; memory-only mode persists nothing and says so.

mod record;
mod store;
mod surface;
#[cfg(test)]
mod tests;

pub use record::{
    CohortHeader, DraftOrigin, DraftRecord, Snapshot, SourceObservation, WorkspaceBinding,
};
pub use store::StoredCohort;

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use strop_core::id::{BufferRevision, DocumentId};
use strop_core::worker::{self, FailureKind, Outcome, WorkerId};

use super::document::DocumentSource;
use super::Editor;

/// Persistence policy for draft recovery. `MemoryOnly` is honest: nothing
/// is written, and the status/explain surfaces say drafts are not durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Policy {
    #[default]
    Automatic,
    MemoryOnly,
}

/// The worker's report back to the editor. Checkpoint payloads never carry
/// draft text; Loaded/Discarded carry the durable store's records.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RecoveryReport {
    Checkpoint(store::Published),
    Loaded(Option<Box<StoredCohort>>),
    Discarded(Box<StoredCohort>),
}

/// One publication work item. Captures and deliberate discards share the
/// single in-flight slot so the checkpoint file has exactly one writer.
enum Publication {
    Capture {
        cohort: u64,
        captured_ms: u64,
        workspace: record::WorkspaceBinding,
        records: Vec<store::PendingRecord>,
    },
    Discard {
        stored: Box<StoredCohort>,
        index: usize,
    },
}

/// What the user asked the recovery store for; the load worker's
/// completion runs it (`:recover` surfaces are read-before-act).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SurfaceIntent {
    List,
    Restore(usize),
    Discard(usize),
}

/// Recovery bookkeeping: clocks, queue, consent and the last known
/// durable store contents. The durable checkpoint file itself is the
/// source of truth; these maps are this session's view of it.
pub struct RecoveryState {
    pub policy: Policy,
    /// Session-scoped explicit consent to persist remote/sensitive drafts
    /// (0056 AR04 §5). Runtime-only by construction: no config layer can
    /// grant it, so a project config can never persist itself.
    pub(crate) consent_remote: bool,
    cohort: u64,
    in_flight: Option<WorkerId>,
    queued: Option<Publication>,
    loading: Option<WorkerId>,
    /// Revisions the latest capture holds (captured checkpoint clock).
    captured: HashMap<DocumentId, BufferRevision>,
    /// Revisions the last completed publication holds (durable clock).
    durable: HashMap<DocumentId, BufferRevision>,
    durable_cohort: Option<(u64, u64)>,
    over_bound: Vec<(DocumentId, u64)>,
    scratch_ids: HashMap<DocumentId, u64>,
    next_scratch: u64,
    pub(crate) last_error: Option<String>,
    announced: bool,
    pub(crate) loaded: Option<StoredCohort>,
    intent: Option<SurfaceIntent>,
}

impl Default for RecoveryState {
    fn default() -> Self {
        Self {
            policy: Policy::Automatic,
            consent_remote: false,
            cohort: 0,
            in_flight: None,
            queued: None,
            loading: None,
            captured: HashMap::new(),
            durable: HashMap::new(),
            durable_cohort: None,
            over_bound: Vec::new(),
            scratch_ids: HashMap::new(),
            next_scratch: 1,
            last_error: None,
            announced: false,
            loaded: None,
            intent: None,
        }
    }
}

impl RecoveryState {
    pub fn pending(&self) -> bool {
        self.in_flight.is_some() || self.loading.is_some()
    }
    pub fn write_pending(&self, request: WorkerId) -> bool {
        self.in_flight == Some(request)
    }
}

/// The status/explain view (0056 AR04 §5): policy, the durable watermark,
/// queue state, consent and failures — never a bare "enabled" flag.
pub struct RecoveryStatus {
    pub memory_only: bool,
    pub durable_cohort: Option<(u64, u64)>,
    pub durable_records: usize,
    pub in_flight: bool,
    pub queued: bool,
    pub consent_remote: bool,
    pub over_bound: Vec<(DocumentId, u64)>,
    pub last_error: Option<String>,
}

impl Editor {
    /// Persistence is real only when policy, session mode and a private
    /// state directory all allow it; anything else is memory-only.
    pub(crate) fn recovery_persistent(&self) -> bool {
        self.recovery.policy == Policy::Automatic
            && self.session_policy == crate::session::SessionPolicy::Automatic
            && self.state_dir.is_some()
    }

    pub fn recovery_memory_only(&self) -> bool {
        !self.recovery_persistent()
    }

    /// Default coverage (0056 AR04 §5): ordinary local dirty documents and
    /// scratch drafts. Remote drafts need explicit session consent;
    /// surfaces, terminals, containers and generated collections are never
    /// substitute source backups and stay out.
    fn recovery_doc_eligible(&self, doc: &super::Document) -> bool {
        if !doc.buf.dirty {
            return false;
        }
        match &doc.source {
            DocumentSource::File => doc.buf.path.is_some(),
            DocumentSource::Scratch => true,
            DocumentSource::Remote(_) => super::privacy::persistence_admitted(
                &self.effect_policy(),
                super::privacy::EffectTarget::Ssh,
            ),
            _ => false,
        }
    }

    /// The latest capture is stale when an eligible draft moved past it or
    /// a captured draft left eligibility (saved, closed, consent revoked).
    fn recovery_stale(&self) -> bool {
        // The verified staleness decisions (0057 VF18,
        // strop_core::cohortguard): an eligible draft whose live revision
        // moved past its capture republishes; a captured draft that
        // closed or left eligibility (a confirmed save makes it clean)
        // retires from the next cohort.
        for (id, doc) in self.docs.iter() {
            if self.recovery_doc_eligible(doc)
                && !strop_core::cohortguard::capture_is_current(
                    self.recovery
                        .captured
                        .get(&id)
                        .map(|revision| revision.get()),
                    doc.buf.revision().get(),
                )
            {
                return true;
            }
        }
        self.recovery.captured.keys().any(|id| {
            let doc = self.docs.get(*id);
            strop_core::cohortguard::retained_is_stale(
                doc.is_some(),
                doc.is_some_and(|doc| self.recovery_doc_eligible(doc)),
            )
        })
    }

    /// Coalesced capture trigger after input and job events. Cheap when
    /// nothing changed: a revision comparison per open document.
    pub(crate) fn recovery_after_event(&mut self) {
        if !self.recovery_persistent() || !self.recovery_stale() {
            return;
        }
        let publication = self.build_capture();
        self.recovery_publish(publication);
    }

    /// A confirmed save retires only the checkpoint it supersedes: the
    /// staleness rule republishes the cohort without the now-clean draft,
    /// and edits after the saved snapshot stay captured (0056 AR04 §5).
    pub(crate) fn recovery_note_saved(&mut self) {
        self.recovery_after_event();
    }

    /// Orderly close/EOF (0056 AR04 §5): input is already quiesced by the
    /// finishing flag; checkpoint eligible drafts before owned services
    /// close. Not a save: failure is reported, never auto-written.
    pub(crate) fn recovery_on_finish(&mut self) {
        if self.recovery_persistent() && self.recovery_stale() {
            let publication = self.build_capture();
            self.recovery_publish(publication);
        }
    }

    fn build_capture(&mut self) -> Publication {
        // The verified watermark step (0057 VF18): cohorts strictly increase.
        self.recovery.cohort = strop_core::cohortguard::next_cohort(self.recovery.cohort);
        let cohort = self.recovery.cohort;
        let eligible: Vec<(DocumentId, BufferRevision)> = self
            .docs
            .iter()
            .filter(|(_, doc)| self.recovery_doc_eligible(doc))
            .map(|(id, doc)| (id, doc.buf.revision()))
            .collect();
        let mut records = Vec::with_capacity(eligible.len());
        let mut captured = HashMap::with_capacity(eligible.len());
        for (id, revision) in eligible {
            let Some(doc) = self.docs.get(id) else {
                continue;
            };
            let origin = match &doc.source {
                DocumentSource::File => {
                    let Some(path) = doc.buf.path.clone() else {
                        continue;
                    };
                    DraftOrigin::LocalFile {
                        path: path.clone(),
                        canonical: doc.buf.file_identity().map(std::path::Path::to_owned),
                    }
                }
                DocumentSource::Remote(source) => DraftOrigin::RemoteFile {
                    label: source.file.to_string(),
                },
                _ => {
                    let next = self.recovery.next_scratch;
                    let scratch = *self.recovery.scratch_ids.entry(id).or_insert(next);
                    if scratch == next {
                        self.recovery.next_scratch += 1;
                    }
                    DraftOrigin::Scratch { id: scratch }
                }
            };
            let source_path = match &origin {
                DraftOrigin::LocalFile { path, .. } => Some(path.clone()),
                _ => None,
            };
            records.push(store::PendingRecord {
                document: id,
                origin,
                revision,
                source_path,
                text: doc.buf.snapshot(),
            });
            captured.insert(id, revision);
        }
        self.recovery.captured = captured;
        Publication::Capture {
            cohort,
            captured_ms: store::now_ms(),
            workspace: self.recovery_workspace(),
            records,
        }
    }

    /// The local workspace's binding epoch: registry incarnation, so a
    /// reconnect/rebind is distinguishable from the checkpoint's world.
    fn recovery_workspace(&self) -> record::WorkspaceBinding {
        let incarnation = self
            .workspaces
            .iter()
            .find(|(_, context)| context.filesystem == strop_workspace::Filesystem::Local)
            .map_or(0, |(_, context)| context.incarnation);
        record::WorkspaceBinding {
            filesystem: strop_workspace::Filesystem::Local,
            root: Some(self.cwd.clone()),
            incarnation,
        }
    }

    /// The bounded queue: one publication in flight, one queued, and the
    /// newest capture replaces the unwritten queued one.
    fn recovery_publish(&mut self, publication: Publication) {
        if self.recovery.in_flight.is_some() {
            self.recovery.queued = Some(publication);
        } else {
            self.start_recovery_publication(publication);
        }
    }

    fn start_recovery_publication(&mut self, publication: Publication) {
        let Some(base) = self.state_dir.clone() else {
            return;
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.recovery.in_flight = Some(request);
        match self.tape.request("io.recovery", &request) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                let outcome = Outcome::failed(FailureKind::Protocol, error.to_string());
                self.handle_recovery(request, outcome);
                return;
            }
        }
        let path = store::checkpoint_path(&base, &self.cwd);
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-recovery",
            move |outcome| {
                let _ = tx.send(super::io::IoEvent::Recovery { request, outcome });
            },
            move |_| match publication {
                Publication::Capture {
                    cohort,
                    captured_ms,
                    workspace,
                    records,
                } => match store::publish_cohort(&path, cohort, captured_ms, workspace, records) {
                    Ok(published) => Outcome::Success(RecoveryReport::Checkpoint(published)),
                    Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                },
                Publication::Discard { stored, index } => {
                    match store::discard(&path, &stored, index) {
                        Ok(kept) => Outcome::Success(RecoveryReport::Discarded(Box::new(kept))),
                        Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
                    }
                }
            },
        );
        self.worker_handles.insert(request, handle);
    }

    /// Recovery store reads: explicit surface/restore/discard requests.
    /// The newest intent replaces an unanswered one.
    pub(crate) fn request_recovery_load(&mut self, intent: SurfaceIntent) {
        self.recovery.intent = Some(intent);
        if !self.recovery_persistent() {
            // Memory-only is honest: there is no store to read.
            self.finish_recovery_load(None);
            return;
        }
        if self.recovery.loading.is_some() {
            return;
        }
        let Some(base) = self.state_dir.clone() else {
            self.finish_recovery_load(None);
            return;
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        self.recovery.loading = Some(request);
        match self.tape.request("io.recovery.load", &request) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                let outcome = Outcome::failed(FailureKind::Protocol, error.to_string());
                self.handle_recovery(request, outcome);
                return;
            }
        }
        let path = store::checkpoint_path(&base, &self.cwd);
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "strop-recovery-load",
            move |outcome| {
                let _ = tx.send(super::io::IoEvent::Recovery { request, outcome });
            },
            move |_| match store::load(&path) {
                Ok(stored) => Outcome::Success(RecoveryReport::Loaded(stored.map(Box::new))),
                Err(error) => Outcome::failed(FailureKind::Io, error.to_string()),
            },
        );
        self.worker_handles.insert(request, handle);
    }

    /// The io-channel completion (0056 AR04 §5): durable watermarks move
    /// only on a completed publication; failures reach status/explain.
    pub(crate) fn handle_recovery(&mut self, request: WorkerId, outcome: Outcome<RecoveryReport>) {
        if self.recovery.in_flight == Some(request) {
            self.finish_publication(request, outcome);
            return;
        }
        if self.recovery.loading == Some(request) {
            self.recovery.loading = None;
            self.worker_handles.remove(&request);
            match outcome {
                Outcome::Success(RecoveryReport::Loaded(stored)) => {
                    self.finish_recovery_load(stored.map(|stored| *stored))
                }
                Outcome::Failed { failure, .. } => {
                    self.recovery.last_error = Some(failure.message.clone());
                    // A failed read must not replay a stale restore/discard
                    // intent against the next successful load.
                    self.recovery.intent = None;
                    self.message = format!("recovery read failed: {}", failure.message);
                }
                _ => {}
            }
        }
    }

    fn finish_publication(&mut self, request: WorkerId, outcome: Outcome<RecoveryReport>) {
        self.recovery.in_flight = None;
        self.worker_handles.remove(&request);
        match outcome {
            Outcome::Success(RecoveryReport::Checkpoint(published)) => {
                self.recovery.durable = published
                    .records
                    .iter()
                    .filter(|record| record.captured)
                    .map(|record| (record.document, record.revision))
                    .collect();
                self.recovery.durable_cohort = Some((published.cohort, published.captured_ms));
                self.recovery.over_bound = published.over_bound;
                self.recovery.last_error = None;
                if self.recovery.over_bound.is_empty() {
                    self.recovery.announced = false;
                } else if !self.recovery.announced {
                    self.recovery.announced = true;
                    self.message =
                        "a draft exceeds the 16 MiB recovery limit — not durable (see :recover)"
                            .into();
                }
            }
            Outcome::Success(RecoveryReport::Discarded(kept)) => {
                self.recovery.durable.retain(|document, _| {
                    kept.header
                        .records
                        .iter()
                        .any(|record| record.document == *document)
                });
                self.recovery.loaded = Some(*kept);
                self.recovery.announced = false;
                self.message = "discarded the recovery record — its draft bytes are gone".into();
                self.reopen_recovery_surface();
            }
            Outcome::Failed { failure, .. } => {
                self.recovery.last_error = Some(failure.message.clone());
                // The failed capture is not durable: rewind the captured
                // clock to the durable one so the next event retries
                // instead of believing unsaved bytes are safe.
                self.recovery.captured = self.recovery.durable.clone();
                if !self.recovery.announced {
                    self.recovery.announced = true;
                    self.message = format!("draft checkpoint failed: {}", failure.message);
                }
            }
            _ => {}
        }
        if let Some(queued) = self.recovery.queued.take() {
            self.start_recovery_publication(queued);
        }
    }

    pub(crate) fn recovery_queue_discard(&mut self, stored: StoredCohort, index: usize) {
        self.recovery_publish(Publication::Discard {
            stored: Box::new(stored),
            index,
        });
    }

    pub(crate) fn recovery_set_remote_consent(&mut self, granted: bool) {
        self.recovery.consent_remote = granted;
        // Granting makes remote drafts eligible; revoking retires them
        // from the next published cohort.
        self.recovery_after_event();
    }

    pub fn recovery_status(&self) -> RecoveryStatus {
        RecoveryStatus {
            memory_only: self.recovery_memory_only(),
            durable_cohort: self.recovery.durable_cohort,
            durable_records: self.recovery.durable.len(),
            in_flight: self.recovery.in_flight.is_some(),
            queued: self.recovery.queued.is_some(),
            consent_remote: self.recovery.consent_remote,
            over_bound: self.recovery.over_bound.clone(),
            last_error: self.recovery.last_error.clone(),
        }
    }
}
