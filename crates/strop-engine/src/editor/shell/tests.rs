//! Deterministic shell lifecycle tests (R9 §4/§5/§6). Injected
//! completions exercise the real handler against real registered
//! intents; real-subprocess cases use one blocking `recv` per result
//! — no sleep loops, no polling, no environment probing.

use std::time::Duration;

use strop_core::worker::{CancelReason, Completion, FailureKind, Outcome, Ticket};

use super::jobs::{ProcessOutput, ShellIntent, ShellKey, ShellResult};
use crate::editor::{Document, Editor, Key};

fn editor_with(text: &str) -> Editor {
    Editor::new(strop_core::Buffer::from_text(text))
}

fn key(ch: char) -> Key {
    Key::Char(ch)
}

/// Register a display intent exactly as `shell_run` would (minus the
/// spawn): ticketed, with the focus token = the request.
fn register_display(e: &mut Editor) -> Ticket<ShellKey> {
    let request = e.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: ShellKey::Display {
            origin: e.current(),
            revision: e.buf().revision(),
            focus: request,
        },
    };
    e.shell_requests.insert(
        request,
        ShellIntent {
            ticket: ticket.clone(),
            command: "injected".into(),
            cwd: e.cwd.clone(),
            original: None,
        },
    );
    ticket
}

/// Register a pipe intent exactly as `pipe_run` would (minus the
/// spawn): normalized range, captured original, current revision.
fn register_pipe(e: &mut Editor, start: usize, end: usize) -> Ticket<ShellKey> {
    let original = e.buf().text().byte_slice(start..end).to_string();
    let request = e.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: ShellKey::Pipe {
            document: e.current(),
            revision: e.buf().revision(),
            start,
            end,
        },
    };
    e.shell_requests.insert(
        request,
        ShellIntent {
            ticket: ticket.clone(),
            command: "injected".into(),
            cwd: e.cwd.clone(),
            original: Some(original),
        },
    );
    ticket
}

fn register_whole_pipe(editor: &mut Editor) -> Ticket<ShellKey> {
    let end = editor.buf().len_bytes();
    register_pipe(editor, 0, end)
}

fn deliver(e: &mut Editor, ticket: Ticket<ShellKey>, outcome: Outcome<ProcessOutput>) {
    e.handle_shell_result(Completion { ticket, outcome });
}

fn success(stdout: &str) -> Outcome<ProcessOutput> {
    Outcome::Success(ProcessOutput {
        stdout: stdout.into(),
        stderr: String::new(),
    })
}

fn failed(kind: FailureKind, message: &str, stderr: &str) -> Outcome<ProcessOutput> {
    Outcome::Failed {
        failure: strop_core::worker::Failure::new(kind, message),
        partial: Some(ProcessOutput {
            stdout: String::new(),
            stderr: stderr.into(),
        }),
    }
}

/// Every `sh: …` output document with its text.
fn sh_outputs(e: &Editor) -> Vec<String> {
    e.docs
        .iter()
        .filter(|(_, d)| {
            d.buf
                .name
                .as_deref()
                .is_some_and(|name| name.starts_with("sh: "))
        })
        .map(|(_, d)| d.buf.text().to_string())
        .collect()
}

/// Blocking barrier for one real worker result — deterministic, no
/// retry loop.
fn next_shell_result(e: &Editor) -> ShellResult {
    e.shell_rx
        .as_ref()
        .unwrap()
        .recv_timeout(Duration::from_secs(30))
        .expect("shell worker settles")
}

// ---- injected completions: ownership, staleness, user text ----

#[test]
fn display_reorder_cannot_steal_focus_and_duplicates_die() {
    let mut e = editor_with("source\n");
    let a = register_display(&mut e);
    let b = register_display(&mut e);
    e.shell_focus = Some(b.request);
    deliver(&mut e, b.clone(), success("B\n"));
    assert!(e.buf().text().to_string().lines().any(|line| line == "B"));
    assert!(e.buf().readonly);
    assert!(e.message.contains("q closes"));
    // the older job lands later: output preserved, view untouched
    let (owner, message) = (e.current(), e.message.clone());
    deliver(&mut e, a.clone(), success("A\n"));
    assert_eq!(e.current(), owner);
    assert_eq!(e.message, message);
    assert!(sh_outputs(&e)
        .iter()
        .any(|text| text.lines().any(|line| line == "A")));
    // duplicate delivery of the consumed ticket changes nothing
    deliver(&mut e, b, success("dup\n"));
    assert!(!sh_outputs(&e)
        .iter()
        .any(|text| text.lines().any(|line| line == "dup")));
    assert!(e.shell_requests.is_empty());
}

#[test]
fn display_failure_keeps_partial_output_and_explains_itself() {
    let mut e = editor_with("source\n");
    let a = register_display(&mut e);
    e.shell_focus = Some(a.request);
    deliver(
        &mut e,
        a,
        Outcome::Failed {
            failure: strop_core::worker::Failure::new(
                FailureKind::Exit,
                "shell exited with exit status: 3",
            ),
            partial: Some(ProcessOutput {
                stdout: "partial\n".into(),
                stderr: "diagnostic\n".into(),
            }),
        },
    );
    let text = e.buf().text().to_string();
    assert!(text.contains("partial\n"));
    assert!(text.lines().any(|line| line == "diagnostic"));
    assert!(text.contains("exit status: 3"));
}

#[test]
fn display_cancellation_publishes_nothing() {
    let mut e = editor_with("source\n");
    let a = register_display(&mut e);
    e.shell_focus = Some(a.request);
    deliver(&mut e, a, Outcome::Cancelled(CancelReason::Dismissed));
    assert!(sh_outputs(&e).is_empty());
    assert!(e.shell_requests.is_empty());
    assert_eq!(e.shell_focus, None);
}

#[test]
fn unowned_result_is_rejected_without_side_effects() {
    let mut e = editor_with("keep\n");
    let ghost = Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: ShellKey::Pipe {
            document: e.current(),
            revision: e.buf().revision(),
            start: 0,
            end: e.buf().len_bytes(),
        },
    };
    let before = e.buf().text().to_string();
    deliver(&mut e, ghost, success("REPLACED\n"));
    assert_eq!(e.buf().text().to_string(), before);
    assert!(!e.message.contains("piped"));
}

#[test]
fn pipe_failure_never_mutates_and_reports_stderr() {
    let mut e = editor_with("keep me\n");
    let a = register_whole_pipe(&mut e);
    deliver(
        &mut e,
        a,
        failed(
            FailureKind::Exit,
            "shell exited with exit status: 1",
            "boom\n",
        ),
    );
    assert_eq!(e.buf().text().to_string(), "keep me\n");
    assert_eq!(e.message, "pipe failed: boom");
    assert!(e.shell_requests.is_empty(), "failed request is consumed");
}

#[test]
fn pipe_failure_without_stderr_reports_the_failure() {
    let mut e = editor_with("keep\n");
    let a = register_whole_pipe(&mut e);
    deliver(&mut e, a, failed(FailureKind::Panic, "worker panicked", ""));
    assert_eq!(e.buf().text().to_string(), "keep\n");
    assert_eq!(e.message, "pipe failed: worker panicked");
}

#[test]
fn pipe_revision_rejects_after_edit_and_undo_restores_bytes() {
    let mut e = editor_with("beta\nalpha\n");
    let a = register_whole_pipe(&mut e);
    // edit, then undo: bytes identical, revision moved on
    e.feed(key('x'));
    e.feed(key('u'));
    assert_eq!(e.buf().text().to_string(), "beta\nalpha\n");
    deliver(&mut e, a, success("alpha\nbeta\n"));
    assert_eq!(
        e.buf().text().to_string(),
        "beta\nalpha\n",
        "stale pipe must not apply even when the bytes match again"
    );
    assert!(e.message.contains("changed under the job"));
}

#[test]
fn pipe_range_mismatch_rejects_at_same_revision() {
    let mut e = editor_with("ab\n");
    let request = e.worker_ids.allocate().unwrap();
    let ticket = Ticket {
        request,
        key: ShellKey::Pipe {
            document: e.current(),
            revision: e.buf().revision(),
            start: 0,
            end: 2,
        },
    };
    // registry says the captured original was different bytes
    e.shell_requests.insert(
        request,
        ShellIntent {
            ticket: ticket.clone(),
            command: "injected".into(),
            cwd: e.cwd.clone(),
            original: Some("WRONG".into()),
        },
    );
    deliver(&mut e, ticket, success("X"));
    assert_eq!(e.buf().text().to_string(), "ab\n");
    assert!(e.message.contains("changed under the job"));
}

#[test]
fn pipe_duplicate_applies_once() {
    let mut e = editor_with("old\n");
    let a = register_whole_pipe(&mut e);
    deliver(&mut e, a.clone(), success("new\n"));
    assert_eq!(e.buf().text().to_string(), "new\n");
    deliver(&mut e, a, success("new\n"));
    assert_eq!(e.buf().text().to_string(), "new\n");
    e.feed(key('u'));
    assert_eq!(e.buf().text().to_string(), "old\n", "exactly one undo unit");
}

#[test]
fn pipe_readonly_refuses() {
    let mut e = editor_with("locked\n");
    let a = register_whole_pipe(&mut e);
    e.buf_mut().readonly = true;
    deliver(&mut e, a, success("X\n"));
    assert_eq!(e.buf().text().to_string(), "locked\n");
    assert_eq!(e.message, "pipe: readonly buffer");
}

#[test]
fn trailing_newline_policy_linewise_keeps_charwise_trims() {
    // linewise range (original ends with \n): output newline kept
    let mut e = editor_with("a\nb\n");
    let linewise = register_whole_pipe(&mut e);
    deliver(&mut e, linewise, success("X\nY\n"));
    assert_eq!(e.buf().text().to_string(), "X\nY\n");
    // charwise (no trailing \n): exactly one trailing LF trimmed
    let mut e = editor_with("ab");
    let charwise = register_pipe(&mut e, 0, 2);
    deliver(&mut e, charwise, success("XY\n"));
    assert_eq!(e.buf().text().to_string(), "XY");
    let mut e = editor_with("ab");
    let bare = register_pipe(&mut e, 0, 2);
    deliver(&mut e, bare, success("XY"));
    assert_eq!(e.buf().text().to_string(), "XY");
}

#[test]
fn closed_document_invalidation_kills_pipe_owner() {
    let mut e = editor_with("main\n");
    let main = e.current();
    let other = e
        .docs
        .insert(Document::scratch(strop_core::Buffer::from_text("piped\n")));
    e.switch_to(other);
    let stale = register_whole_pipe(&mut e);
    e.switch_to(main);
    e.shell_document_closed(other);
    assert!(
        e.shell_requests.is_empty(),
        "owner invalidated synchronously"
    );
    // a late success for the dead owner is rejected by the registry
    deliver(&mut e, stale, success("LATE\n"));
    assert_eq!(e.buf().text().to_string(), "main\n");
}

// ---- real subprocesses through the real producer/handler path ----

#[test]
fn sort_pipe_replaces_selection_in_one_undo_unit() {
    let mut e = editor_with("beta\nalpha\n");
    e.pipe_run(0, e.buf().len_bytes(), "sort");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "alpha\nbeta\n");
    assert_eq!(e.message, "piped");
    e.feed(key('u'));
    assert_eq!(e.buf().text().to_string(), "beta\nalpha\n");
}

#[test]
fn failed_exit_never_touches_source() {
    let mut e = editor_with("keep me\n");
    e.pipe_run(0, e.buf().len_bytes(), "exit 3");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "keep me\n");
    assert!(e.message.starts_with("pipe failed"), "{}", e.message);
}

#[test]
fn bang_opens_focused_output_buffer_with_both_streams() {
    let mut e = editor_with("x\n");
    e.shell_run("echo out; echo err 1>&2");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    let text = e.buf().text().to_string();
    assert!(text.lines().any(|line| line == "out"));
    assert!(text.contains("--- stderr ---"));
    assert!(text.lines().any(|line| line == "err"));
    assert!(e.buf().readonly);
    let output = e.current();
    e.feed(key('q')); // closes like any readonly buffer
    assert!(e.docs.get(output).is_none());
}

#[test]
fn input_between_bang_and_landing_puts_output_in_background() {
    let mut e = editor_with("x\n");
    e.shell_run("echo late");
    e.feed(key('j')); // user input revokes the focus switch
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().name, None, "no focus steal after intervening input");
    assert!(sh_outputs(&e).iter().any(|text| text.contains("late")));
}

#[test]
fn multibyte_offsets_clamp_to_char_boundaries() {
    let mut e = editor_with("éx\n"); // é is two bytes
                                     // raw 1..3 straddles é's tail: registration clamps to 0..3
    e.pipe_run(1, 3, "tr x Z");
    let captured = e
        .shell_requests
        .values()
        .next()
        .map(|intent| (intent.ticket.key.clone(), intent.original.clone()))
        .unwrap();
    match captured.0 {
        ShellKey::Pipe { start, end, .. } => assert_eq!((start, end), (0, 3)),
        other => panic!("expected pipe key, got {other:?}"),
    }
    assert_eq!(captured.1.as_deref(), Some("éx"));
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "éZ\n");
    // reversed arguments normalize to the same range
    let mut e = editor_with("éx\n");
    e.pipe_run(3, 1, "tr x Z");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "éZ\n");
}

#[test]
fn big_cat_pipe_does_not_deadlock() {
    let mut e = editor_with(&"payload line\n".repeat(64 * 1024)); // ~832 KiB
    let original = e.buf().text().to_string();
    e.pipe_run(0, e.buf().len_bytes(), "cat");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), original);
    e.feed(key('u'));
    assert_eq!(e.buf().text().to_string(), original);
}

#[test]
fn cancellation_settles_promptly_then_next_command_progresses() {
    let mut e = editor_with("x\n");
    e.shell_run("sleep 10");
    let request = *e.shell_requests.keys().next().unwrap();
    let handle = e.worker_handles.remove(&request).unwrap();
    handle.cancel(CancelReason::Dismissed);
    let result = next_shell_result(&e); // settle is synchronous — no wait needed
    e.handle_shell_result(result);
    assert!(sh_outputs(&e).is_empty());
    assert!(e.shell_requests.is_empty());
    // the killed job leaves the path clear for the next command
    e.shell_run("echo ok");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert!(e.buf().text().to_string().lines().any(|line| line == "ok"));
}

#[test]
fn head_early_exit_still_replaces() {
    let mut e = editor_with(&"A".repeat(64 * 1024));
    e.pipe_run(0, e.buf().len_bytes(), "head -c 16");
    let result = next_shell_result(&e);
    e.handle_shell_result(result);
    assert_eq!(e.buf().text().to_string(), "A".repeat(16));
    assert_eq!(e.message, "piped");
}

#[test]
fn failed_pipe_then_new_pipe_progresses() {
    let mut e = editor_with("beta\nalpha\n");
    e.pipe_run(0, e.buf().len_bytes(), "false");
    let failed = next_shell_result(&e);
    e.handle_shell_result(failed);
    assert_eq!(e.buf().text().to_string(), "beta\nalpha\n");
    e.pipe_run(0, e.buf().len_bytes(), "sort");
    let sorted = next_shell_result(&e);
    e.handle_shell_result(sorted);
    assert_eq!(e.buf().text().to_string(), "alpha\nbeta\n");
}
