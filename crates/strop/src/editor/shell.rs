//! Shell escapes: `:!cmd` runs and displays, `|cmd` pipes a range
//! through and replaces it (helix's pipe is the better `!`). Every
//! spawn is an owned worker posting one terminal result onto the
//! event loop (0001 §3, R9 §4/§5) — the input path never waits on a
//! shell, no result mutates a buffer without owning the exact
//! request/document/revision it was admitted against, and a failed
//! or cancelled command never touches user text.

mod jobs;
mod process;
#[cfg(test)]
mod tests;

use strop_core::worker::{self, CancelReason, Outcome, Ticket};

use super::trace;
use super::{Document, Editor};
#[cfg(test)]
pub use jobs::ProcessOutput;
pub use jobs::{ShellIntent, ShellKey, ShellResult};

impl Editor {
    /// `:!cmd`: run `sh -c cmd` in a job; the output buffer opens
    /// when the job lands — automatically only if nothing else
    /// happened since (any input revokes the switch; the output
    /// itself always survives in the background).
    pub(crate) fn shell_run(&mut self, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            self.message = ":! needs a command".into();
            return;
        }
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        // the display's focus token is this request: only the newest
        // display job may still switch the view when it lands
        let intent = ShellIntent {
            ticket: Ticket {
                request,
                key: ShellKey::Display {
                    origin: self.current(),
                    revision: self.buf().revision(),
                    focus: request,
                },
            },
            command: cmd.clone(),
            cwd: self.cwd.clone(),
            original: None,
        };
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"shell","request":request.get(),
                "command":cmd,"cwd":self.cwd.to_string_lossy(),
            })
        });
        self.shell_focus = Some(request);
        self.message = format!("sh: {cmd} …");
        self.launch_shell(intent, None);
    }

    /// `|cmd` (visual) or `|cmd` on a normal line: pipe the range
    /// through the command; stdout replaces it (one undo unit).
    pub(crate) fn pipe_run(&mut self, start: usize, end: usize, cmd: &str) {
        let cmd = cmd.trim().to_string();
        if cmd.is_empty() {
            self.message = "pipe: needs a command".into();
            return;
        }
        // normalize, clamp to the document, then to char boundaries:
        // the captured range is exactly what delivery validates —
        // never the raw arguments (multibyte offsets must not slice)
        let document = self.current();
        let len = self.buf().len_bytes();
        let raw_start = start.min(end).min(len);
        let raw_end = end.max(start).min(len);
        let buf = self.buf();
        let s = buf.clamp_boundary(raw_start);
        let e = buf.clamp_boundary(raw_end);
        let original = buf.text().byte_slice(s..e).to_string();
        let revision = buf.revision();
        let request = match self.worker_ids.allocate() {
            Ok(request) => request,
            Err(error) => {
                self.message = error.message;
                return;
            }
        };
        let intent = ShellIntent {
            ticket: Ticket {
                request,
                key: ShellKey::Pipe {
                    document,
                    revision,
                    start: s,
                    end: e,
                },
            },
            command: cmd.clone(),
            cwd: self.cwd.clone(),
            original: Some(original),
        };
        strop_trace::record_with(strop_trace::EventKind::JobStarted, || {
            serde_json::json!({
                "service":"pipe","request":request.get(),"command":cmd,
                "start_byte":s,"end_byte":e,"revision":revision.get(),
            })
        });
        self.message = format!("| {cmd} …");
        let input = intent.original.clone();
        self.launch_shell(intent, input);
    }

    /// Register the intent, then launch the worker against its exact
    /// ticket — registration before launch, one terminal result per
    /// request, panic and thread-start failures included.
    fn launch_shell(&mut self, intent: ShellIntent, input: Option<String>) {
        let request = intent.ticket.request;
        let command = intent.command.clone();
        let cwd = intent.cwd.clone();
        let ticket = intent.ticket.clone();
        self.shell_requests.insert(request, intent);
        let operation = match ticket.key {
            ShellKey::Display { .. } => "shell.display",
            ShellKey::Pipe { .. } => "shell.pipe",
        };
        match self.tape.request(operation, &self.shell_requests[&request]) {
            Ok(false) => return,
            Ok(true) => {}
            Err(error) => {
                self.handle_shell_result(ShellResult {
                    ticket,
                    outcome: Outcome::failed(
                        strop_core::worker::FailureKind::Protocol,
                        error.to_string(),
                    ),
                });
                return;
            }
        }
        let tx = self.shell_tx.clone();
        let handle = worker::spawn(
            "strop-shell",
            move |outcome| {
                let _ = tx.send(ShellResult { ticket, outcome });
            },
            move |token| {
                if token.is_cancelled() {
                    return Outcome::Cancelled(CancelReason::OwnerClosed);
                }
                process::run_shell(&command, &cwd, input, &token)
            },
        );
        self.worker_handles.insert(request, handle);
    }

    /// Any subsequent user input revokes a pending display switch —
    /// the output buffer still lands, but in the background (Main
    /// calls this at `feed_inner` entry; the `:!`/`|` dispatch that
    /// wants the switch registers afterwards and re-arms it).
    pub(crate) fn revoke_shell_focus(&mut self) {
        self.shell_focus = None;
    }

    /// A closed document invalidates its pipe owners (a reused arena
    /// slot must never receive a stale replacement) and any display
    /// focus aimed at it. Display output requests survive — their
    /// buffers are editor-owned. Main calls this from the document
    /// close path before removal.
    pub(crate) fn shell_document_closed(&mut self, document: strop_core::id::DocumentId) {
        if let Some(focus) = self.shell_focus {
            let origin_lost = self
                .shell_requests
                .get(&focus)
                .is_some_and(|intent| {
                    matches!(&intent.ticket.key, ShellKey::Display { origin, .. } if *origin == document)
                });
            if origin_lost {
                self.shell_focus = None;
            }
        }
        let stale: Vec<_> = self
            .shell_requests
            .values()
            .filter(|intent| {
                matches!(&intent.ticket.key, ShellKey::Pipe { document: d, .. } if *d == document)
            })
            .map(|intent| intent.ticket.request)
            .collect();
        for request in stale {
            self.shell_requests.remove(&request);
            if let Some(handle) = self.worker_handles.remove(&request) {
                handle.cancel(CancelReason::OwnerClosed);
            }
        }
    }

    /// One shell job result (TUI events land here directly — 0018).
    /// The registry is the gate: only the exact admitted ticket may
    /// publish, exactly once; duplicates and stale results only trace
    /// their rejection.
    pub(crate) fn handle_shell_result(&mut self, result: ShellResult) {
        trace::services::shell(&result);
        let request = result.ticket.request;
        if !self
            .shell_requests
            .get(&request)
            .is_some_and(|intent| intent.ticket == result.ticket)
        {
            trace::services::rejected("shell", "request already settled or cancelled");
            return;
        }
        let Some(intent) = self.shell_requests.remove(&request) else {
            return;
        };
        self.worker_handles.remove(&request);
        if self.docs.is_empty() {
            return;
        }
        match intent.ticket.key {
            ShellKey::Display {
                origin,
                revision,
                focus,
            } => {
                let may_focus = self.shell_focus == Some(focus)
                    && self.current() == origin
                    && self
                        .docs
                        .get(origin)
                        .is_some_and(|d| d.buf.revision() == revision);
                if self.shell_focus == Some(focus) {
                    self.shell_focus = None;
                }
                let output = match result.outcome {
                    Outcome::Success(output) => output,
                    Outcome::Failed { failure, partial } => {
                        // a failed command's output is still shown —
                        // failure explains itself in the stderr section
                        let mut output = partial.unwrap_or_default();
                        if output.stderr.is_empty() {
                            output.stderr = failure.message;
                        }
                        output
                    }
                    // cancellation revoked publication: no buffer, no switch
                    Outcome::Cancelled(_) => return,
                };
                let text = if output.stderr.is_empty() {
                    output.stdout
                } else {
                    format!("{}\n--- stderr ---\n{}", output.stdout, output.stderr)
                };
                let mut buffer = strop_core::Buffer::from_text(&text);
                buffer.name = Some(format!("sh: {}", intent.command));
                let doc = self.docs.insert(Document::output(buffer));
                self.generation += 1;
                self.mru.push(doc);
                if may_focus {
                    self.switch_to(doc);
                    self.set_head(0);
                    self.view_mut().view_top = 0;
                    self.message = format!("sh: {} — q closes", intent.command);
                }
            }
            ShellKey::Pipe {
                document,
                revision,
                start,
                end,
            } => {
                let Some(doc) = self.docs.get(document) else {
                    trace::services::rejected("shell", "pipe document closed");
                    return;
                };
                if doc.buf.revision() != revision {
                    if self.current() == document {
                        self.message = "pipe: text changed under the job — skipped".into();
                    }
                    trace::services::rejected("shell", "pipe revision changed");
                    return;
                }
                if doc.buf.readonly {
                    if self.current() == document {
                        self.message = "pipe: readonly buffer".into();
                    }
                    trace::services::rejected("shell", "pipe readonly buffer");
                    return;
                }
                let output = match result.outcome {
                    Outcome::Success(output) => output.stdout,
                    Outcome::Failed { failure, partial } => {
                        // a failed command never touches the source —
                        // its stderr explains itself in the message line
                        if self.current() == document {
                            let detail = partial
                                .as_ref()
                                .map(|partial| partial.stderr.trim())
                                .filter(|stderr| !stderr.is_empty())
                                .unwrap_or(&failure.message);
                            self.message = format!("pipe failed: {detail}");
                        }
                        trace::services::rejected("shell", "pipe command failed");
                        return;
                    }
                    Outcome::Cancelled(_) => return,
                };
                let Some(original) = intent.original else {
                    trace::services::rejected("shell", "pipe intent lost its captured text");
                    return;
                };
                // never clobber: the range must still hold what we piped
                let buf = &doc.buf;
                if end > buf.len_bytes()
                    || !buf.is_boundary(start)
                    || !buf.is_boundary(end)
                    || buf.text().byte_slice(start..end) != original.as_str()
                {
                    if self.current() == document {
                        self.message = "pipe: text changed under the job — skipped".into();
                    }
                    trace::services::rejected("shell", "pipe range changed");
                    return;
                }
                // linewise ranges keep their newline; charwise gets
                // the command's trailing newline trimmed
                let replacement = if original.ends_with('\n') {
                    output
                } else {
                    output.strip_suffix('\n').unwrap_or(&output).to_owned()
                };
                // one atomic range replacement against the captured
                // revision — one undo unit for the whole pipe
                let changes = super::transact::ChangeSet {
                    edits: vec![strop_core::Replacement::new(
                        strop_core::Range::charwise(start, end),
                        replacement,
                    )],
                    undo_open: false,
                };
                if let Err(error) = self.apply(document, revision, changes) {
                    if self.current() == document {
                        self.message = format!("pipe: {error}");
                    }
                    trace::services::rejected("shell", "pipe replacement rejected");
                    return;
                }
                if document == self.current() {
                    self.set_head(self.buf().clamp_boundary(start));
                    self.clamp_cursor();
                    self.flash(strop_core::Range::charwise(self.head(), self.head()));
                    self.message = "piped".into();
                }
            }
        }
    }
}
