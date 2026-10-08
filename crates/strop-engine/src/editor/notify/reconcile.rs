//! Applying drained records to documents: hint application, reload
//! admission/guarding, the worker lease observation, and guarded reload
//! completion.
use std::path::PathBuf;

use strop_core::id::DocumentId;
use strop_core::worker::{self, Completion, Outcome};
use strop_worker_protocol::message::NotifyCoverage;
use strop_worker_protocol::NotifyHint;
use strop_workspace::{Filesystem, ResourceLocation};

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
        // settled refusal or coalesced overflow. Replay has no worker
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
        // Lifecycle applies first: a subscribe settle may race hints
        // across the job/forwarder threads, and the last settle wins.
        let (settles, rest): (Vec<_>, Vec<_>) = records.into_iter().partition(|record| {
            matches!(record, Record::Settled(_) | Record::RemoteSettled { .. })
        });
        for record in settles {
            match record {
                Record::Settled(outcome) => self.notify_settled(outcome),
                Record::RemoteSettled { root, outcome } => {
                    self.remote_notify_settled(root, outcome)
                }
                _ => {}
            }
        }
        // Pass 2: identity-checked application. A record's subscription
        // names its owning scope — the local one or one remote root —
        // and events stamped by a superseded identity never act.
        let mut remote_hints: Vec<(ResourceLocation, Vec<NotifyHint>)> = Vec::new();
        let mut remote_rescans: Vec<ResourceLocation> = Vec::new();
        for record in rest {
            let record_subscription = match &record {
                Record::Hints { subscription, .. } | Record::Overflow { subscription } => {
                    *subscription
                }
                Record::Boundary { subscription, .. } => *subscription,
                Record::Settled(_) | Record::RemoteSettled { .. } => continue,
            };
            if Some(record_subscription) == self.notify.subscription {
                match record {
                    Record::Hints { hints: batch, .. } => hints.extend(batch),
                    Record::Overflow { .. } => rescan = true,
                    // The reconcile boundary's ordering guarantee lives in
                    // the consumers' snapshot semantics (picker cache) and
                    // the reload observation guards; the mark itself needs
                    // no editor-side state.
                    Record::Boundary { .. } => {}
                    Record::Settled(_) | Record::RemoteSettled { .. } => {}
                }
                continue;
            }
            let remote_root = self.notify.remote.iter().find_map(|(root, scope)| {
                (scope.subscription == Some(record_subscription)).then(|| root.clone())
            });
            if let Some(root) = remote_root {
                match record {
                    Record::Hints { hints: batch, .. } => {
                        match remote_hints.iter_mut().find(|(known, _)| *known == root) {
                            Some((_, batch_hints)) => batch_hints.extend(batch),
                            None => remote_hints.push((root, batch)),
                        }
                    }
                    Record::Overflow { .. } if !remote_rescans.contains(&root) => {
                        remote_rescans.push(root);
                    }
                    Record::Overflow { .. } => {}
                    _ => {}
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
        for root in remote_rescans {
            self.remote_notify_rescan(&root.path.clone());
        }
        for (root, batch) in remote_hints {
            self.apply_remote_hints(&root.path.clone(), batch);
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

    /// The subscribe job settled: adopt the identity, surface coverage
    /// honestly, and arm the search baseline's push coverage.
    fn notify_settled(&mut self, outcome: Outcome<SubscribedScope>) {
        self.notify.subscribing = false;
        self.notify.subscribe_handle = None;
        match outcome {
            Outcome::Success(settled) => {
                self.notify.subscription = Some(settled.subscription);
                self.notify
                    .queue
                    .set_owned_trace(settled.owned_trace.map(|path| (settled.subscription, path)));
                self.notify.session = self.filesystem.worker().session();
                self.notify.coverage = Some(settled.coverage);
                if matches!(
                    settled.coverage,
                    NotifyCoverage::OnDemand | NotifyCoverage::Unsupported
                ) {
                    // CoverageHonest: a visible-but-quiet note, and the
                    // search baseline keeps per-search scanning.
                    self.message = "filesystem notifications unavailable in this namespace; freshness is on demand".into();
                }
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!(
                    "filesystem notifications refused: {}; freshness is on demand",
                    failure.message
                );
            }
            Outcome::Cancelled(_) => {}
        }
        if let Some(source) = self.picker_source.as_ref() {
            source.set_watching(&self.cwd, self.notify.push_coverage());
        }
    }

    /// The remote subscribe job settled: adopt the scope's identity,
    /// surface coverage honestly, never guess state.
    fn remote_notify_settled(&mut self, root: ResourceLocation, outcome: Outcome<SubscribedScope>) {
        let endpoint = self
            .notify
            .remote
            .get(&root)
            .map(|scope| scope.endpoint.clone());
        let Some(endpoint) = endpoint else {
            return;
        };
        match outcome {
            Outcome::Success(settled) => {
                let session = self
                    .remote
                    .workers
                    .get(&endpoint)
                    .and_then(|worker| worker.worker().session());
                if let Some(scope) = self.notify.remote.get_mut(&root) {
                    scope.subscribing = false;
                    scope.subscribe_handle = None;
                    scope.subscription = Some(settled.subscription);
                    scope.session = session;
                }
                if matches!(
                    settled.coverage,
                    NotifyCoverage::OnDemand | NotifyCoverage::Unsupported
                ) {
                    self.message = format!(
                        "filesystem notifications for {} are on demand in this namespace",
                        root.label()
                    );
                }
            }
            Outcome::Failed { failure, .. } => {
                self.notify.remote.remove(&root);
                self.message = format!(
                    "filesystem notifications refused for {}: {}; freshness is on demand",
                    root.label(),
                    failure.message
                );
            }
            Outcome::Cancelled(_) => {
                if let Some(scope) = self.notify.remote.get_mut(&root) {
                    scope.subscribing = false;
                    scope.subscribe_handle = None;
                }
            }
        }
    }

    /// Subscribe every opened remote workspace root whose endpoint
    /// admits a worker (WK07). Browsing never deploys: only an existing
    /// admitted lease subscribes, and the round trip rides a job — the
    /// settle lands on the notify queue like the local scope's.
    pub(crate) fn notify_observe_lease(&mut self) {
        self.observe_remote_leases();
        let Some(session) = self.notify.session else {
            return;
        };
        if self.filesystem.worker().session() == Some(session) {
            return;
        }
        self.notify.subscription = None;
        self.notify.queue.set_owned_trace(None);
        self.notify.session = None;
        self.notify.coverage = None;
        if let Some(source) = self.picker_source.as_ref() {
            source.set_watching(&self.cwd, false);
        }
        self.notify_rescan();
        self.message =
            "filesystem notifications lost with the worker; reestablishing coverage".into();
        self.start_notifications();
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
                        if document.buf.dirty {
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
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &picker_paths, false);
        }
    }

    /// A conservative rescan obligation (overflow, queue bound, root
    /// replacement, lease loss): invalidate the whole affected baseline
    /// and reobserve before freshness is claimed again.
    fn notify_rescan(&mut self) {
        if let Some(source) = self.picker_source.as_ref() {
            source.invalidate(&self.cwd, &[], true);
        }
        self.cancel_hunk_owner();
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
            } else if self.doc(id).buf.dirty {
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
        if document_ref.buf.dirty || !matches!(document_ref.source, DocumentSource::File) {
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
        let worker = self.filesystem.worker().clone();
        let tx = self.io.tx.clone();
        let handle = worker::spawn(
            "fs-notify-reload",
            move |outcome| {
                let _ = tx.send(IoEvent::NotifyReload(Box::new(Completion {
                    ticket,
                    outcome,
                })));
            },
            move |cancel| reload_run(&worker, &path, &cancel),
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
        if document.buf.revision() != key.revision || document.buf.dirty {
            // Edits landed while the job was in flight: the snapshot is
            // stale and never publishes. A dirty buffer takes
            // external-change state instead.
            if document.buf.dirty {
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
                    match self.publish_source_snapshot(key.document, replacement, false) {
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
                .is_some_and(|document| !document.buf.dirty)
        {
            self.request_notify_reload(key.document);
        }
    }
}
