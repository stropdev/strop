//! Tab live-preview cycling (0059 §6.1): focus, cycle/wrap, revert, accept
//! and type-over invariants over the real worker/event loop.
use super::{fixture, selected, until, words_ready};
use crate::editor::{Editor, Mode};
use strop_core::frontend_input::Key;

fn preview_active(editor: &Editor) -> bool {
    editor
        .completion
        .session
        .as_ref()
        .is_some_and(|session| session.preview.is_some())
}

fn word_candidates(editor: &Editor) -> Vec<String> {
    editor
        .completion
        .session
        .as_ref()
        .and_then(|session| session.words())
        .expect("word candidates settled")
        .items
        .iter()
        .map(|item| item.text.to_string())
        .collect()
}

#[test]
fn tab_focuses_the_popup_and_previews_the_first_candidate() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    assert!(!selected(&editor), "menu opens without a selection");
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    assert!(selected(&editor), "Tab focuses the popup");
    assert_eq!(editor.buf().text().to_string(), "result\nresult\n");
    assert_eq!(editor.mode, Mode::Insert);
    assert!(
        editor.completion.session.is_some(),
        "the session stays open while previewing"
    );
}

#[test]
fn tab_cycles_each_candidate_with_live_preview_and_wraps() {
    let (mut editor, events) = fixture("result retry\n\n", 13);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    let candidates = word_candidates(&editor);
    assert!(candidates.len() >= 2, "cycle needs two candidates");
    for cycle in 0..=candidates.len() {
        editor.feed(Key::Tab);
        until(&mut editor, &events, preview_active);
        let expected = &candidates[cycle % candidates.len()];
        assert_eq!(
            editor.buf().text().to_string(),
            format!("result retry\n{expected}\n"),
            "cycle {cycle} previews {expected}"
        );
    }
}

#[test]
fn shift_tab_cycles_backward_and_wraps_to_the_last_candidate() {
    let (mut editor, events) = fixture("result retry\n\n", 13);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    let candidates = word_candidates(&editor);
    assert!(candidates.len() >= 2, "cycle needs two candidates");
    let last = candidates.last().unwrap();
    editor.feed(Key::Backtab);
    until(&mut editor, &events, preview_active);
    assert_eq!(
        editor.buf().text().to_string(),
        format!("result retry\n{last}\n"),
        "Shift-Tab with no selection previews the last candidate"
    );
    editor.feed(Key::Backtab);
    let expected_first = format!("result retry\n{}\n", candidates[0]);
    until(&mut editor, &events, |editor| {
        *editor.buf().text() == *expected_first && preview_active(editor)
    });
}

#[test]
fn dismiss_restores_the_exact_pre_preview_text_and_caret() {
    let (mut editor, events) = fixture("result retry\n\n", 13);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    editor.feed(Key::Tab);
    until(&mut editor, &events, |editor| {
        preview_active(editor) && *editor.buf().text() != *"result retry\nre\n"
    });
    editor.feed(Key::CtrlE);
    assert!(editor.completion.session.is_none());
    assert_eq!(editor.buf().text().to_string(), "result retry\nre\n");
    assert_eq!(editor.head(), 15, "the typed-prefix caret is restored");
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn escape_reverts_the_preview_and_leaves_insert_in_one_event() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    assert_eq!(editor.buf().text().to_string(), "result\nresult\n");
    editor.feed(Key::Esc);
    assert_eq!(editor.buf().text().to_string(), "result\nre\n");
    assert_eq!(editor.mode, Mode::Normal);
    assert!(editor.completion.session.is_none());
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "result\n\n");
}

#[test]
fn cycle_then_accept_matches_direct_acceptance_in_text_and_undo() {
    let original = "result retry\n\n";
    let mut outcomes = Vec::new();
    for preview in [true, false] {
        let (mut editor, events) = fixture(original, 13);
        editor.feed_text("re");
        until(&mut editor, &events, words_ready);
        let candidates = word_candidates(&editor);
        assert!(candidates.len() >= 2, "parity needs two candidates");
        if preview {
            editor.feed(Key::Tab);
            until(&mut editor, &events, preview_active);
            editor.feed(Key::Tab);
            let expected_second = format!("result retry\n{}\n", candidates[1]);
            until(&mut editor, &events, |editor| {
                preview_active(editor) && *editor.buf().text() == *expected_second
            });
        } else {
            editor.feed(Key::CtrlN);
            until(&mut editor, &events, selected);
            editor.feed(Key::CtrlN);
            until(&mut editor, &events, |editor| {
                editor
                    .completion
                    .session
                    .as_ref()
                    .and_then(|session| session.selected())
                    .is_some_and(|(index, _)| index == 1)
            });
        }
        editor.feed(Key::CtrlY);
        until(&mut editor, &events, |editor| {
            editor.completion.session.is_none()
        });
        let accepted = editor.buf().text().to_string();
        assert_eq!(accepted, format!("result retry\n{}\n", candidates[1]));
        editor.feed(Key::Esc);
        editor.feed_text("u");
        let undone = editor.buf().text().to_string();
        outcomes.push((accepted, undone));
    }
    assert_eq!(outcomes[0], outcomes[1], "preview and direct paths agree");
    assert_eq!(
        outcomes[0].1, original,
        "one undo retires the whole session"
    );
}

#[test]
fn typing_after_a_preview_reverts_then_continues_the_session() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    editor.feed(Key::Char('u'));
    until(&mut editor, &events, words_ready);
    assert_eq!(editor.buf().text().to_string(), "result\nreu\n");
    assert!(
        editor.completion.session.is_some(),
        "typing keeps refining the same session"
    );
}

#[test]
fn preview_accept_dot_repeat_matches_direct_accept() {
    let mut outcomes = Vec::new();
    for preview in [true, false] {
        let (mut editor, events) = fixture("result\n\n\n", 7);
        editor.feed_text("re");
        if preview {
            until(&mut editor, &events, words_ready);
            editor.feed(Key::Tab);
            until(&mut editor, &events, preview_active);
        } else {
            editor.feed(Key::CtrlN);
            until(&mut editor, &events, selected);
        }
        editor.feed(Key::CtrlY);
        until(&mut editor, &events, |editor| {
            editor.completion.session.is_none()
        });
        editor.feed_text("<esc>j0.");
        outcomes.push(editor.buf().text().to_string());
    }
    assert_eq!(outcomes[0], outcomes[1]);
    assert_eq!(outcomes[0], "result\nresult\nresult\n");
}

#[test]
fn session_close_from_an_unrelated_key_reverts_the_preview() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    editor.feed(Key::Right);
    assert_eq!(editor.buf().text().to_string(), "result\nre\n");
    assert!(editor.completion.session.is_none());
    assert_eq!(editor.mode, Mode::Insert);
}

#[test]
fn preview_revert_does_not_create_user_undo_steps() {
    let (mut editor, events) = fixture("result\n\n", 7);
    editor.feed_text("re");
    until(&mut editor, &events, words_ready);
    editor.feed(Key::Tab);
    until(&mut editor, &events, preview_active);
    editor.feed(Key::CtrlE);
    editor.feed(Key::Esc);
    editor.feed_text("u");
    assert_eq!(
        editor.buf().text().to_string(),
        "result\n\n",
        "one undo retires the typed prefix; the preview left no undo step"
    );
    editor.feed_text("u");
    assert_eq!(editor.message.as_str(), "already at oldest change");
}
