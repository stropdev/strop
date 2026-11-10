//! The TUI recovery surface (0056 AR04 §5): a real readonly buffer showing
//! the durable checkpoint's records — original location, checkpoint
//! time/revision, source comparison and conflict/missing/renamed state —
//! plus restore into a checked draft and deliberate discard. Restoring
//! never writes disk content, never recreates a deleted target and never
//! re-grants remote authority; saving the draft goes through the existing
//! source-admission/conflict paths.

use super::offer::{captured_len, Choice, Inherited, Offer, RecoveryOffer};
use super::record::{DraftOrigin, DraftRecord, Snapshot};
use super::{Editor, SurfaceIntent};
use crate::editor::Document;
use std::fmt::Write;
use strop_core::Buffer;

/// One row's recovery state: the source comparison the plan requires.
#[derive(Debug, Clone, PartialEq, Eq)]
enum RowState {
    /// The source is unchanged since the checkpoint's observation.
    Current,
    /// The source changed after the checkpoint's observation.
    Conflict { disk_len: u64 },
    /// The source is gone; restoring never recreates it.
    Missing,
    /// A live rename moved the binding (AR05); the row names the target.
    Renamed { current: String },
    /// Remote drafts restore as local checked drafts.
    Remote,
    /// Scratch drafts have no filesystem target.
    Scratch,
}

impl Editor {
    /// `:recover …` — list / restore N / discard N / consent remote|off.
    pub(crate) fn run_recovery_ex(&mut self, arg: &str) {
        let (sub, rest) = arg.split_once(' ').unwrap_or((arg, ""));
        let (sub, rest) = (sub.trim(), rest.trim());
        match sub {
            "" | "list" => self.request_recovery_load(SurfaceIntent::List),
            "restore" => match rest.parse::<usize>() {
                Ok(n) if n >= 1 => self.request_recovery_load(SurfaceIntent::Restore(n - 1)),
                _ => {
                    self.message =
                        ":recover restore N — the record number from :recover".into()
                }
            },
            "discard" => match rest.parse::<usize>() {
                Ok(n) if n >= 1 => self.request_recovery_load(SurfaceIntent::Discard(n - 1)),
                _ => {
                    self.message =
                        ":recover discard N — the record number from :recover".into()
                }
            },
            "consent" => match rest {
                "remote" => {
                    self.recovery_set_remote_consent(true);
                    self.message = "remote draft persistence consented for this session".into();
                }
                "off" => {
                    self.recovery_set_remote_consent(false);
                    self.message = "remote draft persistence consent revoked".into();
                }
                _ => {
                    self.message =
                        ":recover consent remote|off — session-scoped, never from project config"
                            .into()
                }
            },
            _ => {
                self.message =
                    ":recover [list] · :recover restore N · :recover discard N · :recover consent remote|off"
                        .into()
            }
        }
    }

    /// The load worker's completion hands the fresh store contents here;
    /// every surface action is read-before-act.
    pub(crate) fn finish_recovery_load(&mut self, stored: Option<super::StoredCohort>) {
        let intent = self.recovery.intent.take().unwrap_or(SurfaceIntent::List);
        if std::mem::take(&mut self.recovery.reading_prior) {
            self.recovery.prior = stored.clone().map(Inherited::adopt).unwrap_or_default();
            // the file the session opened with may have been waiting
            self.recovery_offer_for(self.current());
            self.recovery_after_event();
        }
        self.recovery.loaded = stored;
        match intent {
            SurfaceIntent::Startup => {}
            SurfaceIntent::List => self.open_recovery_surface(),
            SurfaceIntent::Restore(index) => self.recovery_restore(index),
            SurfaceIntent::Discard(index) => self.recovery_discard(index),
        }
    }

    /// The record's current state against the live editor and the disk.
    fn recovery_row_state(&self, record: &DraftRecord) -> (RowState, Vec<String>) {
        let mut notes = Vec::new();
        // an earlier session's document ids name nothing live here
        let inherited = self
            .recovery
            .prior
            .iter()
            .any(|draft| draft.same_draft(record));
        if let Some(live) = self.docs.get(record.document).filter(|_| !inherited) {
            if live.buf.revision() > record.revision {
                notes.push(format!(
                    "newer unsaved edits (r{}) since the checkpoint",
                    live.buf.revision()
                ));
            }
            if let (DraftOrigin::LocalFile { path, .. }, Some(current)) =
                (&record.origin, live.buf.path.as_ref())
            {
                if current != path {
                    return (
                        RowState::Renamed {
                            current: current.display().to_string(),
                        },
                        notes,
                    );
                }
            }
        }
        let state = match &record.origin {
            DraftOrigin::Scratch { .. } => RowState::Scratch,
            DraftOrigin::RemoteFile { .. } => RowState::Remote,
            DraftOrigin::LocalFile { path, .. } => match std::fs::metadata(path) {
                Ok(metadata) => {
                    let observed = record.source;
                    let now = super::record::SourceObservation {
                        modified_ms: metadata.modified().map_or(0, super::record::millis),
                        len: metadata.len(),
                    };
                    if observed == Some(now) {
                        RowState::Current
                    } else {
                        RowState::Conflict {
                            disk_len: metadata.len(),
                        }
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => RowState::Missing,
                Err(error) => {
                    notes.push(format!("source check failed: {error}"));
                    RowState::Missing
                }
            },
        };
        (state, notes)
    }

    /// Restore record `index` into a checked draft: a pathless dirty
    /// scratch buffer with the checkpoint's bytes. Saving it is the user's
    /// explicit act through `:w {path}` and its existing conflict rules;
    /// nothing here writes, recreates or reconnects anything.
    fn recovery_restore(&mut self, index: usize) {
        let restored = self.recovery.loaded.as_ref().and_then(|cohort| {
            let record = cohort.header.records.get(index)?;
            match &record.snapshot {
                Snapshot::OverBound { bytes } => {
                    return Some(Err(format!(
                        "record #{} is {bytes} bytes, over the capture limit — nothing durable to restore",
                        index + 1
                    )));
                }
                Snapshot::Captured { .. } => {}
            }
            let bytes = cohort.texts.get(index)?.as_ref()?;
            let text = match std::str::from_utf8(bytes) {
                Ok(text) => text.to_owned(),
                Err(_) => {
                    return Some(Err(format!(
                        "record #{} is not valid UTF-8 — refusing to restore",
                        index + 1
                    )));
                }
            };
            Some(Ok((record.clone(), text)))
        });
        let Some(restored) = restored else {
            self.message = format!(
                "no recovery record #{} — :recover lists the durable checkpoint",
                index + 1
            );
            return;
        };
        let (record, text) = match restored {
            Ok(restored) => restored,
            Err(message) => {
                self.message = message;
                return;
            }
        };
        let (revision, label) = (record.revision, record.origin.label());
        let mut buffer = Buffer::from_text(&text);
        buffer.mark_dirty();
        buffer.name = Some(format!("recovered: {label}"));
        let Ok(id) = self.docs.try_insert(Document::scratch(buffer)) else {
            // The durable checkpoint is untouched; the user can retry
            // after closing documents.
            self.message = format!(
                "checkpoint r{revision} survived, but the document identity space is exhausted — close buffers and :recover again"
            );
            return;
        };
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.set_head(0);
        self.forget_prior_draft(&record);
        self.message = format!(
            "restored checkpoint r{revision} into a checked draft — the source is untouched; :w {{path}} saves through the normal admission path"
        );
    }

    /// A deliberate discard (0056 AR04 §5): explicit user intent, not
    /// transport/window loss — the republication drops the record's bytes.
    fn recovery_discard(&mut self, index: usize) {
        let Some(cohort) = self.recovery.loaded.take() else {
            self.message = "no durable recovery checkpoint to discard from".into();
            return;
        };
        if index >= cohort.header.records.len() {
            let total = cohort.header.records.len();
            self.recovery.loaded = Some(cohort);
            self.message = format!(
                "no recovery record #{} — the durable checkpoint has {total} record(s)",
                index + 1
            );
            return;
        }
        self.recovery_queue_discard(cohort, index);
    }

    /// After a discard lands, the open surface's data changed: refresh it.
    pub(crate) fn reopen_recovery_surface(&mut self) {
        self.open_recovery_surface();
    }

    /// Render the recovery surface as a real readonly buffer (0001 §4).
    pub(crate) fn open_recovery_surface(&mut self) {
        let status = self.recovery_status();
        let mut text = String::from("strop recovery — durable draft checkpoints (0056 AR04)\n\n");
        if status.memory_only {
            text.push_str("policy: memory-only — drafts are NOT durable; nothing is persisted\n");
        } else {
            let _ = writeln!(
                text,
                "policy: automatic — private state storage (remote drafts: {})",
                if status.consent_remote {
                    "consent granted for this session"
                } else {
                    "not persisted without :recover consent remote"
                }
            );
        }
        match status.durable_cohort {
            Some((cohort, ms)) => {
                let _ = writeln!(
                    text,
                    "durable checkpoint: cohort {cohort} captured at {ms} ({} draft(s))",
                    status.durable_records
                );
            }
            None => text.push_str("durable checkpoint: none completed this session\n"),
        }
        if status.in_flight || status.queued {
            text.push_str("checkpoint: publication in progress\n");
        }
        if !status.over_bound.is_empty() {
            let _ = writeln!(
                text,
                "over the 16 MiB capture limit: {} draft(s) not durable",
                status.over_bound.len()
            );
        }
        if let Some(error) = &status.last_error {
            let _ = writeln!(text, "last persistence failure: {error}");
        }

        text.push_str("\n[records]\n");
        match &self.recovery.loaded {
            None => text.push_str("  no durable checkpoint\n"),
            Some(cohort) if cohort.header.records.is_empty() => {
                text.push_str("  the durable checkpoint holds no drafts\n")
            }
            Some(cohort) => {
                let _ = writeln!(
                    text,
                    "  cohort {} captured at {} — workspace {} (binding epoch {})\n",
                    cohort.header.cohort,
                    cohort.header.captured_ms,
                    cohort.header.workspace.filesystem.label(),
                    cohort.header.workspace.incarnation
                );
                let rows: Vec<String> = cohort
                    .header
                    .records
                    .iter()
                    .enumerate()
                    .map(|(index, record)| self.recovery_row(index, record))
                    .collect();
                for row in rows {
                    text.push_str(&row);
                }
            }
        }

        let pending: Vec<String> = self
            .docs
            .iter()
            .filter(|(id, doc)| {
                self.recovery_doc_eligible(doc)
                    && self.recovery.durable.get(id) != Some(&doc.buf.revision())
            })
            .map(|(_, doc)| {
                format!(
                    "  - {} r{} (checkpoint pending)\n",
                    doc.label(&self.cwd),
                    doc.buf.revision()
                )
            })
            .collect();
        if !pending.is_empty() {
            text.push_str("\nlive drafts without a durable checkpoint:\n");
            for row in pending {
                text.push_str(&row);
            }
        }

        text.push_str(
            "\ncommands: :recover restore N — checked draft, source untouched · :recover discard N — deliberate, drops the bytes · :recover consent remote|off\n",
        );
        let mut buffer = Buffer::from_text(&text);
        buffer.name = Some("recovery".into());
        let Some(document) = self.open_temporary_output(buffer) else {
            return; // message set; the checkpoint stays durable
        };
        // set_readonly keeps the first owner; the checkpoint surface is a
        // more specific owner than the generic output surface.
        let buf = &mut self.doc_mut(document).buf;
        buf.clear_readonly();
        buf.set_readonly(strop_core::ReadonlyReason::RecoveryCheckpoint);
    }

    /// Startup: read the drafts an earlier session left. Publications wait
    /// for this read so they cannot overwrite them first.
    pub(crate) fn recovery_read_prior(&mut self) {
        if !self.recovery_persistent() {
            return;
        }
        self.recovery.reading_prior = true;
        self.request_recovery_load(SurfaceIntent::Startup);
    }

    /// A file just opened (or the startup read just landed): offer its
    /// earlier-session draft, vim `ATTENTION` style.
    pub(crate) fn recovery_offer_for(&mut self, document: strop_core::id::DocumentId) {
        if self.recovery.offer.is_some() || self.docs.is_empty() || self.current() != document {
            return;
        }
        let doc = self.doc(document);
        if !matches!(doc.source, crate::editor::DocumentSource::File) || doc.buf.is_dirty() {
            return;
        }
        let Some(path) = doc.buf.path.as_deref() else {
            return;
        };
        let canonical = doc.buf.file_identity();
        let Some(draft) = self.recovery.prior.iter().position(|draft| {
            !draft.declined && draft.text.is_some() && draft.is_for(path, canonical)
        }) else {
            return;
        };
        // Compare the observation the draft's session made with the bytes
        // just read — no extra stat on the input path.
        let observed = self.recovery.prior[draft].record.source;
        let source_changed = observed.is_none_or(|observed| {
            observed.len != doc.buf.len_bytes() as u64
                || doc.buf.disk_stamp().map(super::record::millis) != Some(observed.modified_ms)
        });
        self.recovery.offer = Some(Offer {
            document,
            draft,
            source_changed,
        });
    }

    /// The offer card's content, while one is open.
    pub fn recovery_offer(&self) -> Option<RecoveryOffer> {
        let offer = self.recovery.offer?;
        let draft = self.recovery.prior.get(offer.draft)?;
        let path = match &draft.record.origin {
            DraftOrigin::LocalFile { path, .. } => path
                .strip_prefix(&self.cwd)
                .unwrap_or(path)
                .display()
                .to_string(),
            origin => origin.label(),
        };
        Some(RecoveryOffer {
            path,
            revision: draft.record.revision.get(),
            bytes: captured_len(&draft.record),
            captured_ms: draft.captured_ms,
            source_changed: offer.source_changed,
        })
    }

    /// The offer owns every key until answered; other keys are refused
    /// with the choices, so nothing edits the file behind the question.
    pub(crate) fn feed_recovery_offer(&mut self, key: strop_core::frontend_input::Key) {
        let Some(offer) = self.recovery.offer else {
            return;
        };
        let Some(choice) = Choice::from_key(key) else {
            self.message =
                "a recovery draft exists: r recover · o read-only · d delete draft · e edit anyway"
                    .into();
            return;
        };
        self.recovery.offer = None;
        let live = self.docs.get(offer.document).is_some();
        match choice {
            Choice::Recover if live => self.recover_into(offer),
            Choice::ReadOnly if live => {
                self.recovery.prior[offer.draft].declined = true;
                self.doc_mut(offer.document)
                    .buf
                    .set_readonly(strop_core::ReadonlyReason::Command);
                self.message =
                    "opened read-only — the draft is kept; :recover lists it, :set noro edits"
                        .into();
            }
            Choice::Delete => {
                let record = self.recovery.prior[offer.draft].record.clone();
                self.forget_prior_draft(&record);
                self.message = "deleted the recovery draft — its bytes are gone".into();
            }
            _ => {
                self.recovery.prior[offer.draft].declined = true;
                self.message =
                    "editing the file on disk — the draft is kept; :recover lists it".into();
            }
        }
    }

    /// Recover: the draft replaces the opened text as ONE undoable edit,
    /// so `u` returns to the file on disk. Nothing is written; `:w` saves
    /// through the normal admission path.
    fn recover_into(&mut self, offer: Offer) {
        let Some(text) = self.recovery.prior[offer.draft].text.clone() else {
            return;
        };
        let record = self.recovery.prior[offer.draft].record.clone();
        self.switch_to(offer.document);
        {
            let mut buf = self.buf_mut();
            let end = buf.len_bytes();
            buf.begin_undo_group();
            buf.delete(strop_core::Range::charwise(0, end));
            buf.insert(0, &text.to_string());
            buf.commit_undo_group();
        }
        self.set_head(0);
        self.clamp_cursor();
        self.forget_prior_draft(&record);
        self.message = if offer.source_changed {
            "recovered the draft — the file changed on disk after it was saved; check before :w (u undoes)"
        } else {
            "recovered the draft — :w saves it, u returns to the file on disk"
        }
        .into();
    }

    /// An earlier-session draft is resolved (recovered or deleted): drop
    /// it from every later publication, and republish now so a delete is
    /// real even if nothing else changes this session.
    fn forget_prior_draft(&mut self, record: &DraftRecord) {
        let before = self.recovery.prior.len();
        self.recovery
            .prior
            .retain(|draft| !draft.same_draft(record));
        if self.recovery.prior.len() != before {
            self.recovery.prior_changed = true;
            self.recovery_after_event();
        }
    }

    fn recovery_row(&self, index: usize, record: &DraftRecord) -> String {
        let (state, notes) = self.recovery_row_state(record);
        let label = record.origin.label();
        let revision = record.revision;
        let mut row = match (&record.snapshot, &state) {
            (Snapshot::OverBound { bytes }, _) => format!(
                "  {}  [over-bound] {label} — r{revision}, {bytes} B over the 16 MiB limit, not durable\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Current) => format!(
                "  {}  [current]   {label} — r{revision}, {text_len} B, source unchanged since the checkpoint\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Conflict { disk_len }) => format!(
                "  {}  [conflict]  {label} — r{revision}, {text_len} B, source changed after the checkpoint (disk {disk_len} B); restore drafts, never overwrites\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Missing) => format!(
                "  {}  [missing]   {label} — r{revision}, {text_len} B, source gone; restore drafts without a target, never recreates\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Renamed { current }) => format!(
                "  {}  [renamed]   {label} → {current} — r{revision}, {text_len} B\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Remote) => format!(
                "  {}  [remote]    {label} — r{revision}, {text_len} B; restore creates a local draft, reconnecting needs fresh authority\n",
                index + 1
            ),
            (Snapshot::Captured { text_len }, RowState::Scratch) => format!(
                "  {}  [scratch]   {label} — r{revision}, {text_len} B\n",
                index + 1
            ),
        };
        for note in notes {
            let _ = writeln!(row, "      {note}");
        }
        row
    }
}
