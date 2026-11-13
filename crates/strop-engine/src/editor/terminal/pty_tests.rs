use super::*;
use std::time::{Duration, Instant};

use crate::editor::Mode;
use strop_core::frontend_input::{Input, Key, KeyCode, KeyEvent, Modifiers};
use strop_core::Buffer;

fn drain_until(editor: &mut Editor, condition: impl Fn(&Editor) -> bool, what: &str) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        editor.drain_terminals();
        if condition(editor) {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    assert!(
        condition(editor),
        "terminal never reached {what}: {}",
        editor.message
    );
}

fn live_text(editor: &Editor, document: DocumentId) -> String {
    editor
        .terminal_frame(document)
        .map(|frame| frame.projection.to_string())
        .unwrap_or_default()
}

fn ctrl(code: KeyCode) -> Input {
    Input::Key(KeyEvent {
        code,
        modifiers: Modifiers {
            control: true,
            ..Default::default()
        },
        ..KeyEvent::press(code)
    })
}

fn to_normal(editor: &mut Editor) {
    editor.feed_terminal(ctrl(KeyCode::Char('\\')));
    editor.feed_terminal(ctrl(KeyCode::Char('n')));
    assert!(!editor.terminal_input_active());
}

fn line(editor: &Editor) -> usize {
    editor.buf().line_of(editor.head())
}

/// A fixed POSIX shell: a readline shell (bash ≥ 5.1, fish) enables
/// bracketed paste and prints prompts of its own.
fn running_sh(editor: &mut Editor) -> DocumentId {
    editor.feed_text(":terminal exec /bin/sh\r");
    let document = editor.current();
    drain_until(
        editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Running)),
        "shell startup",
    );
    document
}

/// 0090 §4 on a real PTY: NORMAL is live — delayed output lands in the
/// buffer without any refresh — and following is the cursor being on
/// the last line: `G` follows a scrolling stream, `k` stops.
#[test]
fn real_pty_normal_view_is_live_and_follows_from_the_last_line() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.view_rows = 12;
    let document = running_sh(&mut editor);
    editor.feed_terminal(Input::Text(
        "(sleep 0.15; printf 'DELAYED-%s\\n' DONE) &\n".into(),
    ));
    to_normal(&mut editor);
    editor.feed_text("gg");
    assert_eq!(line(&editor), 0);
    drain_until(
        &mut editor,
        |editor| editor.buf().text().to_string().contains("DELAYED-DONE"),
        "delayed output landing in the live buffer",
    );
    assert_eq!(line(&editor), 0, "a cursor off the last line stays put");
    // G: the view follows a stream that scrolls into history. The chord
    // lands mid-stream, as a user's would, so NORMAL starts on the last
    // (bottom) row the child is writing to.
    editor.feed_text("G");
    let last = editor.buf().last_content_line();
    assert_eq!(line(&editor), last);
    editor.feed(Key::Char('i'));
    editor.feed_terminal(Input::Text(
        "for n in $(seq 1 60); do echo $n; sleep 0.01; done; printf 'SEQ-%s\\n' DONE\n".into(),
    ));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("\n20 "),
        "the stream reaching the bottom row",
    );
    to_normal(&mut editor);
    assert_eq!(line(&editor), editor.buf().last_content_line());
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("SEQ-DONE"),
        "the sequence completing",
    );
    assert!(
        editor.buf().last_content_line() > last,
        "the stream grew the buffer"
    );
    assert_eq!(line(&editor), editor.buf().last_content_line());
    // k: stop following; output from a background job grows the buffer
    // below a cursor that keeps its stream position.
    editor.feed(Key::Char('i'));
    editor.feed_terminal(Input::Text(
        "(sleep 0.3; seq 61 90; printf 'SEQ2-%s\\n' DONE) &\n".into(),
    ));
    to_normal(&mut editor);
    editor.feed_text("k");
    let kept = line(&editor);
    let grown = editor.buf().last_content_line();
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("SEQ2-DONE"),
        "the second sequence completing",
    );
    assert!(editor.buf().last_content_line() > grown);
    assert_eq!(line(&editor), kept);
    assert!(kept < editor.buf().last_content_line());
}

/// 0090 §4: pastes go to the child as every terminal sends them — raw
/// bytes while the child has not negotiated bracketed paste, bracketed
/// once it has. No held paste, no consent ticket.
#[test]
fn real_pty_pastes_are_raw_or_bracketed_by_the_child_mode() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let document = running_sh(&mut editor);
    editor.feed_terminal(Input::Paste("printf 'PASTED-%s\\n' RAW\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("PASTED-RAW"),
        "a raw multiline paste executing",
    );
    editor.feed_terminal(Input::Text(
        "stty -echo; printf '\\033[?2004h'; printf 'BRACKET-%s\\n' READY; cat -v\n".into(),
    ));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("BRACKET-READY"),
        "bracketed paste negotiated",
    );
    // The canonical tty hands `cat` whole lines: the newline after the
    // paste releases the bracketed line in one piece.
    editor.feed_terminal(Input::Paste("alpha".into()));
    editor.feed_terminal(Input::Text("\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("^[[200~alpha^[[201~"),
        "the bracketed paste reaching the child",
    );
    assert_eq!(editor.terminals.entries.len(), 1);
}

/// 0090 §4 (0089 B7/T7): a hidden running session makes `:qa` one
/// notice; `:qa!` stops it and quits.
#[test]
fn real_pty_hidden_session_blocks_qa_with_one_notice_until_qa_bang() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let origin = editor.current();
    let terminal = running_sh(&mut editor);
    assert!(editor.close_buffer(false));
    assert_eq!(editor.current(), origin);
    assert!(editor.terminal_phase(terminal).unwrap().live());

    editor.feed_text(":qa\r");
    assert!(editor.picker.is_none());
    assert!(!editor.should_quit());
    assert_eq!(editor.message, "1 terminal running — :qa! stops them");
    assert!(editor.terminal_phase(terminal).unwrap().live());

    editor.feed_text(":qa!\r");
    assert!(editor.should_quit());
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(terminal), Some(Phase::Exited { .. })),
        "the stopped shell",
    );
}

#[test]
fn real_pty_sessions_number_from_one_per_editor_in_term_names() {
    // 0088 U12: unrelated worker requests (opens, git, discovery) must not
    // leak into the user-facing number — the first terminal is #1.
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    for _ in 0..26 {
        editor.worker_ids.allocate().unwrap();
    }
    for number in 1..=2 {
        let terminal = running_sh(&mut editor);
        let name = editor.buf().name.clone().unwrap();
        assert!(name.starts_with("term://"), "{name}");
        assert!(
            name.ends_with(&format!("//{number}:exec /bin/sh")),
            "{name}"
        );
        assert_eq!(editor.doc(terminal).label(directory.path()), name);
        assert!(editor.close_buffer(false));
    }
}

#[test]
fn real_pty_nested_application_keys_stay_child_owned() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    editor.feed_text(":terminal printf 'READY\\n'; stty raw -echo; cat -v\r");
    let document = editor.current();
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Running)),
        "raw child startup",
    );
    std::thread::sleep(Duration::from_millis(100));
    editor.feed_terminal(Input::Text("\u{1b}\u{3}\u{17}\u{1b}[5~\u{2}\u{6}\n".into()));
    drain_until(
        &mut editor,
        |editor| {
            let output = live_text(editor, document);
            ["^[", "^C", "^W", "^[[5~", "^B", "^F"]
                .iter()
                .all(|expected| output.contains(expected))
        },
        "nested application key output",
    );
    to_normal(&mut editor);
    let output = editor.buf().text().to_string();
    for expected in ["^[", "^C", "^W", "^[[5~", "^B", "^F"] {
        assert!(output.contains(expected), "missing {expected:?}: {output}");
    }
    assert_eq!(editor.terminals.entries.len(), 1);
}

/// 0090 §4: `Space t` opens a new session in a horizontal split below,
/// in TERMINAL mode; re-entry through the alternate buffer lands in
/// NORMAL with the view restored.
#[test]
fn real_pty_space_t_splits_below_in_terminal_mode_and_re_entry_is_normal() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let origin = editor.current();
    editor.feed_text(" t");
    assert_eq!(editor.panes.len(), 2);
    assert_eq!(editor.active_pane, 1);
    assert_eq!(editor.layout.outline().to_string(), r#"["col",[1,2]]"#);
    assert_eq!(editor.panes[0].doc, origin);
    let terminal = editor.current();
    assert!(editor.terminal_document(terminal).is_some());
    assert!(editor.terminal_input_active());
    assert!(crate::editor::state_json(&editor).contains("\"mode\":\"TERMINAL\""));
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(terminal), Some(Phase::Running)),
        "the split shell",
    );
    editor.feed_terminal(Input::Text("printf 'SPLIT-%s\\n' OK\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, terminal).contains("SPLIT-OK"),
        "output in the split",
    );
    to_normal(&mut editor);
    editor.feed_text("gg");
    editor.feed(Key::CtrlCaret);
    assert_eq!(editor.current(), origin);
    editor.feed(Key::CtrlCaret);
    assert_eq!(editor.current(), terminal);
    assert!(!editor.terminal_input_active());
    assert_eq!(editor.mode, Mode::Normal);
    assert_eq!(line(&editor), 0);
    editor.feed(Key::Char('A'));
    assert!(editor.terminal_input_active());
}
