use super::*;

#[test]
fn vi_paren_selects_inner() {
    let mut e = Editor::new(Buffer::from_text("call(a, b)\n"));
    e.feed_text("f("); // onto the open paren
    e.feed_text("vi(");
    let r = e.visual_range().expect("visual range");
    assert_eq!(e.buf().slice_string(r), "a, b");
    // and operators consume it
    e.feed_text("d");
    assert_eq!(e.buf().text().to_string(), "call()\n");
}

#[test]
fn va_quote_includes_quotes() {
    let mut e = Editor::new(Buffer::from_text("say \"hi\" now\n"));
    e.feed_text("w"); // onto "hi"
    e.feed_text("va\"");
    let r = e.visual_range().expect("visual range");
    assert_eq!(e.buf().slice_string(r), "\"hi\"");
}

#[test]
fn ctrl_c_toggles_whole_selected_lines_and_undo_restores_once() {
    for (extension, token) in [("rs", "//"), ("py", "#"), ("lua", "--")] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(format!("example.{extension}"));
        std::fs::write(&path, "  one\n\n  two\n").unwrap();
        let mut e = Editor::new(Buffer::from_text(""));
        e.open_fixture(&path).unwrap();
        e.feed_text("V2j");
        e.feed(strop_core::frontend_input::Key::CtrlC);
        assert_eq!(
            e.buf().text().to_string(),
            format!("  {token} one\n\n  {token} two\n")
        );
        assert_eq!(e.mode, Mode::Normal);
        assert!(!e.should_quit());
        e.feed_text("u");
        assert_eq!(e.buf().text().to_string(), "  one\n\n  two\n");
        e.feed_text("V2j");
        e.feed(strop_core::frontend_input::Key::CtrlC);
        e.feed_text("V2j");
        e.feed(strop_core::frontend_input::Key::CtrlC);
        assert_eq!(e.buf().text().to_string(), "  one\n\n  two\n");
    }
}

#[test]
fn visual_ctrl_c_expands_partial_lines_and_refuses_unknown_languages() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("example.rs");
    std::fs::write(&path, "one\ntwo\nthree\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.open_fixture(&path).unwrap();
    e.feed_text("vjl");
    e.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(e.buf().text().to_string(), "// one\n// two\nthree\n");
    let mut plain = Editor::new(Buffer::from_text("one\ntwo\n"));
    plain.feed_text("Vj");
    plain.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(plain.buf().text(), "one\ntwo\n");
    assert!(plain.message.contains("recognized file language"));
    assert!(!plain.should_quit());
}

#[test]
fn visual_comments_preserve_crlf_and_only_uncomment_complete_runs() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("example.rs");
    std::fs::write(&path, "// one\r\n//two\r\nthree\r\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.open_fixture(&path).unwrap();
    e.feed_text("V2j");
    e.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(
        e.buf().text().to_string(),
        "// // one\r\n// //two\r\n// three\r\n"
    );
    e.feed_text("V2j");
    e.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(e.buf().text().to_string(), "// one\r\n//two\r\nthree\r\n");
    e.feed_text("ggVj");
    e.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(e.buf().text().to_string(), "one\r\ntwo\r\nthree\r\n");
}

#[test]
fn insert_register_is_literal_and_replayed_by_dot() {
    let mut e = Editor::new(Buffer::from_text("yank\nstart\nend\n"));
    e.feed_text("yiwji");
    e.feed(strop_core::frontend_input::Key::CtrlR);
    e.feed_text("\"");
    e.feed(strop_core::frontend_input::Key::CtrlC);
    assert_eq!(e.buf().text().to_string(), "yank\nyankstart\nend\n");
    e.feed_text("j0.");
    assert_eq!(e.buf().text().to_string(), "yank\nyankstart\nyankend\n");
    assert!(!e.should_quit());
}
