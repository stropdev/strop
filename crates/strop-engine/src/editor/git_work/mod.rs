//! Working with the tree (0096): the `Space g g` status buffer, the
//! commit message buffer and the branch picker. The status buffer is a
//! real read-only buffer whose text is rebuilt from typed snapshots;
//! every native read and write is a worker job that owns a ticket, and
//! nothing here runs git on the input path.

mod anchor;
mod keys;
mod porcelain;
pub mod rows;

#[cfg(test)]
mod tests;

use std::collections::{HashMap, VecDeque};

use strop_core::frontend_input::Key;
use strop_core::id::{DocumentId, LineIndex};
use strop_core::worker::{CancelReason, Completion, Outcome, Ticket};
use strop_git::RepoTarget;

pub use rows::{StatusRow, StatusView};

use self::anchor::Anchor;
use self::keys::{Action, Step};
use super::document::Surface;
use super::git_memory::{
    git_failure, repo_or_unavailable, BranchKey, DraftKey, GitJob, GitMutation, MutationKey,
    MutationOp, PorcelainKey, StatusCounts, StatusData, StatusKey,
};
use super::io::OpenIntent;
use super::{trace, Editor};

/// The working tree as the user means it: git's status without the
/// editor's own lock and stage files, which sit beside open files and
/// are never part of anyone's change.
fn working_tree(repo: &strop_git::Repo) -> Result<strop_git::WorkingTree, strop_git::GitError> {
    let mut tree = repo.status()?;
    tree.entries
        .retain(|entry| !strop_fs::is_protocol_path(&entry.rel));
    Ok(tree)
}

/// One status buffer's snapshot in flight. `again` records that the
/// tree moved meanwhile: one follow-up runs when this one lands.
struct StatusRequest {
    ticket: Ticket<StatusKey>,
    again: bool,
}

/// A two-key sequence waiting for its second key in a status buffer.
struct Pending {
    document: DocumentId,
    step: Step,
}

/// The working tree's owned state: the mutation queue every git verb
/// writes through, and request ownership for the status, commit and
/// branch flows.
#[derive(Default)]
pub(crate) struct GitWork {
    /// Mutations waiting their turn — index and working-tree writes are
    /// strictly FIFO, never concurrent.
    pub(crate) mutations: VecDeque<GitMutation>,
    /// The mutation running now, with its payload for the echo row.
    pub(crate) mutation: Option<(Ticket<MutationKey>, MutationOp)>,
    status: HashMap<DocumentId, StatusRequest>,
    pending: Option<Pending>,
    draft: Option<Ticket<DraftKey>>,
    commit: Option<porcelain::CommitSession>,
    porcelain: Option<Ticket<PorcelainKey>>,
    branches: Option<Ticket<BranchKey>>,
    /// strop's private commit-graph builds (0101 §2.1).
    pub(crate) graph: super::git_memory::graph::GraphState,
}

impl GitWork {
    /// Finite work still running (the headless settle and the quit
    /// path wait on it).
    pub(crate) fn pending(&self) -> bool {
        self.mutation.is_some()
            || !self.mutations.is_empty()
            || !self.status.is_empty()
            || self.draft.is_some()
            || self.porcelain.is_some()
            || self.branches.is_some()
            || self.graph.building()
    }

    /// An accepted write that shutdown must let finish: a commit the
    /// user already confirmed with `:wq`.
    pub(crate) fn write_pending(&self, request: strop_core::worker::WorkerId) -> bool {
        self.porcelain
            .as_ref()
            .is_some_and(|ticket| ticket.request == request)
    }

    pub(crate) fn clear_pending_keys(&mut self) {
        self.pending = None;
    }
}

impl Editor {
    /// The current buffer's status payload, when it is a status buffer.
    pub fn status_view(&self) -> Option<&StatusView> {
        match self.surface()? {
            Surface::Status { view, .. } => Some(view),
            _ => None,
        }
    }

    fn status_view_mut(&mut self, document: DocumentId) -> Option<&mut StatusView> {
        match self.docs.get_mut(document)?.surface_payload_mut()? {
            Surface::Status { view, .. } => Some(view),
            _ => None,
        }
    }

    /// The local repository a working-tree command acts on. A remote
    /// repository refuses in one sentence (0096 §2.4) and nothing
    /// local is touched; no repository says so.
    fn working_repo(&mut self, what: &str) -> Option<RepoTarget> {
        let Some(context) = self.git_context() else {
            // discovery is a job: right after opening, it may not be back
            self.message = if matches!(self.git_discovery, strop_core::worker::Load::Running(_)) {
                "still looking for the repository — try again"
            } else {
                "not a git repository"
            }
            .into();
            return None;
        };
        if context.repo.is_remote() {
            self.message = format!("git {what} needs a local repository");
            return None;
        }
        Some(context.repo.clone())
    }

    /// `Space g g`: the changed-files buffer. One per repository — a
    /// second press returns to it and refreshes.
    pub(crate) fn open_status(&mut self) {
        let Some(repo) = self.working_repo("status") else {
            return;
        };
        let open = self.docs.iter().find_map(|(id, document)| {
            (matches!(document.surface_payload(), Some(Surface::Status { .. }))
                && document
                    .git_context()
                    .is_some_and(|context| context.repo == repo))
            .then_some(id)
        });
        if let Some(document) = open {
            if self.current() != document {
                self.push_jump();
                self.switch_to(document);
            }
            self.refresh_status(document);
            return;
        }
        self.push_surface(
            Some("git status"),
            ropey::Rope::from_str("loading status…"),
            Surface::Status {
                view: StatusView::default(),
                return_to: None,
            },
        );
        if self.status_view().is_some() {
            self.refresh_status(self.current());
        }
    }

    /// Every open status buffer takes a fresh snapshot: the tree moved
    /// (a mutation, a commit, a watcher hint, the terminal's focus).
    pub(crate) fn refresh_status_buffers(&mut self) {
        let open: Vec<DocumentId> = self
            .docs
            .iter()
            .filter(|(_, document)| {
                matches!(document.surface_payload(), Some(Surface::Status { .. }))
            })
            .map(|(id, _)| id)
            .collect();
        for document in open {
            self.refresh_status(document);
        }
    }

    /// Register and launch one snapshot for a status buffer. While one
    /// is running, further requests coalesce into a single follow-up.
    pub(crate) fn refresh_status(&mut self, document: DocumentId) {
        if self.finishing {
            return;
        }
        if let Some(running) = self.git_work.status.get_mut(&document) {
            running.again = true;
            return;
        }
        let Some((repo, expanded)) = self.docs.get(document).and_then(|doc| {
            match (doc.surface_payload(), doc.git_context()) {
                (Some(Surface::Status { view, .. }), Some(context)) => {
                    Some((context.repo.clone(), view.expanded.clone()))
                }
                _ => None,
            }
        }) else {
            return;
        };
        let Some(ticket) = self.git_ticket(StatusKey {
            document,
            repo: repo.clone(),
        }) else {
            return;
        };
        self.git_work.status.insert(
            document,
            StatusRequest {
                ticket: ticket.clone(),
                again: false,
            },
        );
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"git","request":"status",
                "document":{"slot":document.index(),"generation":document.generation()},
                "expanded":expanded.len(),
            })
        });
        let args = (ticket.clone(), expanded.clone());
        let workdir = repo.workdir().to_path_buf();
        let tx = self.git_tx.clone();
        let snapshot_ticket = ticket.clone();
        self.launch_git_job(
            "git-status",
            "git.status",
            ticket,
            &args,
            GitJob::Status,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let failed = |failure| Outcome::Failed {
                    failure,
                    partial: None,
                };
                let repo = match repo_or_unavailable(&workdir) {
                    Ok(repo) => repo,
                    Err(failure) => return failed(failure),
                };
                let tree = match working_tree(&repo) {
                    Ok(tree) => tree,
                    Err(error) => return failed(git_failure("status", error)),
                };
                let mut deltas = Vec::new();
                for entry in expanded {
                    if !tree.entries.contains(&entry) {
                        continue;
                    }
                    match repo.file_delta(&entry) {
                        Ok(delta) => deltas.push((entry, delta)),
                        Err(error) => return failed(git_failure("diff", error)),
                    }
                }
                // the list first; every entry's `+N -M` (0100 §7) after it,
                // from two whole-tree diffs — a status of thousands of
                // changed files is not held up by counting them
                let counted = tree.clone();
                let _ = tx.send(GitJob::StatusSnapshot(
                    snapshot_ticket,
                    StatusData { tree, deltas },
                ));
                let counts = match repo.line_stats() {
                    Ok(counts) => counts,
                    Err(error) => return failed(git_failure("diff", error)),
                };
                Outcome::Success(
                    counted
                        .entries
                        .iter()
                        .map(|entry| (!entry.is_directory()).then(|| counts.of(entry)).flatten())
                        .collect(),
                )
            },
        );
    }

    /// The list of a snapshot still being counted: published now.
    pub(crate) fn handle_status_snapshot(&mut self, ticket: Ticket<StatusKey>, data: StatusData) {
        let document = ticket.key.document;
        let owned = self
            .git_work
            .status
            .get(&document)
            .is_some_and(|running| running.ticket == ticket);
        if !owned || self.status_view_mut(document).is_none() {
            trace::services::rejected("git", "status snapshot superseded or buffer closed");
            return;
        }
        self.publish_status(document, data);
    }

    pub(crate) fn handle_status_completion(
        &mut self,
        completion: Completion<StatusKey, StatusCounts>,
    ) {
        let document = completion.ticket.key.document;
        let Some(again) = self
            .git_work
            .status
            .get(&document)
            .filter(|running| running.ticket == completion.ticket)
            .map(|running| running.again)
        else {
            trace::services::rejected("git", "status request superseded");
            return;
        };
        self.git_work.status.remove(&document);
        self.worker_handles.remove(&completion.ticket.request);
        if self.status_view_mut(document).is_none() {
            trace::services::rejected("git", "status buffer closed");
            return;
        }
        match completion.outcome {
            Outcome::Success(counts) => {
                if let Some(view) = self.status_view_mut(document) {
                    // the counts are of the snapshot this request published
                    if view.loaded && counts.len() == view.tree.entries.len() {
                        view.stats = counts;
                    }
                }
            }
            Outcome::Failed { failure, .. } => {
                let loaded = self
                    .status_view_mut(document)
                    .is_some_and(|view| view.loaded);
                if !loaded {
                    // never "loading" forever
                    let text = format!("git status failed: {}\n", failure.message);
                    if self.replace_system(document, &text).is_err() {
                        trace::services::rejected("git", "status publish failed");
                    }
                }
                if !self.docs.is_empty() && self.current() == document {
                    self.message = format!("git status failed: {}", failure.message);
                }
            }
            Outcome::Cancelled(_) => {}
        }
        if again {
            self.refresh_status(document);
        }
    }

    /// Land a snapshot: rebuild the text and the row model together and
    /// put the cursor back on what it was on.
    fn publish_status(&mut self, document: DocumentId, data: StatusData) {
        let current = !self.docs.is_empty() && self.current() == document;
        let line = if current {
            self.buf().line_of(self.head())
        } else {
            0
        };
        let Some(view) = self.status_view_mut(document) else {
            return;
        };
        let first = !view.loaded;
        let anchor = Anchor::capture(view, line);
        let text = view.install(data.tree, data.deltas);
        let land = if first {
            view.rows
                .iter()
                .position(|row| matches!(row, StatusRow::File(_)))
                .unwrap_or(0)
        } else {
            anchor.land(view)
        };
        if let Err(error) = self.replace_system(document, &text) {
            trace::services::rejected("git", "status publish failed");
            self.message = format!("git status publish failed: {error}");
            return;
        }
        if current {
            let at = self.buf().line_start(land);
            self.set_head(at);
            self.clamp_cursor();
            self.scroll_to_cursor(self.view_rows());
        }
    }

    /// Rebuild the buffer from the snapshot already held, after a
    /// purely local change (collapsing an inline diff).
    fn republish_status(&mut self, document: DocumentId) {
        let Some(view) = self.status_view_mut(document) else {
            return;
        };
        let data = StatusData {
            tree: view.tree.clone(),
            deltas: view
                .deltas
                .iter()
                .filter(|(entry, _)| view.expanded.contains(entry))
                .cloned()
                .collect(),
        };
        self.publish_status(document, data);
    }

    /// A closing document releases the status request it owned.
    pub(crate) fn revoke_git_work_for(&mut self, document: DocumentId) {
        if let Some(running) = self.git_work.status.remove(&document) {
            self.cancel_git_worker(running.ticket.request, CancelReason::Dismissed);
        }
        if self
            .git_work
            .pending
            .as_ref()
            .is_some_and(|pending| pending.document == document)
        {
            self.git_work.pending = None;
        }
        self.commit_document_closing(document);
    }

    /// One key in a status buffer. `false` hands it on to the shared
    /// readonly path. The decision is `keys::decide`'s; this carries
    /// it out.
    pub(crate) fn status_key(&mut self, key: Key) -> bool {
        let document = self.current();
        let pending = self
            .git_work
            .pending
            .take()
            .filter(|pending| pending.document == document)
            .map(|pending| pending.step);
        let line = self.buf().line_of(self.head());
        let Some(action) = self
            .status_view()
            .map(|view| keys::decide(view, line, key, pending))
        else {
            return false;
        };
        let wait = |step| Some(Pending { document, step });
        match action {
            Action::Pass => return false,
            Action::Say(text) => self.message = text,
            Action::Mutate { rel, op } => self.queue_status_mutation(rel, op),
            Action::Ask { prompt, step } => {
                self.message = prompt;
                self.git_work.pending = wait(step);
            }
            Action::Await(step) => self.git_work.pending = wait(step),
            Action::Commit { amend } => self.begin_commit(amend),
            Action::ToggleDiff(entry) => {
                let Some(view) = self.status_view_mut(document) else {
                    return true;
                };
                if let Some(at) = view.expanded.iter().position(|open| *open == entry) {
                    view.expanded.remove(at);
                    self.republish_status(document);
                } else {
                    view.expanded.push(entry);
                    self.refresh_status(document);
                }
            }
            Action::Open { entry, line } => {
                let Some(repo) = self.working_repo("status") else {
                    return true;
                };
                let intent = match line {
                    _ if entry.is_directory() => OpenIntent::Browse,
                    Some(line) => OpenIntent::AtLine {
                        line: LineIndex::new(line.saturating_sub(1)),
                    },
                    None => OpenIntent::Switch { readonly: false },
                };
                self.push_jump();
                self.request_open(repo.workdir().join(&entry.rel), intent);
            }
        }
        true
    }

    /// Queue one mutation owned by this status buffer at its revision.
    fn queue_status_mutation(&mut self, rel: std::path::PathBuf, op: MutationOp) {
        let Some(repo) = self.working_repo("staging") else {
            return;
        };
        let key = MutationKey {
            document: self.current(),
            revision: self.buf().revision(),
            kind: op.kind(),
            rel,
            repo,
            git_view: self.git_view,
        };
        self.git_work.mutations.push_back(GitMutation { key, op });
        self.pump_git_mutations();
    }

    /// `]c` / `[c` in a status buffer.
    pub(crate) fn status_hunk_step(&mut self, forward: bool) {
        let line = self.buf().line_of(self.head());
        let Some(step) = self
            .status_view()
            .map(|view| keys::hunk_step(view, line, forward))
        else {
            return;
        };
        match step {
            Ok(line) => {
                let start = self.buf().line_start(line);
                self.set_head(start);
                self.clamp_cursor();
            }
            Err(why) => self.message = why.into(),
        }
    }
}
