//! The flows that end in a `git` CLI run (0096 §2.2, §2.3), so hooks,
//! signing, config and the working-tree checks are git's own.
//!
//! Commit: the message is `<gitdir>/COMMIT_EDITMSG`, an ordinary file
//! buffer. A worker writes the template; closing the buffer after a
//! write runs `git commit`, closing it unwritten aborts.
//!
//! Branches (`Space g b`): the list in the one picker. Enter runs `git
//! switch`; the pinned last row creates a branch from the typed name.
//! Git decides whether the tree allows it and its words are shown.

use std::ffi::OsString;
use std::path::PathBuf;
use std::time::Duration;

use strop_core::id::DocumentId;
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome};
use strop_git::exec::GitExec;
use strop_git::{Branch, RepoTarget};
use strop_picker::{BranchChoice, Item, Kind, Payload, Picker};

use crate::editor::document::DocumentSource;
use crate::editor::git_memory::{
    git_failure, repo_or_unavailable, BranchKey, CommitDraft, DraftKey, GitJob, Porcelain,
    PorcelainDone, PorcelainKey,
};
use crate::editor::io::OpenIntent;
use crate::editor::picker::PickerGlue;
use crate::editor::{trace, Editor};

/// The pinned branch-picker row; the typed filter text is the new name.
const CREATE_ROW: &str = "create a branch from the typed name";

/// Hooks lint and test; a commit may honestly take minutes.
const PORCELAIN_DEADLINE: Duration = Duration::from_secs(600);

/// One commit message being edited.
pub(super) struct CommitSession {
    /// The message file; the buffer editing it is found by this path.
    path: PathBuf,
    repo: RepoTarget,
    amend: bool,
    /// The file was written since the template: closing commits.
    written: bool,
}

/// Git's answer as one echo-row line: the first line of a success, the
/// whole refusal folded onto one line (`:messages` keeps it all).
fn summary(stdout: &[u8], stderr: &[u8], whole: bool) -> String {
    let pick = |bytes: &[u8]| {
        let text = String::from_utf8_lossy(bytes);
        let mut lines = text.lines().map(str::trim).filter(|line| !line.is_empty());
        if whole {
            lines.collect::<Vec<_>>().join(" ")
        } else {
            lines.next().unwrap_or_default().to_string()
        }
    };
    let (first, second) = if whole {
        (stderr, stdout)
    } else {
        (stdout, stderr)
    };
    let line = pick(first);
    if line.is_empty() {
        pick(second)
    } else {
        line
    }
}

impl Editor {
    fn commit_message_document(&self, path: &std::path::Path) -> Option<DocumentId> {
        self.docs.iter().find_map(|(id, document)| {
            (matches!(document.source, DocumentSource::File)
                && document.buf.path.as_deref() == Some(path))
            .then_some(id)
        })
    }

    /// The document is the commit message being edited.
    pub(crate) fn is_commit_message(&self, document: DocumentId) -> bool {
        self.git_work
            .commit
            .as_ref()
            .is_some_and(|session| self.commit_message_document(&session.path) == Some(document))
    }

    /// `Space g c` / `cc`, and `ca` to amend: write the template, then
    /// open it. Nothing staged (and not amending) refuses before any
    /// buffer opens.
    pub(crate) fn begin_commit(&mut self, amend: bool) {
        let Some(repo) = self.working_repo("commit") else {
            return;
        };
        if let Some(session) = &self.git_work.commit {
            if let Some(document) = self.commit_message_document(&session.path) {
                if self.current() != document {
                    self.push_jump();
                    self.switch_to(document);
                }
                self.message = "a commit message is already open — :wq commits, :q! aborts".into();
                return;
            }
            self.git_work.commit = None;
        }
        if self.git_work.draft.is_some() || self.git_work.porcelain.is_some() {
            self.message = "git is still running the previous command".into();
            return;
        }
        let Some(ticket) = self.git_ticket(DraftKey {
            repo: repo.clone(),
            amend,
        }) else {
            return;
        };
        self.git_work.draft = Some(ticket.clone());
        let args = ticket.clone();
        let workdir = repo.workdir().to_path_buf();
        self.launch_git_job(
            "git-commit-draft",
            "git.commit_draft",
            ticket,
            &args,
            GitJob::CommitDraft,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let written = repo_or_unavailable(&workdir).and_then(|repo| {
                    repo.write_commit_template(amend)
                        .map_err(|error| git_failure("commit", error))
                });
                match written {
                    Ok(path) => Outcome::Success(CommitDraft { path }),
                    Err(failure) => Outcome::Failed {
                        failure,
                        partial: None,
                    },
                }
            },
        );
    }

    pub(crate) fn handle_commit_draft(&mut self, completion: Completion<DraftKey, CommitDraft>) {
        if self.git_work.draft.as_ref() != Some(&completion.ticket) {
            trace::services::rejected("git", "commit draft superseded");
            return;
        }
        self.git_work.draft = None;
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        match completion.outcome {
            Outcome::Success(draft) => {
                self.git_work.commit = Some(CommitSession {
                    path: draft.path.clone(),
                    repo: key.repo,
                    amend: key.amend,
                    written: false,
                });
                self.push_jump();
                self.request_open(draft.path, OpenIntent::Switch { readonly: false });
                self.message = if key.amend {
                    "amend: edit the message — :wq commits, :q! aborts"
                } else {
                    "write the message — :wq commits, :q! aborts"
                }
                .into();
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }

    /// A confirmed save of the message file arms the commit.
    pub(crate) fn commit_message_saved(&mut self, document: DocumentId) {
        if self.is_commit_message(document) {
            if let Some(session) = self.git_work.commit.as_mut() {
                session.written = true;
            }
        }
    }

    /// The message buffer is closing: written means commit, unwritten
    /// means abort (`:q!`).
    pub(super) fn commit_document_closing(&mut self, document: DocumentId) {
        if !self.is_commit_message(document) {
            return;
        }
        let Some(session) = self.git_work.commit.take() else {
            return;
        };
        if !session.written {
            self.message = "commit aborted".into();
            return;
        }
        self.run_porcelain(
            session.repo,
            Porcelain::Commit {
                amend: session.amend,
                message: session.path,
            },
        );
    }

    /// Run one `git` CLI command as a job. One at a time: git's own
    /// index lock would refuse a second anyway.
    pub(super) fn run_porcelain(&mut self, repo: RepoTarget, command: Porcelain) {
        if self.git_work.porcelain.is_some() {
            self.message = "git is still running the previous command".into();
            return;
        }
        let Some(ticket) = self.git_ticket(PorcelainKey {
            repo: repo.clone(),
            command: command.clone(),
        }) else {
            return;
        };
        self.git_work.porcelain = Some(ticket.clone());
        let word = |text: &str| OsString::from(text);
        let argv: Vec<OsString> = match &command {
            Porcelain::Commit { amend, message } => {
                // our template's comment lines start with `#` whatever
                // the user's core.commentChar says
                let mut argv = vec![
                    word("-c"),
                    word("core.commentChar=#"),
                    word("commit"),
                    word("--cleanup=strip"),
                    word("-F"),
                    message.clone().into_os_string(),
                ];
                if *amend {
                    argv.push(word("--amend"));
                }
                argv
            }
            Porcelain::Switch { branch } => vec![word("switch"), word(branch)],
            Porcelain::Create { branch } => vec![word("switch"), word("-c"), word(branch)],
        };
        self.message = match &command {
            Porcelain::Commit { .. } => "committing…".into(),
            Porcelain::Switch { branch } => format!("switching to {branch}…"),
            Porcelain::Create { branch } => format!("creating {branch}…"),
        };
        strop_trace::record_with(
            strop_trace::EventKind::JobStarted,
            || serde_json::json!({"service":"git","request":"porcelain","command":command}),
        );
        let args = ticket.clone();
        self.launch_git_job(
            "git-porcelain",
            "git.porcelain",
            ticket,
            &args,
            GitJob::Porcelain,
            move |cancel| {
                let exec = match GitExec::for_target(&repo, None) {
                    Ok(exec) => exec,
                    Err(error) => {
                        return Outcome::failed(FailureKind::Unavailable, error.to_string())
                    }
                };
                let run = match exec.run_within(&argv, PORCELAIN_DEADLINE, &cancel) {
                    Ok(run) => run,
                    Err(error) => {
                        return Outcome::failed(FailureKind::Unavailable, error.to_string())
                    }
                };
                if !run.success {
                    return Outcome::failed(
                        FailureKind::Exit,
                        summary(&run.stdout, &run.stderr, true),
                    );
                }
                Outcome::Success(PorcelainDone {
                    line: summary(&run.stdout, &run.stderr, false),
                    context: strop_git::Repo::discover(repo.workdir()).map(|repo| repo.context()),
                })
            },
        );
    }

    pub(crate) fn handle_porcelain_completion(
        &mut self,
        completion: Completion<PorcelainKey, PorcelainDone>,
    ) {
        if self.git_work.porcelain.as_ref() != Some(&completion.ticket) {
            trace::services::rejected("git", "porcelain request superseded");
            return;
        }
        self.git_work.porcelain = None;
        self.worker_handles.remove(&completion.ticket.request);
        let key = completion.ticket.key;
        match completion.outcome {
            Outcome::Success(done) => {
                self.message = done.line;
                if let Some(context) = done.context {
                    self.adopt_git_context(context);
                }
                if !matches!(key.command, Porcelain::Commit { .. }) {
                    // another branch's files are under every buffer
                    self.notify_rescan();
                }
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => self.message = "git was cancelled".into(),
        }
        // success or not, hooks may have touched the index and tree
        self.invalidate_git_view();
        self.refresh_status_buffers();
    }

    /// HEAD moved: every view of this repository — the cached context
    /// and each open surface's — reads the new branch and commit.
    fn adopt_git_context(&mut self, context: strop_git::GitContext) {
        if self
            .git
            .as_ref()
            .is_some_and(|current| current.repo == context.repo)
        {
            self.git = Some(context.clone());
        }
        let surfaces: Vec<DocumentId> = self
            .docs
            .iter()
            .filter(|(_, document)| {
                document
                    .git_context()
                    .is_some_and(|current| current.repo == context.repo)
            })
            .map(|(id, _)| id)
            .collect();
        for id in surfaces {
            if let DocumentSource::Surface(surface) = &mut self.doc_mut(id).source {
                surface.context = context.clone();
            }
        }
    }

    /// List the branches on a worker; the picker opens when they land.
    pub(crate) fn open_branches(&mut self) {
        let Some(repo) = self.working_repo("branches") else {
            return;
        };
        if let Some(previous) = self.git_work.branches.take() {
            self.cancel_git_worker(previous.request, CancelReason::Superseded);
        }
        let Some(ticket) = self.git_ticket(BranchKey {
            document: self.current(),
            repo: repo.clone(),
        }) else {
            return;
        };
        self.git_work.branches = Some(ticket.clone());
        let args = ticket.clone();
        let workdir = repo.workdir().to_path_buf();
        self.launch_git_job(
            "git-branches",
            "git.branches",
            ticket,
            &args,
            GitJob::Branches,
            move |cancel| {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                let listed = repo_or_unavailable(&workdir).and_then(|repo| {
                    repo.branches()
                        .map_err(|error| git_failure("branches", error))
                });
                match listed {
                    Ok(branches) => Outcome::Success(branches),
                    Err(failure) => Outcome::Failed {
                        failure,
                        partial: None,
                    },
                }
            },
        );
    }

    pub(crate) fn handle_branches_completion(
        &mut self,
        completion: Completion<BranchKey, Vec<Branch>>,
    ) {
        if self.git_work.branches.as_ref() != Some(&completion.ticket) {
            trace::services::rejected("git", "branch list superseded");
            return;
        }
        self.git_work.branches = None;
        self.worker_handles.remove(&completion.ticket.request);
        // the list belongs to the view that asked: a picker or a
        // command line opened since then keeps the keyboard
        let asked_here = !self.docs.is_empty()
            && self.current() == completion.ticket.key.document
            && self.input_owner() == crate::editor::dispatch::InputOwner::Document;
        if !asked_here {
            trace::services::rejected("git", "branch list owner changed");
            return;
        }
        match completion.outcome {
            Outcome::Success(branches) => {
                let mut items: Vec<Item> = branches
                    .into_iter()
                    .map(|branch| Item {
                        badge: None,
                        text: if branch.current {
                            format!("{} — current", branch.name)
                        } else {
                            branch.name
                        },
                        payload: Payload::Branch(BranchChoice::Switch(branch.switch_to)),
                    })
                    .collect();
                items.push(Item {
                    badge: None,
                    text: CREATE_ROW.into(),
                    payload: Payload::Branch(BranchChoice::Create),
                });
                let mut picker = Picker::new(Kind::Branches, items, false);
                picker.pinned_tail = 1;
                self.set_picker(PickerGlue::diagnostics(picker));
            }
            Outcome::Failed { failure, .. } => self.message = failure.message,
            Outcome::Cancelled(_) => {}
        }
    }

    /// Enter in the branch picker. `typed` is the filter text: the
    /// create row turns it into the new branch's name.
    pub(crate) fn accept_branch(&mut self, choice: BranchChoice, typed: &str) {
        let Some(repo) = self.working_repo("branches") else {
            return;
        };
        let command = match choice {
            BranchChoice::Switch(branch) => Porcelain::Switch { branch },
            BranchChoice::Create if typed.is_empty() => {
                self.message = "type the new branch's name, then choose this row".into();
                return;
            }
            BranchChoice::Create => Porcelain::Create {
                branch: typed.to_string(),
            },
        };
        self.run_porcelain(repo, command);
    }
}
