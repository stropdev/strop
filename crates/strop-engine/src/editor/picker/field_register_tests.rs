//! Plan 0092 §1: every text input reads the editor's one register file.
use super::super::*;
use crate::editor::registers::PasteDestination;
use crate::editor::Register;
use strop_core::frontend_input::Key;
use strop_core::Buffer;

fn query(e: &Editor) -> String {
    e.picker.as_ref().unwrap().picker.input.text().to_owned()
}

fn ctrl_r(e: &mut Editor, name: char) {
    e.feed(Key::CtrlR);
    e.feed(Key::Char(name));
}

#[test]
fn a_yank_pastes_with_ctrl_r_in_every_text_input() {
    // the reported bug: `yiw`, then Ctrl-R " in Files, Search, / and :
    for kind in [Kind::Files, Kind::Search] {
        let mut e = Editor::new(Buffer::from_text("Needle\nneedle\n"));
        e.feed_text("yiw");
        e.open_picker(kind);
        ctrl_r(&mut e, '"');
        assert_eq!(query(&e), "Needle", "{kind:?}");
        assert_eq!(e.buf().text(), "Needle\nneedle\n");
    }
    for sigil in ["/", ":"] {
        let mut e = Editor::new(Buffer::from_text("Needle\nneedle\n"));
        e.feed_text("yiw");
        e.feed_text(sigil);
        ctrl_r(&mut e, '"');
        assert_eq!(e.pending.text(), format!("{sigil}Needle"));
    }
    let mut e = Editor::new(Buffer::from_text("Needle\nneedle\n"));
    e.feed_text("yiw/");
    ctrl_r(&mut e, '"');
    e.feed_text("<cr>");
    assert_eq!(e.head(), 7, "the pasted pattern searches");
}

#[test]
fn search_with_field_takes_ctrl_r_and_ctrl_t_toggles_it() {
    let mut e = Editor::new(Buffer::from_text("Needle\n"));
    e.feed_text("yiw");
    e.open_picker(Kind::Search);
    e.feed(Key::CtrlT);
    assert!(e.picker.as_ref().unwrap().picker.replacement_visible);
    assert_eq!(
        e.picker.as_ref().unwrap().picker.field,
        strop_picker::Field::Replace
    );
    ctrl_r(&mut e, '0');
    assert_eq!(
        e.picker.as_ref().unwrap().picker.replace_input.text(),
        "Needle"
    );
    assert_eq!(query(&e), "", "the Find field is untouched");
    e.feed(Key::CtrlT);
    assert!(!e.picker.as_ref().unwrap().picker.replacement_visible);
}

#[test]
fn fields_read_the_same_named_numbered_and_small_registers_as_put() {
    let mut e = Editor::new(Buffer::from_text("alpha beta end\ngamma\ndelta\n"));
    // "a ← alpha, "- ← "beta " (dw within the line), "1 ← gamma line, "0 ← delta
    e.feed_text("\"ayiwwdwjddyiw");
    for (name, expected) in [
        ('a', "alpha"),
        ('-', "beta "),
        ('1', "gamma"),
        ('0', "delta"),
        ('"', "delta"),
    ] {
        e.open_picker(Kind::Files);
        ctrl_r(&mut e, name);
        assert_eq!(query(&e), expected, "Ctrl-R {name}");
        e.close_picker();
        assert_eq!(
            e.register(Some(name)).text.trim_end_matches('\n'),
            expected,
            "the store agrees for {name}"
        );
    }
}

#[test]
fn read_only_registers_are_views_of_editor_state() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("note.txt");
    std::fs::write(&path, "one two\n").unwrap();
    let mut e = Editor::new(Buffer::open(&path).unwrap());
    e.feed_text("/two<cr>:2<cr>");
    e.feed_text(":");
    ctrl_r(&mut e, '/');
    assert_eq!(e.pending.text(), ":two");
    ctrl_r(&mut e, ':');
    assert_eq!(e.pending.text(), ":two2");
    e.feed(Key::Esc);
    e.feed(Key::Esc);
    e.open_picker(Kind::Files);
    ctrl_r(&mut e, '%');
    assert!(query(&e).ends_with("note.txt"), "{}", query(&e));
    e.close_picker();
    // an operator cannot write them, and the refused delete edits nothing
    e.feed_text("\":dd");
    assert_eq!(e.buf().text(), "one two\n");
    assert!(e.message.contains("read-only"), "{}", e.message);
}

#[test]
fn a_one_line_linewise_register_pastes_its_line_and_more_lines_refuse() {
    let mut e = Editor::new(Buffer::from_text("first\nsecond\n"));
    e.feed_text("yy");
    e.open_picker(Kind::Files);
    ctrl_r(&mut e, '"');
    assert_eq!(query(&e), "first");
    e.close_picker();
    e.feed_text("yj");
    e.open_picker(Kind::Files);
    e.feed_text("keep");
    ctrl_r(&mut e, '"');
    assert_eq!(query(&e), "keep");
    assert!(e.message.contains("newline"));
}

#[test]
fn field_normal_mode_puts_with_p_and_a_named_register() {
    let mut e = Editor::new(Buffer::from_text("Needle\n"));
    e.feed_text("yiw");
    e.yank_register(Some('a'), Register::characterwise("é"));
    e.yank_register(None, Register::characterwise("Needle"));
    e.open_picker(Kind::Files);
    e.feed(Key::Esc);
    e.feed_text("p");
    assert_eq!(query(&e), "Needle");
    e.feed_text("0P");
    assert_eq!(query(&e), "NeedleNeedle");
    e.feed_text("\"ap");
    assert_eq!(query(&e), "NeedleéNeedle");
    e.close_picker();
    // the `:` line speaks the same normal-mode put
    e.feed_text(":");
    e.feed(Key::Esc);
    e.feed_text("\"ap");
    assert_eq!(e.pending.text(), ":é");
}

#[test]
fn the_system_clipboard_lands_in_the_field_that_asked_or_nowhere() {
    let ticket = |e: &mut Editor| strop_core::worker::Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: crate::editor::ClipboardKey {
            document: e.current(),
        },
    };
    let deliver = |e: &mut Editor, ticket| {
        e.handle_clipboard(strop_core::worker::Completion {
            ticket,
            outcome: strop_core::worker::Outcome::Success("clip".into()),
        })
    };
    let mut e = Editor::new(Buffer::from_text("doc\n"));
    e.feed_text(":");
    let read = ticket(&mut e);
    e.clip_paste_pending = Some((PasteDestination::Prompt, read.clone()));
    deliver(&mut e, read);
    assert_eq!(e.pending.text(), ":clip");
    e.feed(Key::Esc);
    e.feed(Key::Esc);
    // the prompt closed while the provider ran: the text goes nowhere
    let read = ticket(&mut e);
    e.clip_paste_pending = Some((PasteDestination::Prompt, read.clone()));
    deliver(&mut e, read);
    assert_eq!(e.buf().text(), "doc\n");
    assert!(e.message.contains("destination changed"), "{}", e.message);
    e.open_picker(Kind::Files);
    let read = ticket(&mut e);
    e.clip_paste_pending = Some((PasteDestination::Picker, read.clone()));
    deliver(&mut e, read);
    assert_eq!(query(&e), "clip");
}
