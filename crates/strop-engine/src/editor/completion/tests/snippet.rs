use super::language::{editing, prepare};
use super::*;
use crate::editor::completion::{acceptance, context};
use strop_lsp::PositionEncoding;

/// 0069 D05: a snippet item expands through the real plan, the first
/// placeholder is selected, typing replaces it, Tab cycles the stops,
/// $0 ends the session, and one undo retires the whole expansion.
#[test]
fn a_snippet_expands_and_cycles_its_tabstops() {
    let (_directory, mut editor, _source, _events) = editing();
    let delivery = prepare(
        &mut editor,
        serde_json::json!({
            "label": "fn",
            "insertTextFormat": 2,
            "insertText": "fn ${1:name}(${2:arg}) {\n    ${0}\n}",
        }),
    );
    editor.completion_apply_prepared(&delivery);
    assert!(
        editor.snippet_active(),
        "a snippet acceptance enters the tabstop session"
    );
    let text = editor.buf().text().to_string();
    assert!(text.contains("fn name(arg) {"), "expanded text: {text:?}");
    let crate::editor::snippet::SnippetRanges(primary, extras) = editor.snippet_selection_ranges();
    assert_eq!(extras, Vec::new(), "no linked stops in this snippet");
    let Some((start, end)) = primary else {
        panic!("the first placeholder is selected")
    };
    assert_eq!(
        &text[start..end],
        "name",
        "the placeholder selects its default text"
    );

    // typing replaces the placeholder, once — never re-deletes
    editor.feed_text("start");
    let text = editor.buf().text().to_string();
    assert!(
        text.contains("fn start(arg) {"),
        "replaced placeholder: {text:?}"
    );
    assert!(editor.snippet_active(), "typing stays inside the session");

    // Tab to the second stop, then Tab to $0 — the session ends there
    editor.feed(Key::Tab);
    let crate::editor::snippet::SnippetRanges(primary, _) = editor.snippet_selection_ranges();
    let text = editor.buf().text().to_string();
    let Some((start, end)) = primary else {
        panic!("the second placeholder is selected")
    };
    assert_eq!(&text[start..end], "arg");
    editor.feed(Key::Tab);
    assert!(!editor.snippet_active(), "reaching $0 ends the session");
    {
        let head = editor.head();
        let text = editor.buf().text().to_string();
        assert!(
            text[..head].ends_with("    ") && text[head..].starts_with('\n'),
            "the final stop lands at the snippet's $0 (after the indent): caret {head} in {text:?}"
        );
    }

    // the whole expansion is ONE undo unit (the acceptance recording)
    editor.feed(strop_core::frontend_input::Key::Esc);
    assert_eq!(editor.mode(), crate::editor::Mode::Normal);
    editor.feed_text("u");
    assert_eq!(
        editor.buf().text().to_string(),
        "// import slot\nres\n",
        "one undo retires the expansion"
    );
}

/// Linked tabstops share one selection set: replacing the primary mirrors
/// the edit onto the linked stop through the same cursor machinery.
#[test]
fn linked_tabstops_mirror_the_replacement() {
    let (_directory, mut editor, _source, _events) = editing();
    let delivery = prepare(
        &mut editor,
        serde_json::json!({
            "label": "pair",
            "insertTextFormat": 2,
            "insertText": "${1:thing} = ${1:thing} + $2;",
        }),
    );
    editor.completion_apply_prepared(&delivery);
    let crate::editor::snippet::SnippetRanges(primary, extras) = editor.snippet_selection_ranges();
    let Some((start, end)) = primary else {
        panic!("the first linked stop is selected")
    };
    assert_eq!(extras.len(), 1, "the linked stop rides as an extra");
    editor.feed_text("count");
    let text = editor.buf().text().to_string();
    assert!(
        text.contains("count = count + ;"),
        "both linked stops replaced: {text:?}"
    );
    let _ = (start, end);
}

/// A malformed snippet refuses honestly at expansion — the named refusal,
/// no partial text inserted.
#[test]
fn a_malformed_snippet_refuses_without_inserting() {
    let (_directory, editor, _source, _events) = editing();
    let captured = std::sync::Arc::new(
        context::capture(
            &editor,
            context::Invocation::ManualLanguage,
            editor.completion.settings_generation,
            None,
        )
        .unwrap(),
    );
    let primary = captured.primary().unwrap();
    let item: std::sync::Arc<strop_lsp::completion::CompletionItem> = std::sync::Arc::new(
        serde_json::from_value(serde_json::json!({
            "label": "broken",
            "insertTextFormat": 2,
            "insertText": "fn ${1:name",
        }))
        .unwrap(),
    );
    let refusal = acceptance::language(
        &captured,
        editor.doc(primary.source).buf.text(),
        PositionEncoding::Utf8,
        &item,
        None,
        false,
        0,
    );
    assert!(
        matches!(refusal, Err(acceptance::AcceptanceRefusal::Snippet)),
        "a malformed snippet names its refusal: {refusal:?}"
    );
}
