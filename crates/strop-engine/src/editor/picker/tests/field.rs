//! The picker field is a real modal field: Esc/i, jk on results,
//! arrows move the caret, paste edits the field.

use super::super::*;
use strop_core::Buffer;

#[test]
fn picker_field_is_modal() {
    // rootle's input boxes: Esc enters normal mode on the field,
    // keys edit the query, i returns to insert, Esc closes
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Files);
    e.feed_text("main");
    e.feed(strop_core::frontend_input::Key::Esc);
    assert!(e.picker_open(), "esc once: picker stays open");
    e.feed_text("0x"); // to 0, delete 'm'
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "ain");
    e.feed(strop_core::frontend_input::Key::Esc);
    assert!(!e.picker_open(), "esc twice closes");
}

#[test]
fn picker_normal_mode_jk_walk_results() {
    // Esc into the field's normal mode; j/k move the selection,
    // not the text caret (user report: only Tab/arrows navigated)
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for f in ["a.txt", "b.txt", "c.txt", "d.txt"] {
        std::fs::write(dir.path().join(f), "x\n").unwrap();
        e.open_fixture(&dir.path().join(f)).unwrap();
    }
    e.open_picker(Kind::Buffers);
    e.wait_picker();
    let sel = |e: &Editor| e.picker.as_ref().unwrap().picker.selected;
    assert_eq!(sel(&e), 1, "the bar rests on the alternate buffer");
    e.feed(strop_core::frontend_input::Key::Esc); // normal mode on the field
    e.feed_text("jj");
    assert_eq!(sel(&e), 3, "j moved the selection down twice");
    e.feed_text("k");
    assert_eq!(sel(&e), 2);
    e.feed_text("i"); // back to insert
    e.feed_text("j"); // types into the query instead
    assert_eq!(
        e.picker.as_ref().unwrap().picker.input.text(),
        "j",
        "insert mode: j filters"
    );
}

/// A query typed, Esc'd into normal mode, then edited with the
/// given keys: (final field text, still-normal?).
fn field_after(query: &str, keys: &str) -> (String, bool) {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Files);
    e.feed_text(query);
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text(keys);
    let picker = &e.picker.as_ref().unwrap().picker;
    (picker.input.text().to_string(), picker.input_normal())
}

#[test]
fn picker_field_normal_mode_is_the_real_grammar() {
    // user report: dw did nothing in the symbol-search input — the
    // field's normal mode is the real vim grammar now (0003 §2)
    assert_eq!(field_after("main", "0dw").0, "");
    assert_eq!(field_after("main", "db").0, "");
    assert_eq!(field_after("main one", "0de").0, " one");
    assert_eq!(field_after("ma:in", "0df:").0, "in");
    assert_eq!(field_after("one two three", "02dw").0, "three");
    assert_eq!(field_after("one two", "0wdiw").0, "one ");
    assert_eq!(field_after("main", "0dd").0, "", "dd clears the field");
    // change re-enters the field's own insert mode: x lands as text
    let (text, normal) = field_after("main", "0cwx");
    assert_eq!((text.as_str(), normal), ("x", false));
    // i/a/A return to insert too
    assert!(!field_after("main", "i").1);
    assert!(!field_after("main", "A").1);
}

#[test]
fn picker_field_refusals_are_surfaced_never_swallowed() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Files);
    e.feed_text("main");
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text("qa"); // macros belong to the document surface
    assert!(e.message.contains("record macro"), "{}", e.message);
    e.feed_text("ds\"");
    assert!(e.message.contains("surround"), "{}", e.message);
    e.feed_text(":");
    assert!(e.message.contains("prompt"), "{}", e.message);
    e.feed_text(">w");
    assert!(e.message.contains("indent"), "{}", e.message);
    assert_eq!(
        e.picker.as_ref().unwrap().picker.input.text(),
        "main",
        "refusals never edit"
    );
}

#[test]
fn picker_arrows_navigate_and_move_caret() {
    // user report: physical arrows did nothing in pickers — the
    // translation layer dropped KeyCode::Up/Down entirely
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for f in ["aa.txt", "ab.txt", "ac.txt"] {
        std::fs::write(dir.path().join(f), "x\n").unwrap();
        e.open_fixture(&dir.path().join(f)).unwrap();
    }
    e.open_picker(Kind::Buffers);
    e.feed_text("a");
    e.wait_picker();
    e.feed(strop_core::frontend_input::Key::Down);
    assert_eq!(
        e.picker.as_ref().unwrap().picker.selected,
        1,
        "Down walks results"
    );
    e.feed(strop_core::frontend_input::Key::Up);
    assert_eq!(e.picker.as_ref().unwrap().picker.selected, 0);
    e.feed(strop_core::frontend_input::Key::Left);
    assert_eq!(
        e.picker.as_ref().unwrap().picker.input.cursor(),
        0,
        "Left moves the caret"
    );
    e.feed(strop_core::frontend_input::Key::Right);
    assert_eq!(e.picker.as_ref().unwrap().picker.input.cursor(), 1);
}

#[test]
fn paste_into_picker_edits_the_field_not_the_document() {
    // user report: bracketed paste in a picker or the connect-to-
    // remote field was silently dropped (or worse, could land in
    // the buffer behind the card)
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Files);
    e.paste_bracketed("main");
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "main");
    assert_eq!(e.buf().text(), "x\n", "the document is untouched");
    // caret placement is honored
    e.feed(strop_core::frontend_input::Key::Left);
    e.feed(strop_core::frontend_input::Key::Left);
    e.paste_bracketed("__");
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "ma__in");
    // paste works in the field's normal mode too (the ex line's
    // pending reducer pastes the same way)
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text("0");
    e.paste_bracketed("#");
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "#ma__in");
}

#[test]
fn paste_newline_into_picker_is_rejected_with_a_message() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Files);
    e.paste_bracketed("a\nb");
    assert_eq!(
        e.message, "picker input cannot contain a newline",
        "a refused paste says so — silence reads as a broken terminal"
    );
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "");
}

#[test]
fn remote_hosts_enter_carries_unmatched_text_into_the_address_box() {
    // 0.21.0 field report: a typed hostname that matched no listed
    // destination made Enter a no-op. Now the pinned "Add a host…"
    // row survives filtering and Enter carries the text into the
    // address box as a draft — never a blind connect to a host
    // literally named like the filter text.
    let mut e = Editor::new(Buffer::from_text("x\n"));
    let mut picker = Picker::new(
        Kind::RemoteHosts,
        vec![Item {
            badge: None,
            text: "Add a host\u{2026}".into(),
            payload: Payload::RemoteConnect,
            facts: Default::default(),
        }],
        false,
    );
    picker.pinned_tail = 1;
    e.set_picker(PickerGlue::diagnostics(picker));
    e.feed_text("ewosd-tt-925");
    e.wait_picker();
    assert_eq!(
        e.picker
            .as_ref()
            .unwrap()
            .picker
            .current()
            .map(|i| i.text.as_str()),
        Some("Add a host\u{2026}"),
        "the pinned row survives a dead query"
    );
    e.accept_current_picker();
    let glue = e.picker.as_ref().expect("the address box opens");
    assert_eq!(glue.picker.kind, Kind::RemoteAddress);
    assert_eq!(
        glue.picker.input.text(),
        "ewosd-tt-925",
        "the typed destination arrives as the address draft"
    );
}

#[test]
fn remote_hosts_empty_enter_opens_the_address_box() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    let mut picker = Picker::new(
        Kind::RemoteHosts,
        vec![Item {
            badge: None,
            text: "Add a host\u{2026}".into(),
            payload: Payload::RemoteConnect,
            facts: Default::default(),
        }],
        false,
    );
    picker.pinned_tail = 1;
    e.set_picker(PickerGlue::diagnostics(picker));
    e.wait_picker();
    e.accept_current_picker();
    assert_eq!(
        e.picker.as_ref().map(|glue| glue.picker.kind),
        Some(Kind::RemoteAddress),
        "empty input on the pinned row still opens the address box"
    );
}

#[test]
fn paste_into_remote_address_field_is_accepted_verbatim() {
    // the connect-to-remote case: an ssh://user@host:port/path URL
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::RemoteAddress);
    e.paste_bracketed("ssh://user@example.com:2222/tmp/dir");
    assert_eq!(
        e.picker.as_ref().unwrap().picker.input.text(),
        "ssh://user@example.com:2222/tmp/dir"
    );
}
