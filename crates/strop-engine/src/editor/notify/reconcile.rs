//! Applying drained records to documents: hint application, reload
//! admission/guarding, and guarded reload completion.
use std::path::PathBuf;

use strop_core::id::DocumentId;
use strop_core::worker::{self, Completion, Outcome};
use strop_fs::notify::NotifyHint;
use strop_workspace::Filesystem;

use super::super::directory::DirectoryTask;
use super::super::io::{IoEvent, Opened};
use super::super::DocumentSource;
use super::super::Editor;
use super::subscribe::reload_run;
use super::{Record, ReloadKey, SubscribedScope};

impl Editor {
    /// Drain the notify queue and apply what it recorded. Settled
    /// records apply first (subscribe completion may race hints across
    /// the job/forwarder threads); hints stamped by a superseded
    /// identity are dropped, never acted on.
    pub(crate) fn handle_notify(&mut self) {
        // The wake is not the observation: the bounded queue may hold a
        // settled refusal or coalesced overflow. Replay has no watcher
        // thread to refill it, so consume the same typed drain in both
        // modes rather than guessing from AppEvent::Notify.
        let (records, queue_rescan) = match self
            .tape
            .observe_owned("notify.drain", &(), || self.notify.queue.drain())
        {
            Ok(drained) => drained,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        if records.is_empty() && !queue_rescan {
            return;
        }
        let mut deferred: Vec<Record> = Vec::new();
        let mut rescan = queue_rescan;
        let mut hints: Vec<NotifyHint> = Vec::new();
        // Lifecycle applies first: the settle may race hints, and the
        // last settle wins.
        let (settles, rest): (Vec<_>, Vec<_>) = records
            .into_iter()
            .partition(|record| matches!(record, Record::Settled(_)));
        for record in settles {
            if let Record::Settled(outcome) = record {
                self.notify_settled(outcome);
            }
        }
        // Pass 2: identity-checked application: events stamped by a
        // superseded identity never act.
        for record in rest {
            let record_subscription = match &record {
                Record::Hints { subscription, .. } | Record::Overflow { subscription } => {
                    *subscription
                }
                Record::Boundary { subscription, .. } => *subscription,
                Record::Settled(_) => continue,
            };
            if Some(record_subscription) == self.notify.subscription {
                match record {
                    Record::Hints { hints: batch, .. } => hints.extend(batch),
                    Record::Overflow { .. } => rescan = true,
                    // The reconcile boundary's ordering guarantee lives in
                    // the consumers' snapshot semantics (picker cache) and
                    // the reload observation guards; the mark itself needs
                    // no editor-side state.
                    Record::Boundary { .. } | Record::Settled(_) => {}
                }
                continue;
            }
            if self.notify.subscription.is_none() && self.notify.subscribing {
                // The local subscribe settle has not landed yet; defer so
                // the raced hints apply in order once identity exists.
                deferred.push(record);
            }
            // Otherwise: a dead generation's late events never act.
        }
        if !deferred.is_empty() {
            self.notify.queue.requeue_front(deferred);
        }
        if rescan {
            self.notify_rescan();
            return;
        }
        if !hints.is_empty() {
            self.apply_notify_hints(hints);
        }
    }

    /// The subscription settled: adopt the identity and arm the search
    /// baseline's push coverage. A watcher that later fails settles
    /// again with the failure: coverage is dropped and the baseline is
    /// rescanned, never trusted stale.
    fn notify_settled(&mut self, outcome: Outcome<SubscribedScope>) {
        self.notify.subscribing = false;
        match outcome {
            Outcome::Success(settled) => {
                self.notify.subscription = Some(settled.subscription);
                self.notify
                    .queue
                    .set_owned_trace(settled.owned_trace.map(|path| (settled.subscription, path)));
            }
            Outcome::Failed { failure, .. } => {
                let watched = self.notify.subscription.take().is_some();
                self.notify.queue.set_owned_trace(None);
                self.message = format!(
                    "filesystem notifications unavailable: {}; freshness is on demand",
                    failure.message
                );
                if watched {
                    self.notify_rescan();
                }
            }
            Outcome::Cancelled(_) => {}
        }
        if let Some(source) = self.picker_source.as_ref() {
            source.set_watching(&self.cwd, self.notify.push_coverage());
        }
    }

    /// Hints against open state: dirty documents gain external-change
    /// state (never clobbered), clean documents reload through the
    /// guarded job, Directory buffers reobserve, Git signs refresh
    /// lazily and the search baseline invalidates the hinted subtrees.
    fn apply_notify_hints(&mut self, hints: Vec<NotifyHint>) {
        use std::os::unix::ffi::OsStrExt;
        let mut picker_paths: Vec<PathBuf> = Vec::new();
        let mut reload: Vec<DocumentId> = Vec::new();
        let mut dirty: Vec<DocumentId> = Vec::new();
        let mut directories: Vec<DocumentId> = Vec::new();
        let mut git_touched = false;
        for hint in hints {
            let relative = PathBuf::from(std::ffi::OsStr::from_bytes(&hint.path));
            if relative.as_os_str().is_empty() {
                // The scope root itself moved/was replaced (parent/name
                // guard): the whole baseline is suspect.
                self.notify_rescan();
                return;
            }
            picker_paths.push(relative.clone());
            let absolute = self.cwd.join(&relative);
            if relative
                .components()
                .any(|component| component.as_os_str() == ".git")
            {
                git_touched = true;
            }
            let canonical = std::fs::canonicalize(&absolute).unwrap_or_else(|_| absolute.clone());
            for (id, document) in self.docs.iter() {
                match &document.source {
                    DocumentSource::File => {
                        let bound = document.buf.path.as_ref() == Some(&absolute)
                            || document.buf.file_identity() == Some(absolute.as_path())
                            || document.buf.file_identity() == Some(canonical.as_path());
                        if !bound {
                            continue;
                        }
                        if Some(id) == Some(self.current()) {
                            git_touched = true;
                        }
                        if document.buf.is_dirty() {
                            if !dirty.contains(&id) {
                                dirty.push(id);
                            }
                        } else if !reload.contains(&id) {
                            reload.push(id);
                        }
                    }
                    DocumentSource::Directory(directory) => {
                        if !matches!(directory.location.filesystem, Filesystem::Local) {
                            continue;
                        }
                        let affects_listing = absolute == directory.location.path
                            || absolute.parent() == Some(directory.location.path.as_path());
                        if affects_listing && !directories.contains(&id) {
                            directories.push(id);
                        }
                    }
                    _ => {}
                }
            }
        }
        for id in dirty {
            self.notify_mark_external(id);
        }
        for id in reload {
            self.request_notify_reload(id);
        }
        for id in directories {
            let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
        }
        if git_touched {
            self.cancel_hunk_owner();
        }
        // any change under the root may be a change to the tree's status
        self.refresh_status_buffers();
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &picker_paths, false);
        }
    }

    /// The editor's own git verbs rewrote these files (a discard): each
    /// open buffer on one gets what a watcher hint would give it — a
    /// guarded reload when clean, external-change state when dirty. A
    /// directory covers everything under it.
    pub(crate) fn files_rewritten(&mut self, paths: &[PathBuf]) {
        let open: Vec<(DocumentId, bool)> = self
            .docs
            .iter()
            .filter(|(_, document)| {
                matches!(document.source, DocumentSource::File)
                    && document.buf.path.as_ref().is_some_and(|path| {
                        paths.iter().any(|rewritten| path.starts_with(rewritten))
                    })
            })
            .map(|(id, document)| (id, document.buf.is_dirty()))
            .collect();
        for (id, dirty) in open {
            if dirty {
                self.notify_mark_external(id);
            } else {
                self.request_notify_reload(id);
            }
        }
    }

    /// A conservative rescan obligation (overflow, queue bound, root
    /// replacement, lease loss): invalidate the whole affected baseline
    /// and reobserve before freshness is claimed again.
    pub(crate) fn notify_rescan(&mut self) {
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &[], true);
        }
        self.cancel_hunk_owner();
        self.refresh_status_buffers();
        let documents: Vec<DocumentId> = self.docs.iter().map(|(id, _)| id).collect();
        for id in documents {
            let source_matches = {
                let document = self.doc(id);
                match &document.source {
                    DocumentSource::File => document
                        .buf
                        .path
                        .as_ref()
                        .is_some_and(|path| path.starts_with(&self.cwd)),
                    DocumentSource::Directory(directory) => {
                        matches!(directory.location.filesystem, Filesystem::Local)
                            && directory.location.path.starts_with(&self.cwd)
                    }
                    _ => false,
                }
            };
            if !source_matches {
                continue;
            }
            if matches!(self.doc(id).source, DocumentSource::Directory(_)) {
                let _ = self.start_directory_task(id, DirectoryTask::Reload, None);
            } else if self.doc(id).buf.is_dirty() {
                self.notify_mark_external(id);
            } else {
                self.request_notify_reload(id);
            }
        }
    }

    /// A dirty (or vanished-from-under-clean) document gains
    /// external-change state; the buffer and its edits are preserved.
    pub(super) fn notify_mark_external(&mut self, document: DocumentId) {
        let Some(doc) = self.docs.get_mut(document) else {
            return;
        };
        if doc.external_change {
            return;
        }
        doc.external_change = true;
        let label = doc.label(&self.cwd);
        self.message =
            format!("{label}: changed on disk; the buffer keeps your edits (:w! forces)");
    }

    /// Spawn the guarded reload job for one clean local document.
    /// Re-hints during flight record one owed re-observation.
    fn request_notify_reload(&mut self, document: DocumentId) {
        if self.finishing {
            return;
        }
        if self.notify.reloading.contains_key(&document) {
            self.notify.reload_again.insert(document);
            return;
        }
        let Some(document_ref) = self.docs.get(document) else {
            return;
        };
        if document_ref.buf.is_dirty() || !matches!(document_ref.source, DocumentSource::File) {
            return;
        }
        let Some(path) = document_ref.buf.path.clone().or_else(|| {
            document_ref
                .buf
                .file_identity()
                .map(std::path::Path::to_path_buf)
        }) else {
            return;
        };
        let key = ReloadKey {
            document,
            revision: document_ref.buf.revision(),
            path,
        };
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let ticket = worker::Ticket {
            request,
            key: key.clone(),
        };
        let path = key.path.clone();
        self.notify.reloads.insert(request, key);
        self.notify.reloading.insert(document, request);
        let local = self.filesystem.local().clone();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "fs-notify-reload",
            move |outcome| {
                let _ = tx.send(IoEvent::NotifyReload(Box::new(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |cancel| reload_run(&local, &path, &cancel),
        );
        self.worker_handles.insert(request, handle);
    }

    /// Guarded publication (PUBLICATION): only a reload whose document,
    /// binding and revision still match, whose buffer is still clean,
    /// and whose fresh observation differs from the buffer's baseline
    /// may publish. A vanished file marks external change; a stale
    /// in-flight reload never clears newer edits.
    pub(crate) fn notify_reload_done(&mut self, completion: Completion<ReloadKey, Option<Opened>>) {
        let request = completion.ticket.request;
        if self.notify.reloads.get(&request) != Some(&completion.ticket.key) {
            return;
        }
        self.notify.reloads.remove(&request);
        self.worker_handles.remove(&request);
        let key = completion.ticket.key;
        self.notify.reloading.remove(&key.document);
        let reobserve = self.notify.reload_again.remove(&key.document);
        let Some(document) = self.docs.get(key.document) else {
            return;
        };
        let still_bound = matches!(document.source, DocumentSource::File)
            && (document.buf.path.as_ref() == Some(&key.path)
                || document.buf.file_identity() == Some(key.path.as_path()));
        if !still_bound {
            return; // the binding moved on (save-as, operation receipt)
        }
        if document.buf.revision() != key.revision || document.buf.is_dirty() {
            // Edits landed while the job was in flight: the snapshot is
            // stale and never publishes. A dirty buffer takes
            // external-change state instead.
            if document.buf.is_dirty() {
                self.notify_mark_external(key.document);
            }
            return;
        }
        match completion.outcome {
            Outcome::Success(Some(opened)) => {
                let observed_stamp = opened.document.buf.disk_stamp();
                if observed_stamp == document.buf.disk_stamp() {
                    // The baseline already covers this observation — an
                    // own-save receipt or a duplicate hint. Nothing to do.
                } else {
                    let binding = strop_core::Buffer::from_read(
                        opened
                            .document
                            .buf
                            .path
                            .clone()
                            .unwrap_or_else(|| key.path.clone()),
                        ropey::Rope::new(),
                        observed_stamp,
                        opened
                            .document
                            .buf
                            .file_identity()
                            .map(std::path::Path::to_path_buf)
                            .unwrap_or_else(|| key.path.clone()),
                        true,
                    );
                    let label = self.doc(key.document).label(&self.cwd);
                    let readonly = self.doc(key.document).buf.readonly;
                    let mut replacement = opened.document;
                    replacement.buf.readonly = readonly;
                    match self.publish_source_snapshot(key.document, replacement) {
                        Ok(()) => {
                            {
                                let mut doc = self.doc_mut(key.document);
                                doc.buf.adopt_file_binding(&binding);
                                doc.external_change = false;
                            }
                            self.message = format!("{label}: reloaded — changed on disk");
                        }
                        Err(error) => {
                            self.message = format!("{label}: reload failed: {error}");
                        }
                    }
                }
            }
            Outcome::Success(None) => {
                // Vanished or no longer a regular file: preserve the
                // buffer, surface the state, never clobber.
                self.notify_mark_external(key.document);
            }
            Outcome::Failed { .. } | Outcome::Cancelled(_) => {}
        }
        if reobserve
            && self
                .docs
                .get(key.document)
                .is_some_and(|document| !document.buf.is_dirty())
        {
            self.request_notify_reload(key.document);
        }
    }
}
