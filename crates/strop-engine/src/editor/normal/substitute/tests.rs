use super::*;
use crate::editor::{PendingFieldEscape, PendingFieldMode};
use std::time::Duration;
use strop_core::frontend_input::Key;

fn ready(editor: &Editor) -> SubstitutionPreview<'_> {
    match editor.substitute_preview() {
        Some(SubstitutePreviewState::Ready(preview)) => preview,
        Some(SubstitutePreviewState::Unavailable(reason))
        | Some(SubstitutePreviewState::Invalid(reason))
        | Some(SubstitutePreviewState::Incomplete(reason)) => {
            panic!("expected a ready substitution preview: {reason}")
        }
        Some(SubstitutePreviewState::Resolving) => {
            panic!("substitution preview is still resolving")
        }
        Some(SubstitutePreviewState::NoTarget) => panic!("substitution preview has no target"),
        None => panic!("substitution preview is absent"),
    }
}

fn settle_resolution(editor: &mut Editor) {
    for _ in 0..64 {
        if !editor.resolution.pending() {
            return;
        }
        let event = editor
            .resolution
            .rx
            .as_ref()
            .expect("resolution events")
            .recv_timeout(Duration::from_secs(5))
            .expect("resolution completes");
        editor.handle_resolution(event);
    }
    panic!("current substitution resolution did not publish")
}

#[test]
fn preview_apply_and_cancel_share_one_plan_without_composition_effects() {
    let original = "alpha one\nalpha two alpha\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    let revision = editor.buf().revision();
    editor.feed_text(":%s/alpha/omega/g");

    assert_eq!(editor.buf().text().to_string(), original);
    assert_eq!(editor.buf().revision(), revision);
    assert!(!editor.buf().is_dirty());
    assert!(editor.last_search.is_none());
    assert!(editor.register(None).text.is_empty());
    let preview = ready(&editor);
    assert_eq!(preview.count(), 3);
    assert_eq!(preview.changed_line_count(), 2);
    assert_eq!(preview.scope(), (0, 1));
    assert!(preview.diff_text().contains("-alpha one"));
    assert!(preview.diff_text().contains("+omega one"));
    assert!(preview.diff_text().contains("-alpha two alpha"));
    assert!(preview.diff_text().contains("+omega two omega"));
    assert_eq!(editor.preview().unwrap().unwrap().0.len(), 3);

    editor.feed(Key::Esc);
    let field = editor.pending_field_state().unwrap();
    assert_eq!(field.mode, PendingFieldMode::Normal);
    assert_eq!(field.next_escape, PendingFieldEscape::CancelPrompt);
    editor.feed(Key::Esc);
    assert!(!editor.pending.is_active());
    assert_eq!(editor.buf().text().to_string(), original);
    assert_eq!(editor.buf().revision(), revision);
    assert!(editor.last_search.is_none());
    assert!(editor.register(None).text.is_empty());

    editor.feed_text(":%s/alpha/omega/g\r");
    assert_eq!(
        editor.buf().text().to_string(),
        "omega one\nomega two omega\n"
    );
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), original);
}

#[test]
fn query_metacharacters_and_non_global_line_semantics_are_not_literal() {
    let mut editor = Editor::new(Buffer::from_text("a.c abc ac\naa\naa\n"));
    editor.feed_text(":%s/a.c/X/g\r");
    assert_eq!(editor.buf().text().to_string(), "X X ac\naa\naa\n");
    editor.feed_text(":%s/a/b/\r");
    assert_eq!(editor.buf().text().to_string(), "X X bc\nba\nba\n");
    assert_eq!(editor.message(), "3 substitutions");
}

#[test]
fn delimiters_and_replacement_escaping_have_explicit_literal_rules() {
    let mut editor = Editor::new(Buffer::from_text("a/b a.b a.b\n"));
    editor.feed_text(r":s/a\/b/x\/y\\z\1&$1/");
    editor.feed(Key::Enter);
    assert_eq!(editor.buf().text().to_string(), "x/y\\z\\1&$1 a.b a.b\n");
    editor.feed_text(":%s#a\\.b#&#g\r");
    assert_eq!(editor.buf().text().to_string(), "x/y\\z\\1&$1 & &\n");
}

#[test]
fn unicode_crlf_and_deletion_preview_keep_exact_apply_semantics() {
    let mut editor = Editor::new(Buffer::from_text("α α\r\nβα\r\n"));
    editor.feed_text(":%s/α//g");
    let preview = ready(&editor);
    assert_eq!(preview.count(), 3);
    assert!(preview.diff_text().contains("-α α"));
    assert!(preview.diff_text().contains("+ "));
    assert!(preview.diff_text().contains("-βα"));
    assert!(preview.diff_text().contains("+β"));
    assert_eq!(editor.buf().text().to_string(), "α α\r\nβα\r\n");
    editor.feed(Key::Enter);
    assert_eq!(editor.buf().text().to_string(), " \r\nβ\r\n");
}

#[test]
fn incomplete_invalid_and_stale_input_never_reuses_valid_output() {
    let original = "alpha alpha\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text(":%s/alpha/omega/g");
    assert_eq!(ready(&editor).count(), 2);
    editor.feed_text("x");
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Invalid(_))
    ));
    assert!(editor.preview().is_err());
    assert_eq!(editor.buf().text().to_string(), original);
    editor.feed(Key::Enter);
    assert!(editor.message().contains("unsupported substitute flags"));
    assert_eq!(editor.buf().text().to_string(), original);

    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text(":%s/alpha");
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Incomplete(_))
    ));
    assert!(editor.preview().unwrap().is_none());
    editor.feed_text("/[/omega/");
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Invalid(_))
    ));
    assert_eq!(editor.buf().text().to_string(), original);

    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text(":%s/alpha/omega/g");
    assert_eq!(ready(&editor).count(), 2);
    editor.buf_mut().edit().insert(0, "external").unwrap();
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Unavailable(_))
    ));
    editor.feed(Key::Enter);
    assert!(!editor.buf().text().to_string().contains("omega"));
}

#[test]
fn cursor_identity_no_target_and_offscreen_counts_are_truthful() {
    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.sels_mut().plant_extra(2);
    editor.feed_text(":%s/alpha/omega/g");
    assert_eq!(ready(&editor).count(), 1);
    editor.sels_mut().set_extras([4]);
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Unavailable(_))
    ));

    let mut editor = Editor::new(Buffer::from_text("alpha\n"));
    editor.feed_text(":%s/missing/omega/g");
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::NoTarget)
    ));
    assert!(editor.preview().unwrap().is_none());

    let mut editor = Editor::new(Buffer::from_text(&"alpha\n".repeat(6)));
    editor.view_rows = 2;
    editor.view_mut().view_top = 1;
    editor.feed_text(":%s/alpha/omega/g");
    let preview = ready(&editor);
    assert_eq!(preview.count(), 6);
    assert_eq!(preview.offscreen_count(), 4);
    assert!(preview.diff_text().contains("@@"));
    assert_eq!(preview.diff_text().matches("-alpha").count(), 6);
    assert_eq!(preview.diff_text().matches("+omega").count(), 6);
    assert!(preview.summary().contains("4 offscreen"));
}

#[test]
fn whole_buffer_offsets_are_invalid_instead_of_substituting_everything() {
    let original = "alpha\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text(":%+1s/alpha/omega/g\r");
    assert_eq!(editor.buf().text(), original);
    assert!(editor.message.contains("invalid Ex range after %"));
}

#[test]
fn large_preview_resolves_without_mutating_and_enter_waits_for_the_same_work() {
    let original = format!("{}alpha\n", "x".repeat(5000));
    let mut editor = Editor::new(Buffer::from_text(&original));
    editor.feed_text(":%s/alpha/omega/g");
    assert!(matches!(
        editor.substitute_preview(),
        Some(SubstitutePreviewState::Resolving)
    ));
    assert_eq!(editor.buf().text().to_string(), original);
    settle_resolution(&mut editor);
    assert_eq!(ready(&editor).count(), 1);
    assert_eq!(editor.buf().text().to_string(), original);
    editor.feed(Key::Enter);
    assert_eq!(
        editor.buf().text().to_string(),
        format!("{}omega\n", "x".repeat(5000))
    );

    let mut editor = Editor::new(Buffer::from_text(&original));
    editor.feed_text(":%s/alpha/omega/g\r");
    assert!(editor.resolution.blocked());
    assert_eq!(editor.buf().text().to_string(), original);
    settle_resolution(&mut editor);
    assert_eq!(
        editor.buf().text().to_string(),
        format!("{}omega\n", "x".repeat(5000))
    );
}

#[test]
fn outer_operator_field_mode_and_next_escape_are_exposed_for_ex_and_search() {
    for (prompt, outer_operator) in [
        (":%s/alpha/omega/g", None),
        ("/alpha", Some(strop_grammar::Op::Delete)),
    ] {
        let mut editor = Editor::new(Buffer::from_text("alpha\n"));
        if outer_operator.is_some() {
            editor.feed_text("d");
        }
        editor.feed_text(prompt);
        let field = editor.pending_field_state().unwrap();
        assert_eq!(field.outer_operator, outer_operator);
        assert_eq!(field.mode, PendingFieldMode::Insert);
        assert_eq!(field.next_escape, PendingFieldEscape::EnterNormal);
        editor.feed(Key::Esc);
        let field = editor.pending_field_state().unwrap();
        assert_eq!(field.mode, PendingFieldMode::Normal);
        assert_eq!(field.next_escape, PendingFieldEscape::CancelPrompt);
        editor.feed(Key::Esc);
        assert!(!editor.pending.is_active());
        assert_eq!(editor.buf().text().to_string(), "alpha\n");
    }
}
