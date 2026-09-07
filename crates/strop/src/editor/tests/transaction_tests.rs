use strop_core::worker::{Completion, FailureKind, Outcome, Ticket};

use super::*;
use crate::editor::shell::{ProcessOutput, ShellIntent, ShellKey};

/// Register a pipe intent the way `pipe_run` does (minus the spawn)
/// against the given captured original, then deliver `output` as its
/// success result — the real handler against a real registered
/// request.
fn inject_pipe(e: &mut Editor, start: usize, end: usize, original: &str, output: &str) {
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
            original: Some(original.into()),
        },
    );
    e.handle_shell_result(Completion {
        ticket,
        outcome: Outcome::Success(ProcessOutput {
            stdout: output.into(),
            stderr: String::new(),
        }),
    });
}

#[test]
fn ranged_ex_delete_maps_marks_through_the_gateway() {
    let mut editor = Editor::new(Buffer::from_text("remove\nTARGET\n"));
    editor.feed_text("jma:1d<cr>");
    assert_eq!(editor.buf().text().to_string(), "TARGET\n");
    assert_eq!(
        editor.marks[&'a'].1, 0,
        "mark follows its text across ranged Ex deletion"
    );
}

#[test]
fn delayed_pipe_rejects_offsets_inside_new_multibyte_text() {
    let mut editor = Editor::new(Buffer::from_text("ab\n"));
    let origin = editor.current();
    let ticket = {
        let request = editor.worker_ids.allocate().unwrap();
        let ticket = Ticket {
            request,
            key: ShellKey::Pipe {
                document: origin,
                revision: editor.buf().revision(),
                start: 1,
                end: 2,
            },
        };
        editor.shell_requests.insert(
            request,
            ShellIntent {
                ticket: ticket.clone(),
                command: "injected".into(),
                cwd: editor.cwd.clone(),
                original: Some("b".into()),
            },
        );
        ticket
    };
    // the buffer changes shape under the job: é lands inside the
    // captured range and the revision moves
    editor.replace_system(origin, "éx\n").unwrap();
    editor.handle_shell_result(Completion {
        ticket,
        outcome: Outcome::Success(ProcessOutput {
            stdout: "replacement".into(),
            stderr: String::new(),
        }),
    });
    assert_eq!(editor.buf().text().to_string(), "éx\n");
    assert!(
        !editor.message.is_empty(),
        "stale result has visible feedback"
    );
}

#[test]
fn accepted_pipe_maps_anchors_and_is_undoable() {
    let mut editor = Editor::new(Buffer::from_text("prefix TARGET\n"));
    editor.feed_text("wma");
    inject_pipe(&mut editor, 0, 7, "prefix ", "x ");
    assert_eq!(editor.buf().text().to_string(), "x TARGET\n");
    assert_eq!(editor.marks[&'a'].1, 2);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "prefix TARGET\n");
}

#[test]
fn gateway_refusal_leaves_the_document_untouched() {
    // A failed pipe is refused before any mutation: no undo revision,
    // no anchor movement, the exact original bytes.
    let mut editor = Editor::new(Buffer::from_text("prefix TARGET\n"));
    editor.feed_text("wma");
    let ticket = {
        let request = editor.worker_ids.allocate().unwrap();
        let ticket = Ticket {
            request,
            key: ShellKey::Pipe {
                document: editor.current(),
                revision: editor.buf().revision(),
                start: 0,
                end: 7,
            },
        };
        editor.shell_requests.insert(
            request,
            ShellIntent {
                ticket: ticket.clone(),
                command: "injected".into(),
                cwd: editor.cwd.clone(),
                original: Some("prefix ".into()),
            },
        );
        ticket
    };
    let depth_before = editor.buf().history().depth();
    editor.handle_shell_result(Completion {
        ticket,
        outcome: Outcome::Failed {
            failure: strop_core::worker::Failure::new(FailureKind::Exit, "exit status: 2"),
            partial: Some(ProcessOutput::default()),
        },
    });
    assert_eq!(editor.buf().text().to_string(), "prefix TARGET\n");
    assert_eq!(editor.marks[&'a'].1, 7);
    assert_eq!(
        editor.buf().history().depth(),
        depth_before,
        "refused pipe must not create an undo revision"
    );
    editor.feed_text("u");
    assert_eq!(
        editor.buf().text().to_string(),
        "prefix TARGET\n",
        "refused pipe must not consume an undo slot"
    );
}
