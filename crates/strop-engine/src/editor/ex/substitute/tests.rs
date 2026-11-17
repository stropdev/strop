use super::*;
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

/// The previewed text of `line` (zero-based).
fn previewed(preview: &SubstitutionPreview<'_>, line: usize) -> String {
    preview.line(line).expect("a changed line").text.clone()
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
    assert_eq!(previewed(&preview, 0), "omega one");
    assert_eq!(previewed(&preview, 1), "omega two omega");
    assert_eq!(
        preview.line(1).unwrap().added,
        [(0, 5), (10, 15)],
        "the new text is marked"
    );
    assert_eq!(editor.preview().unwrap().unwrap().0.len(), 3);

    // one Esc cancels (0110 §3.1)
    editor.feed(Key::Esc);
    assert!(!editor.cmdline.is_open());
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
    assert_eq!(editor.message(), "3 substitutions on 3 lines");
}

#[test]
fn the_replacement_is_vims_and_the_preview_shows_the_same_text() {
    let mut editor = Editor::new(Buffer::from_text("a/b key=value\n"));
    // an escaped delimiter and an escaped backslash are themselves
    editor.feed_text(r":s/a\/b/x\/y\\z/");
    editor.feed(Key::Enter);
    assert_eq!(editor.buf().text().to_string(), "x/y\\z key=value\n");
    // groups, `&`, case and a line break, seen in the card before Enter
    editor.feed_text(r":s/\(key\)=\(value\)/\2=\U\1\E [&]\rnext/");
    let preview = ready(&editor);
    // the added line break is drawn on the same row
    assert_eq!(previewed(&preview, 0), "x/y\\z value=KEY [key=value]↵next");
    editor.feed(Key::Enter);
    assert_eq!(
        editor.buf().text().to_string(),
        "x/y\\z value=KEY [key=value]\nnext\n"
    );
    // `\&` is an ampersand, `\n` a NUL, `\t` a tab
    editor.feed_text(":2s/next/\\&\\n\\t/\r");
    assert_eq!(
        editor.buf().text().to_string(),
        "x/y\\z value=KEY [key=value]\n&\0\t\n"
    );
}

#[test]
fn unicode_crlf_and_deletion_preview_keep_exact_apply_semantics() {
    let mut editor = Editor::new(Buffer::from_text("α α\r\nβα\r\n"));
    editor.feed_text(":%s/α//g");
    let preview = ready(&editor);
    assert_eq!(preview.count(), 3);
    assert_eq!(previewed(&preview, 0), " ");
    assert_eq!(previewed(&preview, 1), "β");
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
    assert!(editor.message().contains("trailing characters: x"));
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
    assert_eq!(preview.offscreen(), (1, 3));
    for line in 0..6 {
        assert_eq!(previewed(&preview, line), "omega");
    }
    assert_eq!(preview.summary(), "6 changes · 1 above · 3 below");
}

#[test]
fn whole_buffer_offsets_are_invalid_instead_of_substituting_everything() {
    let original = "alpha\n";
    let mut editor = Editor::new(Buffer::from_text(original));
    editor.feed_text(":%+1s/alpha/omega/g\r");
    assert_eq!(editor.buf().text(), original);
    assert!(editor.message.contains("not an editor command"));
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
fn one_escape_cancels_ex_and_search_lines_with_an_outer_operator() {
    for (prompt, outer_operator) in [(":%s/alpha/omega/g", false), ("/alpha", true)] {
        let mut editor = Editor::new(Buffer::from_text("alpha\n"));
        if outer_operator {
            editor.feed_text("d");
        }
        editor.feed_text(prompt);
        editor.feed(Key::Esc);
        assert!(!editor.cmdline.is_open());
        assert!(
            editor.walker.is_ground(),
            "{prompt}: the operator is gone too"
        );
        assert_eq!(editor.buf().text().to_string(), "alpha\n");
    }
}

/// The doctrine as an assertion (0001: the preview cannot lie): for
/// many buffers and `:s` lines, every line the preview shows reads
/// exactly as that line does after Enter.
#[test]
fn the_previewed_text_of_every_changed_line_is_the_line_after_enter() {
    const WORDS: [&str; 6] = ["ab", "b", "abc", "  ", "ba", "\t"];
    const PATTERNS: [&str; 8] = ["a", "b", "ab", "a.", "^", "$", "b*", "\\w\\+"];
    const REPLACEMENTS: [&str; 6] = ["x", "", "&&", "<&>", "\\u&", "yy"];
    let mut seed: u64 = 0x5eed;
    let mut next = |limit: usize| {
        seed = seed
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (seed >> 33) as usize % limit
    };
    for case in 0..300 {
        let lines = 1 + next(5);
        let text: String = (0..lines)
            .map(|_| {
                let words = next(4);
                let line: String = (0..words).map(|_| WORDS[next(WORDS.len())]).collect();
                line + "\n"
            })
            .collect();
        let pattern = PATTERNS[next(PATTERNS.len())];
        let replacement = REPLACEMENTS[next(REPLACEMENTS.len())];
        let flags = if next(2) == 0 { "g" } else { "" };
        let range = if next(2) == 0 { "%" } else { "" };
        let command = format!(":{range}s/{pattern}/{replacement}/{flags}");
        let mut editor = Editor::new(Buffer::from_text(&text));
        editor.feed_text(&command);
        let expected: Vec<(usize, String)> = match editor.substitute_preview() {
            Some(SubstitutePreviewState::Ready(preview)) => (0..lines)
                .filter_map(|line| preview.line(line).map(|shown| (line, shown.text.clone())))
                .collect(),
            _ => Vec::new(),
        };
        editor.feed(Key::Enter);
        for (line, shown) in expected {
            assert_eq!(
                editor.buf().line_text(line),
                shown,
                "case {case}: {command:?} on {text:?}"
            );
        }
    }
}
