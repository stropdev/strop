use super::session::{Selection, WorkerLifecycle};
use crate::editor::{events, Editor, Mode};
use strop_core::frontend_input::Key;
use strop_core::Buffer;
mod language;
mod preview;
mod snippet;
mod sources;

fn fixture(text: &str, caret: usize) -> (Editor, events::EventReceiver) {
    let mut editor = Editor::new_in(Buffer::from_text(text), "/completion-fixture".into());
    let (tx, rx) = events::channel();
    // Install the real event wake without starting unrelated filesystem/git/LSP
    // services. The native completion worker and production reducer still run.
    editor.app_tx = Some(tx);
    editor.set_head(caret);
    editor.feed(Key::Char('i'));
    (editor, rx)
}
fn until(editor: &mut Editor, events: &events::EventReceiver, ready: impl Fn(&Editor) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !ready(editor) {
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        assert!(!remaining.is_zero(), "completion worker stalled");
        let wait = if editor.completion_retiring() {
            remaining.min(events::QUIESCENCE_POLL)
        } else {
            remaining
        };
        match events.recv_timeout(wait) {
            Ok(event) => editor.handle_app_event(event),
            Err(events::RecvTimeoutError::Timeout) if editor.completion_retiring() => {
                editor.handle_app_event(events::AppEvent::Completion)
            }
            Err(error) => panic!("completion worker stalled: {error}"),
        }
    }
}
fn words_ready(editor: &Editor) -> bool {
    editor
        .completion
        .session
        .as_ref()
        .is_some_and(|session| session.words().is_some())
}
fn selected(editor: &Editor) -> bool {
    editor
        .completion
        .session
        .as_ref()
        .is_some_and(|session| session.selected().is_some())
}

#[test]
fn explicit_word_acceptance_changes_text_once_and_one_escape_commits_one_undo_group() {
    let original = "result\n\n";
    let (mut editor, events) = fixture(original, 7);
    editor.feed_text("re");
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    editor.feed(Key::CtrlL);
    assert!(
        selected(&editor),
        "a repaint does not revoke a completion choice"
    );
    assert_eq!(editor.buf().text().to_string(), "result\nre\n");
    editor.feed(Key::CtrlY);
    until(&mut editor, &events, |editor| {
        editor.completion.session.is_none()
    });
    assert_eq!(editor.buf().text().to_string(), "result\nresult\n");
    editor.feed(Key::Esc);
    assert_eq!(editor.mode, Mode::Normal);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), original);
}

#[test]
fn automatic_results_do_not_steal_newline_or_indent_without_selection() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    assert!(matches!(
        editor.completion.session.as_ref().unwrap().selection,
        Selection::None
    ));
    editor.feed(Key::Enter);
    assert_eq!(editor.buf().text().to_string(), "result\nre\n\n");
    let indentation = editor.cur_indent().unit();
    editor.feed(Key::Tab);
    assert_eq!(
        editor.buf().text().to_string(),
        format!("result\nre\n{indentation}\n")
    );
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn manual_only_stays_idle_until_requested_then_keeps_up_with_typing() {
    let (mut editor, events) = fixture("result\n\n", 7);
    let mut config = editor.config.clone();
    config.completion.auto_popup = false;
    editor.set_config(config);
    editor.feed_text("re");
    assert!(editor.completion.session.is_none());
    assert!(matches!(editor.completion.worker, WorkerLifecycle::Idle));
    editor.feed(Key::CtrlSpace);
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Char('s'));
    until(&mut editor, &events, words_ready);
    let session = editor.completion.session.as_ref().unwrap();
    assert!(session.query.context.invocation.manual());
    assert_eq!(session.words().unwrap().items[0].text.as_ref(), "result");
    assert_eq!(
        session.query.context.primary().unwrap().revision,
        editor.buf().revision()
    );
    editor.feed(Key::CtrlE);
    assert_eq!(editor.mode, Mode::Insert);
    editor.feed(Key::Char('u'));
    assert!(editor.completion.session.is_none());
    assert_eq!(editor.buf().text().to_string(), "result\nresu\n");
}

#[test]
fn disable_revokes_queued_acceptance_and_reenable_does_not_restore_it() {
    let (mut editor, events) = fixture("result\nre\n", 9);
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    editor.feed(Key::CtrlY);
    let mut config = editor.config.clone();
    config.completion.enabled = false;
    editor.set_config(config.clone());
    until(&mut editor, &events, |editor| !editor.completion.pending());
    assert_eq!(editor.buf().text().to_string(), "result\nre\n");
    assert!(editor.completion.session.is_none());
    assert!(matches!(editor.completion.worker, WorkerLifecycle::Idle));
    config.completion.enabled = true;
    editor.set_config(config);
    assert!(editor.completion.session.is_none());
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    editor.feed(Key::CtrlY);
    until(&mut editor, &events, |editor| {
        editor.completion.session.is_none()
    });
    assert_eq!(editor.buf().text().to_string(), "result\nresult\n");
}

#[test]
fn completion_repeat_records_only_the_new_inserted_part_like_vim() {
    for (initial, caret, typed, expected_last_line) in [
        ("result\n\n\n", 7, "re", "result"),
        ("result\nre\n\n", 9, "", "sult"),
    ] {
        let (mut editor, events) = fixture(initial, caret);
        editor.feed_text(typed);
        editor.feed(Key::CtrlN);
        until(&mut editor, &events, selected);
        editor.feed(Key::CtrlY);
        until(&mut editor, &events, |editor| {
            editor.completion.session.is_none()
        });
        editor.feed_text("<esc>j0.");
        assert_eq!(
            editor.buf().text().to_string(),
            format!("result\nresult\n{expected_last_line}\n")
        );
    }
}

#[test]
fn compatible_multicursor_words_apply_to_every_caret() {
    let (mut editor, events) = fixture("result\nre re", 9);
    editor.sels_mut().set_extras([12]);
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    editor.feed(Key::CtrlY);
    until(&mut editor, &events, |editor| {
        editor.completion.session.is_none()
    });
    assert_eq!(editor.buf().text().to_string(), "result\nresult result");
    assert_eq!(editor.head(), 13);
    assert_eq!(editor.extra_selections()[0].head, 20);
    editor.feed_text("<esc>u");
    assert_eq!(editor.buf().text().to_string(), "result\nre re");
}

#[test]
fn escape_revokes_a_cold_query_without_a_second_escape_or_text_change() {
    let text = format!("{}\nre", "result reference repeat ".repeat(50_000));
    let (mut editor, events) = fixture(&text, text.len());
    editor.feed(Key::CtrlN);
    editor.feed(Key::Esc);
    assert_eq!(editor.mode, Mode::Normal);
    assert!(editor.completion.session.is_none());
    editor.stop_completion();
    until(&mut editor, &events, |editor| !editor.completion.pending());
    assert_eq!(editor.buf().text().to_string(), text);
    assert!(matches!(editor.completion.worker, WorkerLifecycle::Idle));
}

#[test]
fn failed_worker_start_does_not_retry_on_every_keystroke() {
    let (mut editor, _events) = fixture("result\n\n", 7);
    let starts = std::rc::Rc::new(std::cell::Cell::new(0));
    let observed = starts.clone();
    editor.tape = std::rc::Rc::new(strop_trace::replay::Tape::fixture(move |operation, _| {
        assert_eq!(operation, "completion.start");
        observed.set(observed.get() + 1);
        Ok(serde_json::json!({"Err": "completion thread unavailable"}))
    }));
    editor.feed_text("result");
    assert_eq!(editor.buf().text().to_string(), "result\nresult\n");
    assert_eq!(starts.get(), 1);
    editor.feed(Key::CtrlN); // A fresh deliberate request may retry.
    assert_eq!(starts.get(), 2);
}

#[test]
fn macro_completion_waits_for_its_query_and_acceptance_without_overtaking_escape() {
    let (mut editor, events) = fixture("result\n\n\n", 7);
    editor.resolution.enabled = true;
    editor.feed_text("<esc>2Gqaire");
    editor.feed(Key::CtrlN);
    until(&mut editor, &events, selected);
    editor.feed(Key::CtrlY);
    until(&mut editor, &events, |editor| {
        editor.completion.session.is_none()
    });
    editor.feed_text("<esc>qj0@a");
    // Deliver generated i,r,e,Ctrl-N, then attempt Ctrl-Y before any provider
    // publication. This schedule is independent of the native worker's speed.
    for _ in 0..5 {
        editor.handle_app_event(events::AppEvent::ResumeInput);
    }
    until(&mut editor, &events, |editor| {
        !editor.resolution.pending()
            && editor.mode == Mode::Normal
            && editor.completion.session.is_none()
    });
    assert_eq!(editor.buf().text().to_string(), "result\nresult\nresult\n");
}
