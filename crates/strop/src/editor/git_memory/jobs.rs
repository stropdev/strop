//! Owned git jobs (R9/R6): every native read and mutation carries a
//! `Ticket` naming the exact view it belongs to. Success, failure,
//! panic and cancellation are all terminal — the shared worker wrapper
//! guarantees a completion exists, and handlers validate the ticket
//! before touching any editor state, so a stale reply can never clear
//! a newer owner's loading state or publish over it.

use std::path::{Path, PathBuf};

use strop_core::id::DocumentId;
use strop_core::worker::{
    self, CancelReason, Completion, Failure, FailureKind, Load, Outcome, Ticket, WorkerId,
};

use strop_git::memory::{BlameCard, BlameLine, LogRow};
use strop_git::{GitContext, GitError};

use super::types::*;
use super::CommitFiles;
use crate::editor::{trace, Editor, Surface};

/// What the dive's origin surface turned out to be at completion
/// time — copied out of the document before any mutation, so the
/// surface check and the publish never fight over a borrow.
enum DiveLanding {
    LogSurface,
    Files {
        sha: String,
        files: Vec<strop_git::memory::ChangedFile>,
    },
    Delta {
        cf: CommitFiles,
        path: PathBuf,
    },
    Mismatch,
}

// ---- shared plumbing -------------------------------------------------------

/// Map a typed repository error onto the shared failure model.
pub(crate) fn git_failure(op: &str, error: GitError) -> Failure {
    match error {
        GitError::OutsideWorkdir => Failure::new(
            FailureKind::InvalidInput,
            format!("{op}: path outside the repository workdir"),
        ),
        GitError::Native(message) => Failure::new(FailureKind::Exit, format!("{op}: {message}")),
    }
}

/// Rediscover the repository inside a worker, or fail typed — never an
/// anonymous empty result.
pub(crate) fn repo_or_unavailable(workdir: &Path) -> Result<strop_git::Repo, Failure> {
    strop_git::Repo::discover(workdir).ok_or_else(|| {
        Failure::new(
            FailureKind::Unavailable,
            format!("git repository unavailable at {}", workdir.display()),
        )
    })
}

impl Editor {
    /// Allocate a request ticket for a git job. Identity exhaustion is
    /// reported, never unwrapped.
    pub(crate) fn git_ticket<K>(&mut self, key: K) -> Option<Ticket<K>> {
        match self.worker_ids.allocate() {
            Ok(request) => Some(Ticket { request, key }),
            Err(failure) => {
                self.message = format!("git: {}", failure.message);
                trace::services::rejected("git", "worker request IDs exhausted");
                None
            }
        }
    }

    /// Cancel one worker and drop its handle; a completion with
    /// `Cancelled` may still arrive — handlers reject it once the
    /// owner is gone.
    pub(crate) fn cancel_git_worker(&mut self, request: WorkerId, reason: CancelReason) {
        if let Some(handle) = self.worker_handles.remove(&request) {
            handle.cancel(reason);
        }
    }

    /// The launch gate (replay contract): the owner/ticket is already
    /// installed when this runs. `true` launches native work, `false`
    /// means replay will supply the result, and a tape error is sticky
    /// and never launches.
    fn tape_launch(&mut self, op: &'static str, args: &impl serde::Serialize) -> bool {
        match self.tape.request(op, args) {
            Ok(true) => true,
            Ok(false) => false,
            Err(error) => {
                self.message = format!("replay tape error: {error}");
                trace::services::rejected("git", "replay tape failed");
                false
            }
        }
    }

    /// Spawn the native half of a registered request. Callers MUST
    /// have installed the owner (Load/registry entry) first.
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn launch_git_job<K, T>(
        &mut self,
        name: &'static str,
        op: &'static str,
        ticket: Ticket<K>,
        args: &impl serde::Serialize,
        make: impl FnOnce(Completion<K, T>) -> GitJob + Send + 'static,
        work: impl FnOnce(worker::CancelToken) -> Outcome<T> + Send + 'static,
    ) where
        K: Clone + Send + 'static,
        T: Send + 'static,
    {
        if !self.tape_launch(op, args) {
            return; // replay: registration stands, native work withheld
        }
        let tx = self.git_tx.clone();
        let emit_ticket = ticket.clone();
        let handle = worker::spawn(
            name,
            move |outcome| {
                let _ = tx.send(make(Completion {
                    ticket: emit_ticket,
                    outcome,
                }));
            },
            work,
        );
        self.worker_handles.insert(ticket.request, handle);
    }

    /// A pure-validation failure for an already-registered request:
    /// settle it through the same terminal path as worker failures —
    /// same channel, same handler, same ownership check. No spawning.
    pub(crate) fn send_git_failure<K, T>(
        &self,
        ticket: Ticket<K>,
        kind: FailureKind,
        message: impl Into<String>,
        make: impl FnOnce(Completion<K, T>) -> GitJob,
    ) {
        let completion = Completion {
            ticket,
            outcome: Outcome::Failed {
                failure: Failure::new(kind, message),
                partial: None,
            },
        };
        let _ = self.git_tx.send(make(completion));
    }

    /// The index moved under every cached diff: a new git view, the
    /// old owner cancelled, both hunk vectors cleared honestly for the
    /// one frame the recompute takes.
    pub(crate) fn invalidate_git_view(&mut self) {
        let running = match &self.hunk_load {
            Load::Running(ticket) => Some(ticket.request),
            _ => None,
        };
        if let Some(request) = running {
            self.cancel_git_worker(request, CancelReason::Superseded);
        }
        self.hunk_load = Load::Idle;
        self.hunks.clear();
        self.staged_hunks.clear();
        self.hunks_untracked = false;
        if let Ok(view) = self.worker_ids.allocate() {
            self.git_view = view;
        }
    }

    /// Revoke every git request a closing document owned (the
    /// document-close path calls this BEFORE removal): late results
    /// for a dead or recycled slot cannot publish.
    pub(crate) fn revoke_git_requests_for(&mut self, doc: DocumentId) {
        if let Some(ticket) = self.log_requests.remove(&doc) {
            self.cancel_git_worker(ticket.request, CancelReason::Dismissed);
        }
        if let Some(ticket) = self.dive_requests.remove(&doc) {
            self.cancel_git_worker(ticket.request, CancelReason::Dismissed);
        }
    }

    // ---- context (discovery) ------------------------------------------------

    pub(crate) fn handle_context_completion(
        &mut self,
        completion: Completion<ContextKey, Option<GitContext>>,
    ) {
        if !self.git_discovery.owns(&completion.ticket) {
            trace::services::rejected("git", "discovery superseded");
            return;
        }
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        match completion.outcome {
            Outcome::Success(context) => {
                self.git_discovery = Load::Ready(key);
                // an equal context (same HEAD, branch, remotes) means
                // the cached view is still valid — no churn, no reload
                if self.git.as_ref() != context.as_ref() {
                    self.git = context;
                    self.invalidate_git_view();
                }
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("git discovery failed: {}", failure.message);
                self.git_discovery = Load::Failed { key, failure };
            }
            Outcome::Cancelled(reason) => {
                self.git_discovery = Load::Cancelled { key, reason };
            }
        }
    }

    // ---- hunks ----------------------------------------------------------------

    pub(crate) fn handle_hunk_completion(&mut self, completion: Completion<HunkKey, HunkData>) {
        if !self.hunk_load.owns(&completion.ticket) {
            trace::services::rejected("git", "hunk request no longer owns the view");
            return;
        }
        self.worker_handles.remove(&completion.ticket.request);
        self.hunk_load = Load::Idle;
        let key = completion.ticket.key;
        // the snapshot applies only to the document that asked, at
        // that revision, in that git view — nothing else
        let valid = !self.docs.is_empty()
            && self.current() == key.document
            && self.docs.get(key.document).is_some_and(|d| {
                d.buf.revision() == key.revision && d.buf.path.as_ref() == Some(&key.path)
            })
            && self.git_view == key.git_view;
        if !valid {
            trace::services::rejected("git", "hunk document, revision or view changed");
            return;
        }
        match completion.outcome {
            Outcome::Success(data) => {
                self.hunks = data.unstaged;
                self.staged_hunks = data.staged;
                self.hunks_untracked = data.untracked;
                self.hunk_load = Load::Ready(key);
            }
            Outcome::Failed { failure, .. } => {
                self.message = format!("git diff failed: {}", failure.message);
                self.hunk_load = Load::Failed { key, failure };
            }
            Outcome::Cancelled(reason) => {
                self.hunk_load = Load::Cancelled { key, reason };
            }
        }
    }

    // ---- mutations --------------------------------------------------------------

    /// Launch the next queued mutation if none is running. Index
    /// writes are serialized FIFO; each request still gets its own
    /// ticket at launch and settles terminally.
    pub(crate) fn pump_git_mutations(&mut self) {
        while self.git_mutation.is_none() {
            let Some(mutation) = self.git_mutations.pop_front() else {
                return;
            };
            // pure re-validation at launch: the buffer and the view
            // the command targeted must still be current
            let valid = self.git_view == mutation.key.git_view
                && self
                    .docs
                    .get(mutation.key.document)
                    .is_some_and(|d| d.buf.revision() == mutation.key.revision);
            if !valid {
                trace::services::rejected("git", "mutation superseded before launch");
                continue;
            }
            let Some(ticket) = self.git_ticket(mutation.key.clone()) else {
                return; // identity exhausted: later requests cannot fare better
            };
            self.git_mutation = Some(ticket.clone());
            strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
                serde_json::json!({
                    "service":"git","request":"mutation","edge":ticket.key.kind.edge(),
                    "document":{"slot":ticket.key.document.index(),"generation":ticket.key.document.generation()},
                    "revision":ticket.key.revision.get(),"path":ticket.key.rel.to_string_lossy(),
                })
            });
            let args = (ticket.clone(), mutation.op.clone());
            let workdir = ticket.key.workdir.clone();
            let rel = ticket.key.rel.clone();
            let kind = ticket.key.kind;
            let op = mutation.op;
            self.launch_git_job(
                "git-mutate",
                "git.mutation",
                ticket,
                &args,
                GitJob::Mutation,
                move |cancel| {
                    if cancel.is_cancelled() {
                        return Outcome::Cancelled(CancelReason::Superseded);
                    }
                    let repo = match repo_or_unavailable(&workdir) {
                        Ok(repo) => repo,
                        Err(failure) => {
                            return Outcome::Failed {
                                failure,
                                partial: None,
                            }
                        }
                    };
                    let result = match &op {
                        MutationOp::Stage { hunk } => repo.stage_hunk(&rel, hunk),
                        MutationOp::Unstage { hunk } => repo.unstage_hunk(&rel, hunk),
                    };
                    match result {
                        Ok(()) => Outcome::Success(()),
                        Err(message) => Outcome::Failed {
                            failure: Failure::new(
                                FailureKind::Exit,
                                format!("{}: {message}", kind.edge()),
                            ),
                            partial: None,
                        },
                    }
                },
            );
        }
    }

    pub(crate) fn handle_mutation_completion(&mut self, completion: Completion<MutationKey, ()>) {
        if self.git_mutation.as_ref() != Some(&completion.ticket) {
            trace::services::rejected("git", "mutation superseded");
            return;
        }
        self.git_mutation = None;
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        // the index write already happened (or failed) — validation
        // decides only whether THIS editor view still reports it
        let current = !self.docs.is_empty()
            && self.current() == key.document
            && self
                .docs
                .get(key.document)
                .is_some_and(|d| d.buf.revision() == key.revision)
            && self.git_view == key.git_view;
        match completion.outcome {
            Outcome::Success(()) => {
                // the index changed under every cached diff: new view
                self.invalidate_git_view();
                if current {
                    self.message = match key.kind {
                        MutationKind::Stage => "hunk staged".into(),
                        MutationKind::Unstage => "hunk unstaged".into(),
                    };
                }
            }
            Outcome::Failed { failure, .. } => {
                if current {
                    self.message = match key.kind {
                        MutationKind::Stage => {
                            format!("stage failed: {}", failure.message)
                        }
                        MutationKind::Unstage => {
                            format!("unstage failed: {}", failure.message)
                        }
                    };
                }
            }
            Outcome::Cancelled(_) => {}
        }
        self.pump_git_mutations();
    }

    // ---- log --------------------------------------------------------------------

    /// Land successful log rows in the surface buffer — the existing
    /// text/focus/cursor contract, extracted from the old inline
    /// handler. An empty successful log is a completed empty list.
    fn publish_log_rows(&mut self, doc: DocumentId, rows: Vec<LogRow>) {
        let text = rows
            .iter()
            .map(|r| r.text.as_str())
            .collect::<Vec<_>>()
            .join("\n")
            + "\n";
        if let Err(error) = self.replace_system(doc, &text) {
            trace::services::rejected("git", "log publish failed");
            self.message = format!("git log publish failed: {error}");
            return;
        }
        let mut focus_row = None;
        if let Some(Some(Surface::CommitLog {
            rows: slot, focus, ..
        })) = self.docs.get_mut(doc).map(|d| d.surface_payload_mut())
        {
            focus_row = focus
                .take()
                .and_then(|sha| rows.iter().position(|r| r.sha.as_deref() == Some(&sha)));
            *slot = rows;
        }
        if let Some(row) = focus_row {
            // the blame dive asked for this commit: land on it (only
            // when the browser is still what's being driven)
            if self.current() == doc {
                let at = self.doc(doc).buf.line_start(row);
                self.set_head(at);
                self.view_mut().view_top = row;
            }
        }
    }

    /// Replace a log surface's loading text with a terminal status;
    /// rows stay empty. The surface never shows "loading" forever.
    fn publish_log_status(&mut self, doc: DocumentId, status: &str) {
        if let Err(error) = self.replace_system(doc, status) {
            trace::services::rejected("git", "log status publish failed");
            self.message = format!("git log publish failed: {error}");
        }
    }

    pub(crate) fn handle_log_completion(&mut self, completion: Completion<LogKey, Vec<LogRow>>) {
        let doc = completion.ticket.key.document;
        if self.log_requests.get(&doc) != Some(&completion.ticket) {
            trace::services::rejected("git", "log request superseded");
            return;
        }
        self.log_requests.remove(&doc);
        self.worker_handles.remove(&completion.ticket.request);
        // only the still-live CommitLog surface at its request
        // revision accepts rows — a closed/recycled slot cannot
        let valid = self.docs.get(doc).is_some_and(|d| {
            d.buf.revision() == completion.ticket.key.revision
                && matches!(d.surface_payload(), Some(Surface::CommitLog { .. }))
        });
        if !valid {
            trace::services::rejected("git", "log surface changed or closed");
            return;
        }
        match completion.outcome {
            Outcome::Success(rows) => self.publish_log_rows(doc, rows),
            Outcome::Failed { failure, .. } => {
                self.publish_log_status(doc, &format!("git log failed: {}\n", failure.message));
                if !self.docs.is_empty() && self.current() == doc {
                    self.message = failure.message;
                }
            }
            Outcome::Cancelled(_) => self.publish_log_status(doc, "git log cancelled\n"),
        }
    }

    // ---- blame -------------------------------------------------------------------

    pub(crate) fn handle_gutter_completion(
        &mut self,
        completion: Completion<BlameKey, Vec<BlameLine>>,
    ) {
        let path = completion.ticket.key.path.clone();
        if !self
            .blame_gutters
            .get(&path)
            .is_some_and(|g| g.request.as_ref() == Some(&completion.ticket))
        {
            trace::services::rejected("git", "gutter request superseded or toggled off");
            return;
        }
        self.blame_gutters
            .get_mut(&path)
            .and_then(|g| g.request.take());
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        let valid = self.docs.get(key.document).is_some_and(|d| {
            d.buf.revision() == key.revision
                && d.buf
                    .path
                    .as_deref()
                    .is_some_and(|p| self.blame_key_of(p) == key.path)
        });
        match completion.outcome {
            Outcome::Success(lines) => {
                if !valid {
                    // stale pairing: keep the marker inert (its
                    // trust gate already refuses mismatches), never
                    // republish over a changed buffer
                    trace::services::rejected("git", "gutter document changed");
                    return;
                }
                if let Some(gutter) = self.blame_gutters.get_mut(&path) {
                    gutter.lines = lines;
                }
                // the gutter supersedes the interim card that covered
                // the load for this buffer
                if !self.docs.is_empty() && self.current() == key.document {
                    if let Some(ticket) = self.card_request.take() {
                        self.cancel_git_worker(ticket.request, CancelReason::Superseded);
                    }
                    self.blame_card = None;
                }
            }
            Outcome::Failed { failure, .. } => {
                // a failed load removes its own loading marker: the
                // next toggle genuinely starts another request
                self.blame_gutters.remove(&path);
                if valid && !self.docs.is_empty() && self.current() == key.document {
                    self.message = format!("blame failed: {}", failure.message);
                }
            }
            Outcome::Cancelled(_) => {
                // cancelled while loading: the marker dies with the
                // request (toggle-off already removed it — this is
                // the supersedure case)
                self.blame_gutters.remove(&path);
            }
        }
    }

    pub(crate) fn handle_card_completion(
        &mut self,
        completion: Completion<CardKey, Box<BlameCard>>,
    ) {
        if self.card_request.as_ref() != Some(&completion.ticket) {
            trace::services::rejected("git", "card request superseded or dismissed");
            return;
        }
        self.card_request = None;
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        // the card describes the cursor line of the document that
        // asked, at the revision it asked at
        let valid = !self.docs.is_empty()
            && self.current() == key.origin.document
            && self.buf().revision() == key.origin.revision
            && self.buf().line_of(self.head()) + 1 == key.line;
        if !valid {
            trace::services::rejected("git", "card origin changed");
            return;
        }
        match completion.outcome {
            Outcome::Success(card) => self.blame_card = Some(*card),
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }

    // ---- dive --------------------------------------------------------------------

    pub(crate) fn handle_dive_completion(&mut self, completion: Completion<DiveKey, DiveData>) {
        let doc = completion.ticket.key.document;
        if self.dive_requests.get(&doc) != Some(&completion.ticket) {
            trace::services::rejected("git", "dive request superseded");
            return;
        }
        let key = completion.ticket.key;
        self.dive_requests.remove(&doc);
        self.worker_handles.remove(&completion.ticket.request);
        let outcome = completion.outcome;
        // read the surface NOW, copy out what the landing needs, then
        // drop the borrow before any editor mutation
        let landing = self
            .docs
            .get(doc)
            .map_or(DiveLanding::Mismatch, |document| {
                match (&key.target, document.surface_payload()) {
                    (DiveTarget::CommitFiles { .. }, Some(Surface::CommitLog { .. })) => {
                        DiveLanding::LogSurface
                    }
                    (
                        DiveTarget::FileDelta { sha, .. },
                        Some(Surface::ChangedFiles {
                            sha: surface_sha,
                            files,
                            ..
                        }),
                    ) if surface_sha == sha => DiveLanding::Files {
                        sha: sha.clone(),
                        files: files.clone(),
                    },
                    (
                        DiveTarget::FileDelta { sha, path },
                        Some(Surface::Diff {
                            commit: Some(cf), ..
                        }),
                    ) if cf.sha == *sha && cf.files.iter().any(|f| f.path == *path) => {
                        DiveLanding::Delta {
                            cf: cf.clone(),
                            path: path.clone(),
                        }
                    }
                    _ => DiveLanding::Mismatch,
                }
            });
        match (landing, outcome) {
            (DiveLanding::LogSurface, Outcome::Success(DiveData::Files(files))) => {
                let sha = match &key.target {
                    DiveTarget::CommitFiles { sha } => sha.clone(),
                    _ => return,
                };
                let mut text = format!("commit {}\n\n", &sha[..10.min(sha.len())]);
                for f in &files {
                    text.push_str(&f.path.display().to_string());
                    text.push('\n');
                }
                self.push_surface(
                    Some("commit files"),
                    &text,
                    Surface::ChangedFiles {
                        sha,
                        files,
                        return_to: None,
                    },
                );
            }
            (DiveLanding::Files { sha, files }, Outcome::Success(DiveData::Delta(diff))) => {
                let commit = CommitFiles { sha, files };
                let path = match &key.target {
                    DiveTarget::FileDelta { path, .. } => path.clone(),
                    _ => return,
                };
                let label = path.display().to_string();
                self.open_delta("delta", &label, diff.hunks, None, Some(commit));
            }
            (DiveLanding::Delta { cf, path }, Outcome::Success(DiveData::Delta(diff))) => {
                self.load_commit_delta(&cf, &path, diff.hunks);
            }
            // a payload that does not match its target cannot come
            // from our producers; reject rather than mis-publish
            (_, Outcome::Success(_)) => {
                trace::services::rejected("git", "dive surface changed or payload mismatched")
            }
            (_, Outcome::Failed { failure, .. }) => self.message = failure.message,
            (_, Outcome::Cancelled(_)) => {}
        }
    }

    // ---- dispatch ------------------------------------------------------------------

    /// One git job result (TUI events land here directly — 0018; the
    /// headless drains call this too).
    pub(crate) fn handle_git_job(&mut self, job: GitJob) {
        trace::services::git(&job);
        match job {
            GitJob::Context(c) => self.handle_context_completion(c),
            GitJob::Hunks(c) => self.handle_hunk_completion(c),
            GitJob::Mutation(c) => self.handle_mutation_completion(c),
            GitJob::Log(c) => self.handle_log_completion(c),
            GitJob::Gutter(c) => self.handle_gutter_completion(c),
            GitJob::Card(c) => self.handle_card_completion(c),
            GitJob::Dive(c) => self.handle_dive_completion(c),
        }
    }
}
