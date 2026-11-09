use super::*;
use std::time::{Duration, Instant};

use strop_core::frontend_input::Key;
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
        .terminal_frame(document, true)
        .map(|frame| frame.projection.to_string())
        .unwrap_or_default()
}

fn choose_open_action(editor: &mut Editor, label: &str) {
    let (picker, index) = {
        let glue = editor.picker.as_ref().expect("context action picker");
        let index = glue
            .picker
            .items
            .iter()
            .position(|item| item.text.contains(label))
            .unwrap_or_else(|| panic!("missing {label:?} action: {:?}", glue.picker.items));
        (glue.id, index)
    };
    editor.accept_filesystem_action(picker, index);
}

fn inspect(editor: &mut Editor) {
    editor.feed_terminal(strop_core::frontend_input::Input::Key(
        strop_core::frontend_input::KeyEvent::press(strop_core::frontend_input::KeyCode::Function(
            2,
        )),
    ));
    assert!(!editor.terminal_input_active());
}

#[test]
fn real_pty_delayed_output_preserves_selection_yank_until_refresh_or_return() {
    use strop_core::frontend_input::Input;
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    // A fixed POSIX shell: a readline shell (bash ≥ 5.1, fish) enables
    // bracketed paste, which changes whether a multiline paste is held.
    editor.feed_text(":terminal exec /bin/sh\r");
    let document = editor.current();
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Running)),
        "shell startup",
    );
    editor.feed_terminal(Input::Text(
        "(sleep 0.15; printf 'DELAYED-%s\\n' DONE) &\n".into(),
    ));
    inspect(&mut editor);
    editor.feed_text("0v$y");
    let yanked = editor.register(None).text.clone();
    let pinned = editor.buf().text().to_string();
    editor.feed_text("l");
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("DELAYED-DONE"),
        "delayed output under a pinned selection",
    );
    assert_eq!(editor.buf().text().to_string(), pinned);
    assert_eq!(editor.register(None).text, yanked);
    assert!(editor.terminal_has_new_output(document));
    assert!(editor
        .feedback()
        .iter()
        .any(|record| { record.summary().contains("new output") && record.unresolved() }));

    editor.feed_text(" a");
    choose_open_action(&mut editor, "Refresh pinned terminal output");
    assert!(editor.buf().text().to_string().contains("DELAYED-DONE"));
    assert!(!editor.terminal_has_new_output(document));
    assert_eq!(editor.register(None).text, yanked);
    editor.feed(Key::Char('i'));
    assert!(editor.terminal_input_active());
    assert_eq!(editor.terminals.entries.len(), 1);
}

#[test]
fn real_pty_paste_review_decisions_are_redacted_and_ticket_scoped() {
    use strop_core::frontend_input::Input;
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    // A fixed POSIX shell: a readline shell (bash ≥ 5.1, fish) enables
    // bracketed paste, which changes whether a multiline paste is held.
    editor.feed_text(":terminal exec /bin/sh\r");
    let document = editor.current();
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Running)),
        "shell startup",
    );
    let accepted = "printf 'ACCEPTED-%s\\n' PASTE\n";
    editor.feed_terminal(Input::Paste(accepted.into()));
    drain_until(
        &mut editor,
        |editor| editor.terminal_paste_held(document),
        "held paste",
    );
    let session = editor.terminal_document(document).unwrap().session;
    let accepted_ticket = editor.terminals.entries[&session]
        .paste
        .as_ref()
        .unwrap()
        .ticket;
    inspect(&mut editor);
    editor.feed_text("l");
    assert!(editor.terminal_paste_held(document));
    assert!(editor
        .feedback()
        .iter()
        .all(|record| !record.detail().contains("ACCEPTED")));
    editor.feed_text(" a");
    let rows = &editor.picker.as_ref().unwrap().picker.items;
    assert!(rows
        .iter()
        .any(|item| item.text.contains("Review held paste")));
    assert!(rows
        .iter()
        .any(|item| item.text.contains("Send held paste")));
    assert!(rows
        .iter()
        .any(|item| item.text.contains("Discard held paste")));
    assert!(!rows.iter().any(|item| item.text.contains("ACCEPTED")));
    choose_open_action(&mut editor, "Review held paste");
    assert!(editor.message.contains("content redacted"));
    assert!(!editor.message.contains("ACCEPTED"));
    editor.context_actions();
    choose_open_action(&mut editor, "Send held paste");
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("ACCEPTED-PASTE"),
        "accepted paste without an added Enter",
    );
    drain_until(
        &mut editor,
        |editor| !editor.terminal_paste_held(document),
        "accepted paste ticket clearing",
    );

    editor.feed_terminal(Input::Paste("printf 'DISCARDED-%s\\n' PASTE\n".into()));
    drain_until(
        &mut editor,
        |editor| editor.terminal_paste_held(document),
        "second held paste",
    );
    let discarded_ticket = editor.terminals.entries[&session]
        .paste
        .as_ref()
        .unwrap()
        .ticket;
    inspect(&mut editor);
    editor.context_actions();
    choose_open_action(&mut editor, "Discard held paste");
    drain_until(
        &mut editor,
        |editor| !editor.terminal_paste_held(document),
        "discarded paste ticket clearing",
    );
    editor.feed(Key::Char('i'));
    editor.feed_terminal(Input::Text("printf 'AFTER-%s\\n' DISCARD\n".into()));
    drain_until(
        &mut editor,
        |editor| live_text(editor, document).contains("AFTER-DISCARD"),
        "input after discard",
    );
    assert!(!live_text(&editor, document).contains("DISCARDED-PASTE"));

    editor.terminal_paste_decision_for(session, accepted_ticket, Some(true));
    assert!(editor.message.contains("no held paste") || editor.message.contains("stale"));
    editor.feed_terminal(Input::Text("exit 0\n".into()));
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(document), Some(Phase::Exited { .. })),
        "session exit",
    );
    let before = live_text(&editor, document);
    editor.terminal_paste_decision_for(session, discarded_ticket, Some(true));
    assert!(editor.message.contains("stale"));
    assert_eq!(live_text(&editor, document), before);
    assert!(!editor.terminal_input_active());
}

#[test]
fn real_pty_hidden_session_quit_review_focus_stop_and_return_do_not_implicitly_kill() {
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    let origin = editor.current();
    // A fixed POSIX shell: a readline shell (bash ≥ 5.1, fish) enables
    // bracketed paste, which changes whether a multiline paste is held.
    editor.feed_text(":terminal exec /bin/sh\r");
    let terminal = editor.current();
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(terminal), Some(Phase::Running)),
        "shell startup",
    );
    assert!(editor.close_buffer(false));
    assert_eq!(editor.current(), origin);
    assert!(editor.terminal_phase(terminal).unwrap().live());

    editor.feed_text(":qa\r");
    assert!(editor.picker.is_some());
    choose_open_action(&mut editor, "Return without stopping");
    assert!(!editor.should_quit());
    assert!(editor.terminal_phase(terminal).unwrap().live());

    editor.feed_text(":qa\r");
    choose_open_action(&mut editor, "Focus terminal session");
    assert_eq!(editor.current(), terminal);
    assert!(editor.terminal_phase(terminal).unwrap().live());
    assert!(editor.close_buffer(false));

    editor.feed_text(":qa\r");
    choose_open_action(&mut editor, "Stop terminal session");
    drain_until(
        &mut editor,
        |editor| matches!(editor.terminal_phase(terminal), Some(Phase::Exited { .. })),
        "explicit stop",
    );
    assert!(!editor.should_quit());
}

#[test]
fn real_pty_sessions_number_from_one_per_editor() {
    // 0088 U12: unrelated worker requests (opens, git, discovery) must not
    // leak into the user-facing number — the first terminal is #1.
    let directory = tempfile::tempdir().unwrap();
    let mut editor = Editor::new_in(Buffer::from_text("origin"), directory.path().to_owned());
    for _ in 0..26 {
        editor.worker_ids.allocate().unwrap();
    }
    for number in 1..=2 {
        editor.feed_text(":terminal exec /bin/sh\r");
        let terminal = editor.current();
        drain_until(
            &mut editor,
            |editor| matches!(editor.terminal_phase(terminal), Some(Phase::Running)),
            "shell startup",
        );
        let expected = format!("terminal #{number}");
        assert_eq!(editor.buf().name.as_deref(), Some(expected.as_str()));
        let label = editor.doc(terminal).label(directory.path());
        assert!(
            label.starts_with(&format!("{expected} · launch ")),
            "{label}"
        );
        assert!(editor.close_buffer(false));
    }
}

#[test]
fn real_pty_nested_application_keys_stay_child_owned() {
    use strop_core::frontend_input::Input;
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
    inspect(&mut editor);
    let output = live_text(&editor, document);
    for expected in ["^[", "^C", "^W", "^[[5~", "^B", "^F"] {
        assert!(output.contains(expected), "missing {expected:?}: {output}");
    }
    assert_eq!(editor.terminals.entries.len(), 1);
}
