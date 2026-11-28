//! Structural indent on the document's one syntax tree (#309).
use super::smartindent_tests::{code_editor, settle_indentation};

/// 400 functions: past every lexical shortcut, nested three deep.
fn source() -> String {
    let mut text = String::new();
    for index in 0..400 {
        text.push_str(&format!(
            "fn f{index}(x: u32) -> u32 {{\n    if x > {index} {{\n        call(x,\n            {index})\n    }} else {{\n        x\n    }}\n}}\n\n"
        ));
    }
    text
}

#[test]
fn a_published_tree_answers_o_and_enter_on_the_input_path() {
    let text = source();
    let mut editor = code_editor(&text, "big.rs");
    editor.analysis_fixture();
    let admitted = editor.admission_probe().resolutions;
    let line = text.find("fn f300(").unwrap() + "fn f300(x: u32) -> u32 {\n    if x > 300 {".len();
    editor.set_head(line - 1);
    editor.feed_text("o");
    assert!(
        !editor.resolution.blocked(),
        "the tree answered without a job"
    );
    editor.feed_text("y<esc>");
    let opened = editor.buf().text().to_string();
    assert!(opened.contains("    if x > 300 {\n        y\n        call(x,"));
    // inside the open call: Enter lands one unit in, the closer aligns
    editor.analysis_fixture();
    let at = opened.find("call(x,\n            300)").unwrap() + "call(".len();
    editor.set_head(at);
    editor.feed_text("i<cr><esc>");
    assert!(!editor.resolution.blocked());
    assert!(editor
        .buf()
        .text()
        .to_string()
        .contains("        call(\n            x,\n            300)"));
    assert_eq!(editor.admission_probe().resolutions, admitted);
}

#[test]
fn a_tree_behind_the_buffer_defers_and_keeps_typeahead_in_order() {
    let text = source();
    let mut editor = code_editor(&text, "big.rs");
    editor.analysis_fixture();
    let line = text.find("fn f200(").unwrap() + "fn f200(x: u32) -> u32 {".len();
    editor.set_head(line);
    // an edit the published tree has not seen yet
    editor.feed_text("a <esc>");
    let admitted = editor.admission_probe().resolutions;
    editor.feed_text("oxyz<esc>");
    assert!(editor.resolution.blocked());
    settle_indentation(&mut editor);
    assert_eq!(editor.admission_probe().resolutions, admitted + 1);
    assert!(editor
        .buf()
        .text()
        .to_string()
        .contains("fn f200(x: u32) -> u32 { \n    xyz\n    if x > 200 {"));
}

#[test]
fn a_file_too_large_to_parse_indents_from_the_previous_line_and_says_so_once() {
    let unit = "fn f() {\n    if x {\n        y(\n";
    let text = unit.repeat(strop_syntax::PARSE_LIMIT / unit.len() + 1);
    let mut editor = code_editor(&text, "huge.rs");
    editor.set_head(text.len() - 1);
    editor.feed_text("o");
    assert!(!editor.resolution.blocked(), "no parse starts");
    assert!(editor.message.contains("too large to parse"));
    editor.feed_text("z<esc>");
    assert!(editor
        .buf()
        .text()
        .to_string()
        .ends_with("        y(\n        z\n"));
    editor.message.clear();
    editor.feed_text("o<esc>");
    assert!(editor.message.is_empty(), "said once");
}
