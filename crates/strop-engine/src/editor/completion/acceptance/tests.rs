use super::*;
use crate::editor::completion::context::{capture, Invocation};
use crate::editor::Editor;
use serde_json::json;
use strop_core::frontend_input::Key;
use strop_core::Buffer;

fn editing(text: &str, caret: usize) -> Editor {
    let mut editor = Editor::new_in(Buffer::from_text(text), "/completion-fixture".into());
    editor.feed_text("i");
    editor.set_head(caret);
    editor
}
fn context(editor: &Editor) -> CompletionContext {
    capture(editor, Invocation::ManualCombined, 0, None).unwrap()
}
fn item(value: serde_json::Value) -> CompletionItem {
    serde_json::from_value(value).unwrap()
}
fn apply(editor: &mut Editor, plan: CompletionAcceptancePlan) {
    editor
        .apply(
            plan.target.document(),
            plan.target.revision(),
            crate::editor::transact::ChangeSet {
                edits: plan.edits,
                undo_open: true,
            },
        )
        .unwrap();
    editor.sels_mut().collapse_primary(plan.caret_bytes[0]);
    editor
        .sels_mut()
        .set_extras(plan.caret_bytes.into_iter().skip(1));
}

#[test]
fn a_word_replaces_its_whole_suffix_and_joins_the_insert_undo_group() {
    let mut editor = editing("result resSuffix", 10);
    let plan = word(&context(&editor), "result", 0).unwrap();
    apply(&mut editor, plan);
    assert_eq!(editor.buf().text().to_string(), "result result");
    assert_eq!(editor.head(), "result result".len());
    editor.feed(Key::Esc);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "result resSuffix");
}

#[test]
fn text_edit_wins_over_insert_text_and_label_and_imports_undo_with_it() {
    let original = "// header\nresSuffix\n";
    let mut editor = editing(original, 13);
    let candidate = item(json!({
        "label":"label fallback", "insertText":"insert fallback",
        "textEdit":{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":9}},"newText":"result"},
        "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use item;\n"}]
    }));
    let plan = language(
        &context(&editor),
        editor.buf().text(),
        PositionEncoding::Utf8,
        &candidate,
        None,
        false,
        0,
    )
    .unwrap();
    apply(&mut editor, plan);
    let expected = "use item;\n// header\nresult\n";
    assert_eq!(editor.buf().text().to_string(), expected);
    assert_eq!(
        editor.head(),
        expected.find("result").unwrap() + "result".len()
    );
    editor.feed(Key::Esc);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), original);
}

#[test]
fn required_resolve_keeps_text_unchanged_until_the_import_is_available() {
    let original = "// header\nres\n";
    let mut editor = editing(original, 13);
    let candidate = item(json!({"label":"result"}));
    assert!(matches!(
        language(
            &context(&editor),
            editor.buf().text(),
            PositionEncoding::Utf8,
            &candidate,
            None,
            true,
            0
        ),
        Err(AcceptanceRefusal::ResolveRequired)
    ));
    let resolved = strop_lsp::completion::decode_resolved(json!({
        "label":"result", "documentation":"selected item documentation",
        "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use item;\n"}]
    }), &candidate).unwrap();
    let plan = language(
        &context(&editor),
        editor.buf().text(),
        PositionEncoding::Utf8,
        &candidate,
        Some(&resolved),
        true,
        0,
    )
    .unwrap();
    apply(&mut editor, plan);
    assert_eq!(
        editor.buf().text().to_string(),
        "use item;\n// header\nresult\n"
    );
    editor.feed(Key::Esc);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), original);
}

#[test]
fn mirrored_primary_edits_apply_the_document_import_once() {
    let mut editor = editing("// header\nres res\n", 13);
    editor.sels_mut().set_extras([17]);
    let candidate = item(json!({"label":"result",
        "textEdit":{"range":{"start":{"line":1,"character":0},"end":{"line":1,"character":3}},"newText":"result"},
        "additionalTextEdits":[{"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"use item;\n"}]
    }));
    let plan = language(
        &context(&editor),
        editor.buf().text(),
        PositionEncoding::Utf8,
        &candidate,
        None,
        false,
        0,
    )
    .unwrap();
    apply(&mut editor, plan);
    assert_eq!(
        editor.buf().text().to_string(),
        "use item;\n// header\nresult result\n"
    );
    editor.feed(Key::Esc);
    editor.feed_text("u");
    assert_eq!(editor.buf().text().to_string(), "// header\nres res\n");
}

#[test]
fn insert_replace_uses_replacement_and_checks_its_shared_prefix() {
    let mut editor = editing("remainder", 2);
    let candidate = item(json!({"label":"result", "textEdit":{
        "newText":"result", "insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},
        "replace":{"start":{"line":0,"character":0},"end":{"line":0,"character":9}}
    }}));
    let plan = language(
        &context(&editor),
        editor.buf().text(),
        PositionEncoding::Utf8,
        &candidate,
        None,
        false,
        0,
    )
    .unwrap();
    apply(&mut editor, plan);
    assert_eq!(editor.buf().text().to_string(), "result");
    let invalid = item(json!({"label":"result", "textEdit":{
        "newText":"result", "insert":{"start":{"line":0,"character":0},"end":{"line":0,"character":2}},
        "replace":{"start":{"line":0,"character":1},"end":{"line":0,"character":6}}
    }}));
    assert!(matches!(
        language(
            &context(&editor),
            editor.buf().text(),
            PositionEncoding::Utf8,
            &invalid,
            None,
            false,
            0
        ),
        Err(AcceptanceRefusal::InvalidRange)
    ));
}

#[test]
fn invalid_ranges_overlapping_imports_and_unsupported_effects_refuse_whole_items() {
    let editor = editing("a😀res", 8);
    let cases = [
        (
            // a MALFORMED snippet still refuses with the named refusal;
            // a valid one expands (0069 D05)
            json!({"label":"result", "insertText":"${1:result", "insertTextFormat":2}),
            AcceptanceRefusal::Snippet,
        ),
        (
            json!({"label":"result", "command":{"title":"run","command":"external"}}),
            AcceptanceRefusal::Command,
        ),
        (
            json!({"label":"result", "textEdit":{"range":{"start":{"line":0,"character":2},"end":{"line":0,"character":6}},"newText":"result"}}),
            AcceptanceRefusal::InvalidRange,
        ),
        (
            json!({"label":"result", "additionalTextEdits":[
                {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"one"},
                {"range":{"start":{"line":0,"character":0},"end":{"line":0,"character":0}},"newText":"two"}
            ]}),
            AcceptanceRefusal::Overlap,
        ),
    ];
    for (value, expected) in cases {
        let candidate = item(value);
        assert_eq!(
            language(
                &context(&editor),
                editor.buf().text(),
                PositionEncoding::Utf16,
                &candidate,
                None,
                false,
                0
            )
            .err(),
            Some(expected)
        );
    }
}
