use super::*;

#[test]
fn enter_copies_and_deepens_indent() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n    let x = 1;\n}\n"));
    e.feed_text("j$"); // on the let line, at EOL
    e.feed(crate::editor::Key::Char('a'));
    e.feed(crate::editor::Key::Enter);
    e.feed_text("let y = 2;");
    assert_eq!(
        e.buf().rope.to_string(),
        "fn f() {\n    let x = 1;\n    let y = 2;\n}\n"
    );
    // after an opener, one level deeper
    e.feed(crate::editor::Key::Esc);
    e.feed_text("gg$");
    e.feed(crate::editor::Key::Char('a'));
    e.feed(crate::editor::Key::Enter);
    e.feed_text("// body");
    let got = e.buf().rope.to_string();
    assert!(got.starts_with("fn f() {\n    // body"), "got: {got:?}");
}

#[test]
fn o_auto_indents() {
    let mut e = Editor::new(Buffer::from_text("fn f() {\n}\n"));
    e.feed_text("o");
    e.feed_text("let x = 1;");
    assert_eq!(e.buf().rope.to_string(), "fn f() {\n    let x = 1;\n}\n");
}

#[test]
fn tab_size_from_config() {
    let mut e = Editor::new(Buffer::from_text("a\nb\n"));
    e.config = crate::config::Config {
        tab_size: 2,
        ..Default::default()
    };
    e.feed_text(">>");
    assert_eq!(e.buf().rope.to_string(), "  a\nb\n");
    e.feed_text("<<");
    assert_eq!(e.buf().rope.to_string(), "a\nb\n");
}

#[test]
fn new_file_opens_empty_and_saves() {
    let path = "/tmp/strop-newfile-test.rs";
    std::fs::remove_file(path).ok();
    let mut e = Editor::new(Buffer::open(path).expect("missing file is a new buffer"));
    assert_eq!(e.buf().len_bytes(), 0);
    e.feed_text("ifresh");
    e.feed(crate::editor::Key::Esc);
    e.feed_text(":w<cr>");
    assert_eq!(std::fs::read_to_string(path).unwrap(), "fresh");
    std::fs::remove_file(path).ok();
}
