//! Hermetic replay regressions (R11/R12): real shell/picker preparation
//! and the production AppEvent handlers, recorded on a fixture tape and
//! replayed end to end. No native command ever runs; a missing recorded
//! observation fails the test instead of consulting the host.
use std::io;
use std::rc::Rc;

use strop_core::worker::{Completion, Outcome};
use strop_trace::replay::Tape;

use crate::editor::events::AppEvent;
use crate::editor::picker::PickerEvent;
use crate::editor::shell::ProcessOutput;
use crate::editor::Editor;

use super::drive::{self, Action};
use super::seed::Seed;

fn fixture() -> Editor {
    let mut editor = Editor::new_in(
        strop_core::Buffer::from_text("source\n"),
        "/recorded".into(),
    );
    editor.tape = Rc::new(Tape::fixture(|_, _| {
        Err(io::Error::other("unexpected native observation"))
    }));
    editor.tape.seed(&Seed::capture(&editor).unwrap()).unwrap();
    editor
}

fn action(editor: &mut Editor, event: AppEvent) {
    let mut tick = editor.tape.now();
    tick.monotonic_ms += 1;
    editor.recorded_action(Action::Event(event), tick).unwrap();
}

fn keys(editor: &mut Editor, text: &str) {
    for key in crate::editor::keys::parse(text) {
        action(editor, AppEvent::Terminal(key));
    }
}

fn complete(editor: &mut Editor, command: &str, text: &str) -> AppEvent {
    let ticket = editor
        .shell_requests
        .values()
        .find(|intent| intent.command == command)
        .unwrap()
        .ticket
        .clone();
    AppEvent::Shell(Completion {
        ticket,
        outcome: Outcome::Success(ProcessOutput {
            stdout: text.into(),
            stderr: String::new(),
        }),
    })
}

fn replay_fixture(editor: &Editor) -> Editor {
    editor.tape.finish().unwrap();
    drive::replay(editor.tape.fixture_nodes()).unwrap()
}

/// Reordered shell completions: the newest focus wins in both modes, the
/// stale display still lands as its own buffer.
#[test]
fn recorded_reordered_shell_outputs_keep_both_but_only_newest_focus() {
    let mut editor = fixture();
    keys(&mut editor, ":!first<cr>");
    keys(&mut editor, ":!second<cr>");
    let first = complete(&mut editor, "first", "A\n");
    let second = complete(&mut editor, "second", "B\n");
    action(&mut editor, second);
    let focused = editor.current();
    action(&mut editor, first);
    assert_eq!(editor.current(), focused);
    assert_eq!(editor.buf().text().to_string(), "B\n");
    let replayed = replay_fixture(&editor);
    assert_eq!(replayed.current(), focused);
    assert_eq!(replayed.buf().text().to_string(), "B\n");
    assert!(replayed
        .docs
        .iter()
        .any(|(_, document)| document.buf.text() == "A\n"));
    assert!(replayed.shell_requests.is_empty());
}

/// A duplicated wire delivery is applied once, and undo after replay
/// restores the original text.
#[test]
fn duplicate_pipe_completion_is_applied_once_and_replay_undo_restores_original() {
    let mut editor = fixture();
    keys(&mut editor, "V |cat<cr>");
    let event = complete(&mut editor, "cat", "replaced\n");
    let wire = serde_json::to_value(&event).unwrap();
    action(&mut editor, event);
    action(&mut editor, serde_json::from_value(wire).unwrap());
    assert_eq!(editor.buf().text().to_string(), "replaced\n");
    keys(&mut editor, "u");
    assert_eq!(editor.buf().text().to_string(), "source\n");
    let replayed = replay_fixture(&editor);
    assert_eq!(replayed.buf().text().to_string(), "source\n");
}

/// A stale picker stream's terminal event cannot end the recorded newer
/// query; the newer stream's items land in both modes.
#[test]
fn stale_picker_terminal_cannot_end_recorded_new_query() {
    use strop_picker::{Item, Payload, PickerMsg};
    let mut editor = fixture();
    keys(&mut editor, " /a");
    let first = editor.picker.as_ref().unwrap().active.clone().unwrap();
    keys(&mut editor, "b");
    let second = editor.picker.as_ref().unwrap().active.clone().unwrap();
    action(
        &mut editor,
        AppEvent::Picker(PickerEvent {
            ticket: first,
            msg: PickerMsg::Finished(Outcome::Success(())),
        }),
    );
    assert!(editor.picker.as_ref().unwrap().picker.streaming);
    action(
        &mut editor,
        AppEvent::Picker(PickerEvent {
            ticket: second.clone(),
            msg: PickerMsg::Items(vec![Item {
                text: "matching row".into(),
                payload: Payload::File("hit".into()),
            }]),
        }),
    );
    action(
        &mut editor,
        AppEvent::Picker(PickerEvent {
            ticket: second,
            msg: PickerMsg::Finished(Outcome::Success(())),
        }),
    );
    let replayed = replay_fixture(&editor);
    let picker = &replayed.picker.as_ref().unwrap().picker;
    assert!(!picker.streaming);
    assert_eq!(picker.items[0].text, "matching row");
}

/// Tampering with a recorded request's owner identity fails replay at the
/// gate — before any native launch could have happened.
#[test]
fn changing_recorded_request_identity_fails_before_native_launch() {
    let mut editor = fixture();
    keys(&mut editor, ":!first<cr>");
    editor.tape.finish().unwrap();
    let mut nodes = editor.tape.fixture_nodes();
    let request = nodes
        .iter_mut()
        .find(|node| {
            matches!(node, strop_trace::replay::Node::Request { operation, .. } if operation == "shell.display")
        })
        .expect("shell producer emits request");
    if let strop_trace::replay::Node::Request { arguments, .. } = request {
        *arguments = serde_json::json!({"wrong_owner": true});
    }
    assert!(drive::replay(nodes).is_err());
}

/// Frames replay through the same draw and check the recorded cells.
#[test]
fn recorded_frames_replay_the_same_cell_observation() {
    let mut editor = fixture();
    let mut tick = editor.tape.now();
    tick.monotonic_ms += 1;
    editor
        .recorded_action(
            Action::Frame {
                columns: 40,
                rows: 6,
            },
            tick,
        )
        .unwrap();
    keys(&mut editor, "Ax<esc>");
    let mut tick = editor.tape.now();
    tick.monotonic_ms += 1;
    editor
        .recorded_action(
            Action::Frame {
                columns: 40,
                rows: 6,
            },
            tick,
        )
        .unwrap();
    let replayed = replay_fixture(&editor);
    assert_eq!(replayed.buf().text().to_string(), "sourcex\n");
}

/// Capture is startup-only: an open picker means services started and a
/// full-replay seed would be a lie.
#[test]
fn seed_refuses_mid_session_capture() {
    let mut editor = fixture();
    keys(&mut editor, ":!first<cr>");
    assert!(
        Seed::capture(&editor).is_ok(),
        "a shell request is not an open picker"
    );
    editor.open_picker(strop_picker::Kind::Files);
    assert!(Seed::capture(&editor).is_err());
}

/// Actions recorded after the deliberate end are refused, so a finished
/// tape can never grow a post-mortem tail.
#[test]
fn recording_after_finish_is_refused() {
    let mut editor = fixture();
    editor.tape.finish().unwrap();
    let mut tick = editor.tape.now();
    tick.monotonic_ms += 1;
    assert!(editor.recorded_action(Action::Finish, tick).is_err());
}
