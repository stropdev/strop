//! The jumplist as a menu (0047 §2).

use super::super::*;
use strop_core::Buffer;

#[test]
fn jumps_picker_walks_history_and_ctrl_o_returns() {
    // Older entries are newest-first, the current position is marked,
    // and accepting an entry records the spot so ctrl-o returns.
    let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\nfour\n"));
    e.feed_text("G"); // jump to line 4
    e.feed_text("gg"); // jump back to line 1
    e.open_picker(Kind::Jumps);
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    let texts: Vec<&str> = glue.picker.items.iter().map(|i| i.text.as_str()).collect();
    assert_eq!(texts.len(), 3, "two jumps + the current row: {texts:?}");
    assert!(
        texts[0].contains(":4") && texts[0].contains("four"),
        "newest past entry first: {texts:?}"
    );
    assert!(
        texts[2].starts_with("> "),
        "the current row is marked: {texts:?}"
    );
    e.feed(strop_core::frontend_input::Key::Enter);
    assert!(!e.picker_open(), "accept closes the picker");
    assert_eq!(e.buf().line_of(e.head()), 3, "landed on the accepted entry");
    e.feed(strop_core::frontend_input::Key::CtrlO);
    assert_eq!(
        e.buf().line_of(e.head()),
        0,
        "ctrl-o returns to where the picker was opened"
    );
}

#[test]
fn jumps_picker_projects_older_selected_live_and_newer_rows() {
    let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\nfour\n"));
    e.feed_text("G3Ggg");
    e.feed(strop_core::frontend_input::Key::CtrlO);
    e.feed_text("j");
    e.open_picker(Kind::Jumps);
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    let texts: Vec<&str> = glue
        .picker
        .items
        .iter()
        .map(|item| item.text.as_str())
        .collect();
    assert_eq!(texts.len(), 4, "older + selected + live + newer: {texts:?}");
    assert!(texts[0].contains(":4"));
    assert!(!texts[1].starts_with("> ") && texts[1].contains(":3"));
    assert!(texts[2].starts_with("> ") && texts[2].contains(":4"));
    assert!(!texts[3].starts_with("> ") && texts[3].contains(":1"));
}

#[test]
fn jumps_picker_filters_dead_documents() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("other.txt");
    std::fs::write(&path, "x\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
    e.open_fixture(&path).unwrap();
    e.feed_text("G"); // a jump inside other.txt
    let dead = e.current();
    e.close_buffer(true);
    assert!(e.docs.get(dead).is_none(), "the document is gone");
    e.open_picker(Kind::Jumps);
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    assert!(
        glue.picker
            .items
            .iter()
            .all(|i| !i.text.contains("other.txt")),
        "dead-document entries are absent: {:?}",
        glue.picker
            .items
            .iter()
            .map(|i| i.text.as_str())
            .collect::<Vec<_>>()
    );
}

#[test]
fn jumps_picker_speaks_the_query_language() {
    // Consistency (0051 §3 / 0109): the jumplist menu takes the path
    // filters its rows carry — flat (`language:`, `path:`) and Boolean
    // — and the ranking pass applies them to the kept rows.
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("lib.rs"), "fn one() {}\nfn two() {}\n").unwrap();
    std::fs::create_dir(dir.path().join("tests")).unwrap();
    std::fs::write(dir.path().join("tests/widget.rs"), "fn helper() {}\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    e.open_fixture(&dir.path().join("lib.rs")).unwrap();
    e.feed_text("G");
    std::fs::write(dir.path().join("app.py"), "print('one')\nprint('two')\n").unwrap();
    e.open_fixture(&dir.path().join("app.py")).unwrap();
    e.feed_text("G");
    e.open_fixture(&dir.path().join("tests/widget.rs")).unwrap();
    e.feed_text("G");
    e.open_picker(Kind::Jumps);
    e.wait_picker();
    fn texts(e: &Editor) -> Vec<&str> {
        let picker = &e.picker.as_ref().unwrap().picker;
        picker
            .rows
            .iter()
            .map(|row| picker.items[row.item].text.as_str())
            .collect()
    }
    let whole = texts(&e);
    assert!(
        whole.iter().any(|row| row.contains("lib.rs"))
            && whole.iter().any(|row| row.contains("app.py"))
            && whole.iter().any(|row| row.contains("tests/widget.rs")),
        "the full history lists every document: {whole:?}"
    );

    // A flat qualifier narrows exactly like file find: rust rows
    // stay, the python row drops, undecidable evidence admits.
    e.paste_bracketed("language:rust");
    e.wait_picker();
    let narrowed = texts(&e);
    assert!(
        narrowed.iter().any(|row| row.contains("lib.rs"))
            && narrowed.iter().any(|row| row.contains("tests/widget.rs")),
        "rust jumps stay: {narrowed:?}"
    );
    assert!(
        narrowed.iter().all(|row| !row.contains("app.py")),
        "the python jump is filtered out: {narrowed:?}"
    );

    // Qualifiers compose; path: narrows further.
    e.paste_bracketed(" path:tests");
    e.wait_picker();
    let narrowed = texts(&e);
    assert!(
        narrowed
            .iter()
            .all(|row| !row.contains("lib.rs") && !row.contains("app.py")),
        "only the tests/ jump remains: {narrowed:?}"
    );

    // The Boolean grammar narrows through line evidence too.
    e.picker.as_mut().unwrap().picker.input.set_text("");
    e.paste_bracketed("language:rust AND helper");
    e.wait_picker();
    let narrowed = texts(&e);
    assert!(
        narrowed.iter().all(|row| row.contains("tests/widget.rs")),
        "content atoms match the jumped line's text: {narrowed:?}"
    );

    // The kept list is never rebuilt: plain text ranks it.
    e.picker.as_mut().unwrap().picker.input.set_text("");
    e.paste_bracketed("helper");
    e.wait_picker();
    let picker = &e.picker.as_ref().unwrap().picker;
    assert!(
        picker.items.iter().any(|item| item.text.contains("app.py")),
        "the kept list stays whole"
    );
    assert!(texts(&e).iter().all(|row| row.contains("tests/widget.rs")));

    // kind: is not a filter the jumps carry (0109 decision 4).
    e.picker.as_mut().unwrap().picker.input.set_text("");
    e.paste_bracketed("kind:fn");
    e.wait_picker();
    let error = e.picker.as_ref().unwrap().picker.error.clone();
    assert!(
        error
            .as_deref()
            .is_some_and(|error| error.starts_with("kind: does not filter jumps")),
        "{error:?}"
    );

    // An incomplete query keeps the rows and explains itself.
    e.picker.as_mut().unwrap().picker.input.set_text("");
    e.paste_bracketed("language:");
    e.wait_picker();
    let glue = e.picker.as_ref().unwrap();
    assert!(
        glue.picker
            .error
            .as_deref()
            .is_some_and(|error| error.contains("expects a value")),
        "the diagnostic is on the card: {:?}",
        glue.picker.error
    );
    assert!(
        texts(&e).iter().any(|row| row.contains("tests/widget.rs")),
        "rows stay while the query is incomplete"
    );
}

#[test]
fn jumps_preview_tracks_selection_and_rejects_a_closed_document() {
    let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
    let document = e.current();
    let label = e.doc(document).label(&e.cwd);
    let last_line = e.doc(document).buf.line_of(e.doc(document).buf.len_bytes()) + 1;
    let beyond_eof = e.doc(document).buf.len_bytes() + 100;
    e.open_picker(Kind::Jumps);
    e.picker_items_fixture(vec![
        Item {
            badge: None,
            text: "first".into(),
            payload: Payload::Jump {
                document,
                offset: 0,
            },
            facts: Default::default(),
        },
        Item {
            badge: None,
            text: "clamped".into(),
            payload: Payload::Jump {
                document,
                offset: beyond_eof,
            },
            facts: Default::default(),
        },
    ]);
    e.wait_picker();
    // Opening Jumps records the current position before the two fixture rows.
    e.feed(strop_core::frontend_input::Key::Down);

    for focus in [1, last_line] {
        let preview = e.picker_preview().unwrap();
        assert_eq!(preview.0, PreviewTitle::local(label.clone()));
        assert_eq!(preview.1, Some(focus));
        assert!(matches!(
            preview.2,
            PreviewSource::Buffer(source) if source == document
        ));
        let cached = e.picker_preview_cached().unwrap();
        assert_eq!(cached.0, preview.0);
        assert_eq!(cached.1, preview.1);
        assert!(matches!(
            cached.2,
            PreviewSource::Buffer(source) if source == document
        ));
        e.feed(strop_core::frontend_input::Key::Down);
    }

    e.docs.remove(document);
    assert!(e.picker_preview().is_none());
    assert!(e.picker_preview_cached().is_none());
}
