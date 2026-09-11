use super::*;

#[test]
fn enter_copies_and_deepens_indent() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n    let x = 1;\n}\n"));
    e.feed_text("j$"); // on the let line, at EOL
    e.feed(crate::editor::Key::Char('a'));
    e.feed(crate::editor::Key::Enter);
    e.feed_text("let y = 2;");
    assert_eq!(
        e.buf().text().to_string(),
        "fn f() {\n    let x = 1;\n    let y = 2;\n}\n"
    );
    // after an opener, one level deeper
    e.feed(crate::editor::Key::Esc);
    e.feed_text("gg$");
    e.feed(crate::editor::Key::Char('a'));
    e.feed(crate::editor::Key::Enter);
    e.feed_text("// body");
    let got = e.buf().text().to_string();
    assert!(got.starts_with("fn f() {\n    // body"), "got: {got:?}");
}

#[test]
fn o_auto_indents() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o");
    e.feed_text("let x = 1;");
    assert_eq!(e.buf().text().to_string(), "fn f() {\n    let x = 1;\n}\n");
}

#[test]
fn tab_size_from_config() {
    let mut e = Editor::new(Buffer::from_text("a\nb\n"));
    e.config = crate::config::Config {
        tab_size: 2,
        ..Default::default()
    };
    e.reresolve_indents();
    e.feed_text(">>");
    assert_eq!(e.buf().text().to_string(), "  a\nb\n");
    e.feed_text("<<");
    assert_eq!(e.buf().text().to_string(), "a\nb\n");
}

#[test]
fn new_file_opens_empty_and_saves() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.rs");
    let mut e = Editor::new(Buffer::open(&path).expect("missing file is a new buffer"));
    assert_eq!(e.buf().len_bytes(), 0);
    e.feed_text("ifresh");
    e.feed(crate::editor::Key::Esc);
    e.feed_text(":w<cr>");
    e.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "fresh");
}

#[test]
fn tab_key_inserts_the_documents_indent_unit() {
    // spaces document: Tab inserts the width in spaces
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.feed_text("i");
    e.feed(crate::editor::Key::Tab);
    assert_eq!(e.buf().text().to_string(), "    x\n");
    e.feed(crate::editor::Key::Esc);
    // tabs document (detected from content): Tab inserts one tab
    let mut e = Editor::new(Buffer::from_text("\t\tx = 1\n"));
    e.reresolve_indents();
    assert_eq!(e.cur().indent.style, crate::config::IndentStyle::Tabs);
    e.feed_text("Gi");
    e.feed(crate::editor::Key::Tab);
    assert_eq!(e.buf().text().to_string(), "\t\t\tx = 1\n");
}

#[test]
fn detection_follows_the_files_own_convention() {
    // a two-space file: >> adds two, not the config default four
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n}\n",
    ));
    e.reresolve_indents();
    assert_eq!(e.cur().indent.width, 2);
    e.feed_text(">>");
    assert_eq!(e.buf().line_text(0), "  fn f() {");
}

#[test]
fn detection_off_uses_config_despite_content() {
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n}\n",
    ));
    e.config = crate::config::Config {
        indent_detect: false,
        ..Default::default()
    };
    e.reresolve_indents();
    assert_eq!(e.cur().indent.width, 4, "config wins when detection is off");
}

#[test]
fn tabs_style_config_indents_with_tabs() {
    let mut e = Editor::new(Buffer::from_text("a\nb\n"));
    e.config = crate::config::Config {
        indent_style: crate::config::IndentStyle::Tabs,
        ..Default::default()
    };
    e.reresolve_indents();
    e.feed_text(">>");
    assert_eq!(e.buf().line_text(0), "\ta");
    e.feed_text("<<");
    assert_eq!(e.buf().line_text(0), "a");
}
