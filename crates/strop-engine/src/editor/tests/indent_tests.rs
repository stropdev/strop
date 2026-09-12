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
    // tabs document (detected from content): Tab inserts one tab.
    // Detection needs real evidence (0051 R08): one line is a guess.
    let mut e = Editor::new(Buffer::from_text(
        "\t\ta = 1\n\t\tb = 2\n\t\tc = 3\n\t\tx = 1\n",
    ));
    e.reresolve_indents();
    assert_eq!(e.cur().indent.style, crate::config::IndentStyle::Tabs);
    e.feed_text("Gi");
    e.feed(crate::editor::Key::Tab);
    assert_eq!(
        e.buf().text().to_string(),
        "\t\ta = 1\n\t\tb = 2\n\t\tc = 3\n\t\t\tx = 1\n"
    );
}

#[test]
fn detection_follows_the_files_own_convention() {
    // a two-space file: >> adds two, not the config default four.
    // (0051 R08: detection needs ≥4 evidence lines — three is a guess)
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
    ));
    e.reresolve_indents();
    assert_eq!(e.cur().indent.width, 2);
    e.feed_text(">>");
    assert_eq!(e.buf().line_text(0), "  fn f() {");
}

#[test]
fn detection_off_uses_config_despite_content() {
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
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

/// 0051 R08: a tab-indented file keeps the configured display width —
/// detection of the Tab style is not a license to hardcode 4.
#[test]
fn detected_tabs_keep_the_configured_width() {
    let mut e = Editor::new(Buffer::from_text("\ta;\n\tb;\n\t\tc;\n\td;\n"));
    e.config = crate::config::Config {
        tab_size: 8,
        ..Default::default()
    };
    e.reresolve_indents();
    assert_eq!(e.cur().indent.style, crate::config::IndentStyle::Tabs);
    assert_eq!(
        e.cur().indent.width,
        8,
        "width comes from config, not detection"
    );
    assert_eq!(
        e.cur().indent.width_source,
        crate::editor::document::IndentSource::Configured
    );
}

/// 0051 R08: `:tab-size N` beats detection and config — for rendering
/// and new indentation, never for existing bytes.
#[test]
fn tab_size_override_beats_detection() {
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
    ));
    e.reresolve_indents();
    assert_eq!(e.cur().indent.width, 2);
    e.tab_size_command("8");
    let indent = e.cur().indent;
    assert_eq!(indent.width, 8);
    assert_eq!(
        indent.width_source,
        crate::editor::document::IndentSource::Manual
    );
    // the detected style survives; only the width was overridden
    assert_eq!(indent.style, crate::config::IndentStyle::Spaces);
    assert_eq!(
        indent.style_source,
        crate::editor::document::IndentSource::Detected
    );
    assert!(
        e.message.contains("manual"),
        "feedback names the provenance: {}",
        e.message
    );
    e.feed_text(">>");
    assert_eq!(e.buf().line_text(0), "        fn f() {");
}

/// 0051 R08: `:tab-size auto` clears the width override; the
/// detection/config policy decides again.
#[test]
fn tab_size_auto_clears_the_override() {
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
    ));
    e.reresolve_indents();
    e.tab_size_command("8");
    assert_eq!(e.cur().indent.width, 8);
    e.tab_size_command("auto");
    let indent = e.cur().indent;
    assert_eq!(indent.width, 2, "detection decides again");
    assert_eq!(
        indent.width_source,
        crate::editor::document::IndentSource::Detected
    );
}

/// 0051 R08: widths outside 1–16 are refused visibly; nothing changes.
#[test]
fn tab_size_out_of_range_is_refused_visibly() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for bad in ["0", "17", "4000", "banana", "4.5"] {
        e.tab_size_command(bad);
        assert!(
            e.message.contains("1–16"),
            "{bad:?}: refusal names the range: {}",
            e.message
        );
        assert_eq!(e.cur().indent.width, 4, "{bad:?}: nothing changed");
        assert_eq!(e.cur().indent_override.width, None);
    }
    // the bounds themselves are accepted
    e.tab_size_command("1");
    assert_eq!(e.cur().indent.width, 1);
    e.tab_size_command("16");
    assert_eq!(e.cur().indent.width, 16);
}

/// 0051 R08: `:indent-style` overrides the style side only; `auto`
/// clears it; anything else is refused visibly.
#[test]
fn indent_style_override_and_refusal() {
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
    ));
    e.reresolve_indents();
    e.indent_style_command("tabs");
    let indent = e.cur().indent;
    assert_eq!(indent.style, crate::config::IndentStyle::Tabs);
    assert_eq!(
        indent.style_source,
        crate::editor::document::IndentSource::Manual
    );
    // a manual tabs style cannot reuse the detected spaces width
    assert_eq!(indent.width, 4, "width falls back to configured");
    e.feed_text(">>");
    assert_eq!(e.buf().line_text(0), "\tfn f() {");
    e.indent_style_command("auto");
    assert_eq!(e.cur().indent.style, crate::config::IndentStyle::Spaces);
    assert_eq!(e.cur().indent.width, 2, "detection decides again");
    e.indent_style_command("bogus");
    assert!(
        e.message.contains("spaces, tabs or auto"),
        "visible refusal: {}",
        e.message
    );
}

/// 0051 R08: overrides live on the document — config refreshes and
/// other buffers never change them.
#[test]
fn override_survives_refresh_and_other_buffers() {
    let dir = tempfile::tempdir().unwrap();
    let other = dir.path().join("other.txt");
    std::fs::write(
        &other,
        "        eight\n        eight\n        eight\n        eight\n",
    )
    .unwrap();
    let mut e = Editor::new(Buffer::from_text(
        "fn f() {\n  let x = 1;\n  let y = 2;\n  let z = 3;\n  let w = 4;\n}\n",
    ));
    e.reresolve_indents();
    e.tab_size_command("3");
    assert_eq!(e.cur().indent.width, 3);
    let first = e.current();
    // a config refresh (startup layering) preserves the override
    e.reresolve_indents();
    assert_eq!(e.cur().indent.width, 3, "refresh preserves the override");
    // opening another buffer never touches this document's override
    let second = e.open_fixture(&other).unwrap();
    assert_eq!(e.current(), second);
    assert_eq!(e.cur().indent.width, 8, "the other buffer resolves its own");
    e.switch_to(first);
    assert_eq!(e.cur().indent.width, 3, "the override is per buffer");
}

/// 0051 R08: the selector shows current setting + provenance, applies
/// rows, and validates a typed custom width on accept.
#[test]
fn tab_size_selector_applies_and_validates() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.tab_size_command("");
    let glue = e
        .picker
        .as_ref()
        .expect("bare :tab-size opens the selector");
    assert_eq!(glue.picker.kind, strop_picker::Kind::TabSize);
    let texts: Vec<&str> = glue.picker.items.iter().map(|i| i.text.as_str()).collect();
    assert!(
        glue.picker.items.iter().any(|i| {
            i.badge.as_deref() == Some("4") && i.text.contains("current (configured)")
        }),
        "the current setting and provenance are marked: {texts:?}"
    );
    assert!(
        texts.iter().any(|t| t.contains("style: spaces")),
        "style choices ride the same selector: {texts:?}"
    );
    // Acceptance goes through the selector that owns the captured source.
    e.feed_text("3");
    e.wait_picker();
    e.feed(crate::editor::Key::Enter);
    assert_eq!(e.cur().indent.width, 3);
    assert_eq!(
        e.cur().indent.width_source,
        crate::editor::document::IndentSource::Manual
    );
    // a valid typed custom width applies; invalid ones refuse visibly
    e.accept_indent_choice(e.current(), strop_picker::IndentChoice::CustomWidth, "5");
    assert_eq!(e.cur().indent.width, 5);
    e.accept_indent_choice(e.current(), strop_picker::IndentChoice::CustomWidth, "99");
    assert!(e.message.contains("1–16"), "{}", e.message);
    assert_eq!(e.cur().indent.width, 5, "refusal changes nothing");
    e.accept_indent_choice(e.current(), strop_picker::IndentChoice::CustomWidth, "");
    assert!(e.message.contains("type a width"), "{}", e.message);
}
