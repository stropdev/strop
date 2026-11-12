//! The commit browser and a file's history (`Space g l`, `Space g h`,
//! the line-range history): one streamed job per surface. Rows arrive
//! in pages and are only ever appended, in git's order, so nothing above
//! the cursor moves and no row is replaced (0101 §2.2). The first page
//! replaces `loading log…`.

use std::sync::mpsc::Sender;
use std::time::{Duration, Instant};

use strop_core::id::DocumentId;
use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};
use strop_git::exec::GitExec;
use strop_git::log::{self, HistoryRead, LogRow};

use super::types::{GitJob, LogDone, LogKey, LogRequest};
use super::{graph, jobs, Surface};
use crate::editor::{trace, Editor};

/// The most commits a history surface lists.
const HISTORY_ROWS: usize = 200;
/// The first page goes out once this many rows are in…
const FIRST_PAGE_ROWS: usize = 50;
/// …or this much time has passed; later pages at this interval.
const PAGE_INTERVAL: Duration = Duration::from_millis(30);

/// Collects rows on the worker and posts them onto the event loop in
/// pages; what is left when git exits goes in the final completion.
struct Pager {
    tx: Sender<GitJob>,
    ticket: Ticket<LogKey>,
    started: Instant,
    last: Option<Instant>,
    rows: Vec<LogRow>,
}

impl Pager {
    fn take(&mut self, mut rows: Vec<LogRow>) {
        self.rows.append(&mut rows);
        let due = match self.last {
            None => self.rows.len() >= FIRST_PAGE_ROWS || self.started.elapsed() >= PAGE_INTERVAL,
            Some(last) => last.elapsed() >= PAGE_INTERVAL,
        };
        if due && !self.rows.is_empty() {
            let page = std::mem::take(&mut self.rows);
            let _ = self.tx.send(GitJob::LogPage(self.ticket.clone(), page));
            self.last = Some(Instant::now());
        }
    }
}

/// Open a history surface and start its read.
pub(super) fn open(
    editor: &mut Editor,
    file_scoped: bool,
    focus: Option<String>,
    range: Option<(usize, usize)>,
) {
    let Some(context) = editor.git_context().cloned() else {
        editor.message = "not a git repo".into();
        return;
    };
    // `git log -L` speaks file line numbers: a partial remote
    // window's lines are window-relative, and pretending they are
    // file coordinates would show the history of the WRONG lines
    // (0036: partial windows never masquerade as full-file Git
    // inputs).
    let repo = context.repo.clone();
    let file = if file_scoped {
        editor.current_buffer_rel(&repo)
    } else {
        None
    };
    // a file's history is named for its file (0100 §4.3)
    let name = match (&file, range) {
        (Some(file), Some((start, end))) => {
            format!("git log · {}:{start}-{end}", file.display())
        }
        (Some(file), None) => format!("git log · {}", file.display()),
        (None, _) => "git log".to_owned(),
    };
    editor.push_surface(
        Some(&name),
        ropey::Rope::from_str("loading log…"),
        Surface::CommitLog {
            rows: vec![],
            focus,
            return_to: None,
        },
    );
    // the new surface document owns its request; registration
    // happens before launch (replay contract)
    let doc = editor.current();
    let revision = editor.buf().revision();
    let key = LogKey {
        document: doc,
        revision,
        repo: repo.clone(),
    };
    let Some(ticket) = editor.git_ticket(key) else {
        return;
    };
    editor.log_requests.insert(
        doc,
        LogRequest {
            ticket: ticket.clone(),
            expected: revision,
            published: false,
        },
    );
    strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
        serde_json::json!({
            "service":"git","request":"log",
            "target":if repo.is_remote() { "remote" } else { "local" },
            "document":{"slot":doc.index(),"generation":doc.generation()},
            "revision":revision.get(),
            "path":file.as_ref().map(|p|p.to_string_lossy()),
        })
    });
    let args = (
        ticket.clone(),
        file.clone().map(trace::services::NativePath),
        range,
    );
    let client = editor.remote_client();
    let graphs = editor.state_dir().map(strop_git::graph::graphs_root);
    let tx = editor.git_tx.clone();
    let pager_ticket = ticket.clone();
    editor.launch_git_job(
        "git-log",
        "git.log",
        ticket,
        &args,
        GitJob::Log,
        move |cancel| {
            if cancel.is_cancelled() {
                return Outcome::Cancelled(CancelReason::Superseded);
            }
            let host = jobs::repo_host(&client, &repo, &cancel);
            let exec = match host.and_then(|host| {
                GitExec::for_target(&repo, host.as_ref()).map_err(|error| error.to_string())
            }) {
                Ok(exec) => exec,
                Err(error) => return Outcome::failed(FailureKind::Unavailable, error.to_string()),
            };
            // strop's private commit-graph, on a local repository only
            let side = match (&repo, &graphs) {
                (strop_git::RepoTarget::Local { workdir }, Some(graphs)) => {
                    strop_git::graph::for_history(graphs, workdir)
                }
                _ => None,
            };
            let started = Instant::now();
            let mut pager = Pager {
                tx,
                ticket: pager_ticket,
                started,
                last: None,
                rows: Vec::new(),
            };
            let read = HistoryRead {
                exec: &exec,
                cancel: &cancel,
                max: HISTORY_ROWS,
                file: file.as_deref(),
                range,
                graph: side.as_deref(),
            };
            match log::read_history(&read, |rows| pager.take(rows)) {
                Ok(()) => Outcome::Success(LogDone {
                    rows: pager.rows,
                    took: started.elapsed(),
                }),
                Err(message) => Outcome::failed(FailureKind::Exit, message),
            }
        },
    );
}

/// The request a log result names, when it is still the one the
/// surface waits for and the surface is still at the revision the
/// request expects.
fn live_request(editor: &Editor, ticket: &Ticket<LogKey>) -> Option<LogRequest> {
    let doc = ticket.key.document;
    let request = editor
        .log_requests
        .get(&doc)
        .filter(|request| request.ticket == *ticket)?;
    // only the still-live CommitLog surface at the request's
    // revision accepts rows — a closed/recycled slot cannot
    editor
        .docs
        .get(doc)
        .is_some_and(|d| {
            d.buf.revision() == request.expected
                && matches!(d.surface_payload(), Some(Surface::CommitLog { .. }))
        })
        .then(|| request.clone())
}

pub(super) fn handle_page(editor: &mut Editor, ticket: Ticket<LogKey>, rows: Vec<LogRow>) {
    let Some(request) = live_request(editor, &ticket) else {
        trace::services::rejected("git", "log page for a superseded request or surface");
        return;
    };
    let doc = ticket.key.document;
    if !publish_rows(editor, doc, rows, request.published) {
        return;
    }
    let expected = editor.doc(doc).buf.revision();
    if let Some(request) = editor.log_requests.get_mut(&doc) {
        request.expected = expected;
        request.published = true;
    }
}

/// Land rows in the surface buffer: the first rows replace the
/// loading text, later ones are appended. An empty successful log
/// is a completed empty list. Returns false when the buffer refused.
fn publish_rows(editor: &mut Editor, doc: DocumentId, rows: Vec<LogRow>, append: bool) -> bool {
    let mut text = String::new();
    for row in &rows {
        text.push_str(&row.text);
        text.push('\n');
    }
    let landed = if append {
        let end = editor.doc(doc).buf.len_bytes();
        editor
            .doc_mut(doc)
            .buf
            .system_edit()
            .replace(strop_core::Range::charwise(end, end), &text)
    } else {
        if text.is_empty() {
            text.push('\n');
        }
        editor.replace_system(doc, &text)
    };
    if let Err(error) = landed {
        trace::services::rejected("git", "log publish failed");
        editor.message = format!("git log publish failed: {error}");
        return false;
    }
    let mut focus_row = None;
    if let Some(Some(Surface::CommitLog {
        rows: slot, focus, ..
    })) = editor.docs.get_mut(doc).map(|d| d.surface_payload_mut())
    {
        let before = slot.len();
        if let Some(found) = focus.as_deref().and_then(|sha| {
            rows.iter()
                .position(|row| row.sha() == Some(sha))
                .map(|row| before + row)
        }) {
            focus_row = Some(found);
            *focus = None;
        }
        slot.extend(rows);
    }
    if let Some(row) = focus_row {
        // the blame dive asked for this commit: land on it (only
        // when the browser is still what's being driven)
        if editor.current() == doc {
            let at = editor.doc(doc).buf.line_start(row);
            editor.set_head(at);
            editor.view_mut().view_top = row;
        }
    }
    true
}

/// Replace a log surface's loading text with a terminal status;
/// rows published by earlier pages go with the text they were. The
/// surface never shows "loading" forever.
fn publish_status(editor: &mut Editor, doc: DocumentId, status: &str) {
    if let Err(error) = editor.replace_system(doc, status) {
        trace::services::rejected("git", "log status publish failed");
        editor.message = format!("git log publish failed: {error}");
        return;
    }
    if let Some(Some(Surface::CommitLog { rows, .. })) =
        editor.docs.get_mut(doc).map(|d| d.surface_payload_mut())
    {
        rows.clear();
    }
}

pub(super) fn handle_completion(editor: &mut Editor, completion: Completion<LogKey, LogDone>) {
    let doc = completion.ticket.key.document;
    let live = live_request(editor, &completion.ticket);
    if editor
        .log_requests
        .get(&doc)
        .is_some_and(|request| request.ticket == completion.ticket)
    {
        editor.log_requests.remove(&doc);
        editor.worker_handles.remove(&completion.ticket.request);
    }
    let Some(request) = live else {
        trace::services::rejected("git", "log request superseded, surface changed or closed");
        return;
    };
    match completion.outcome {
        Outcome::Success(done) => {
            if !request.published || !done.rows.is_empty() {
                publish_rows(editor, doc, done.rows, request.published);
            }
            graph::history_read(editor, &completion.ticket.key.repo, done.took);
        }
        Outcome::Failed { failure, .. } => {
            publish_status(
                editor,
                doc,
                &format!("git log failed: {}\n", failure.message),
            );
            if !editor.docs.is_empty() && editor.current() == doc {
                editor.message = failure.message;
            }
        }
        Outcome::Cancelled(_) => publish_status(editor, doc, "git log cancelled\n"),
    }
}
