//! When strop builds its private commit-graph (0101 §2.1). The graph
//! itself, and why it can not change a row, are `strop_git::graph`'s.
//!
//! The trigger is the measurement: a history read on a local repository
//! that took git more than [`SLOW`] asks for a build, and the build
//! starts after that event — never from a key's dispatch. One build in
//! the whole editor, on its own worker; nothing waits for it: a press
//! during a build runs git as before, and the press after it is fast.

use std::collections::HashMap;
use std::path::PathBuf;
use std::time::Duration;

use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_git::graph::{self as side, Step};
use strop_git::RepoTarget;

use super::types::{GitJob, GraphBuilt, GraphKey};
use crate::editor::{trace, Editor};

/// A history read slower than this builds a graph.
pub(crate) const SLOW: Duration = Duration::from_millis(100);

/// What a session remembers about one repository's graph.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Memo {
    /// Built at this `HEAD`: a read slow again at the same `HEAD` builds
    /// nothing (the graph cannot help it more).
    Built(Option<String>),
    /// The repository keeps a commit-graph of its own.
    Gits,
    /// A step failed (an old git refusing an option): not tried again.
    Failed,
}

/// The build state, one small record per repository.
#[derive(Debug, Default)]
pub(crate) struct GraphState {
    /// A slow read asked for a build of this repository.
    wanted: Option<RepoTarget>,
    building: Option<Ticket<GraphKey>>,
    memo: HashMap<PathBuf, Memo>,
}

impl GraphState {
    pub(crate) fn building(&self) -> bool {
        self.building.is_some()
    }
}

/// A history read finished: on a local repository, slower than [`SLOW`],
/// it asks for a build. The build starts in [`consider`], after the event.
pub(crate) fn history_read(editor: &mut Editor, repo: &RepoTarget, took: Duration) {
    if took > SLOW && matches!(repo, RepoTarget::Local { .. }) {
        editor.git_work.graph.wanted = Some(repo.clone());
    }
}

/// After every event: start the build a slow read asked for, unless one
/// is running, there is no state directory, or this session already
/// built for this repository at this `HEAD` (or learned it should not).
pub(crate) fn consider(editor: &mut Editor) {
    if editor.git_work.graph.wanted.is_none() || editor.git_work.graph.building() {
        return;
    }
    let Some(repo) = editor.git_work.graph.wanted.take() else {
        return;
    };
    let Some(graphs) = editor.state_dir().map(side::graphs_root) else {
        return;
    };
    if editor.finishing {
        return;
    }
    let head = editor
        .git
        .as_ref()
        .filter(|context| context.repo == repo)
        .and_then(|context| context.head_sha.clone());
    let workdir = repo.workdir().to_path_buf();
    match editor.git_work.graph.memo.get(&workdir) {
        Some(Memo::Gits | Memo::Failed) => return,
        Some(Memo::Built(built)) if *built == head => return,
        _ => {}
    }
    let key = GraphKey {
        repo: repo.clone(),
        head,
    };
    let Some(ticket) = editor.git_ticket(key) else {
        return;
    };
    editor.git_work.graph.building = Some(ticket.clone());
    strop_trace::record_with(
        strop_trace::EventKind::JobStarted,
        || serde_json::json!({"service":"git","request":"graph"}),
    );
    let args = ticket.clone();
    editor.launch_git_job(
        "git-graph",
        "git.graph",
        ticket,
        &args,
        GitJob::Graph,
        move |cancel| {
            let Some(objects) = side::common_objects(&workdir) else {
                return Outcome::failed(FailureKind::Unavailable, "not a git repository");
            };
            if side::owns_graph(&objects) {
                return Outcome::Success(GraphBuilt::Gits);
            }
            let dir = side::side_dir(&graphs, &objects);
            side::evict(&graphs, &dir, side::KEEP);
            for step in [Step::Reachable, Step::ChangedPaths] {
                if cancel.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::Superseded);
                }
                if let Err(message) = side::build(step, &workdir, &dir, &cancel) {
                    return Outcome::failed(FailureKind::Exit, message);
                }
            }
            Outcome::Success(GraphBuilt::Built)
        },
    );
}

/// A document closed: a build for a repository no open buffer belongs
/// to any more is cancelled.
pub(crate) fn document_closed(editor: &mut Editor) {
    let Some(ticket) = editor.git_work.graph.building.clone() else {
        return;
    };
    let workdir = ticket.key.repo.workdir();
    let in_use = editor.docs.iter().any(|(_, document)| {
        document
            .git_context()
            .is_some_and(|context| context.repo == ticket.key.repo)
            || document
                .buf
                .path
                .as_deref()
                .is_some_and(|path| editor.cwd.join(path).starts_with(workdir))
    });
    if !in_use {
        editor.git_work.graph.building = None;
        editor.cancel_git_worker(ticket.request, CancelReason::Dismissed);
    }
}

pub(crate) fn handle_completion(editor: &mut Editor, completion: Completion<GraphKey, GraphBuilt>) {
    if editor.git_work.graph.building.as_ref() != Some(&completion.ticket) {
        trace::services::rejected("git", "graph build superseded or cancelled");
        return;
    }
    editor.git_work.graph.building = None;
    editor.worker_handles.remove(&completion.ticket.request);
    let key = completion.ticket.key;
    let memo = match completion.outcome {
        Outcome::Success(GraphBuilt::Built) => Memo::Built(key.head),
        Outcome::Success(GraphBuilt::Gits) => Memo::Gits,
        Outcome::Failed { failure, .. } => {
            strop_trace::record_with(
                strop_trace::EventKind::JobFinished,
                || serde_json::json!({"service":"git","result":"graph-failed","message":failure.message}),
            );
            Memo::Failed
        }
        Outcome::Cancelled(_) => return,
    };
    editor
        .git_work
        .graph
        .memo
        .insert(key.repo.workdir().to_path_buf(), memo);
}

#[cfg(test)]
mod tests;
