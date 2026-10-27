use super::super::*;
use strop_core::Buffer;

#[test]
fn yanked_register_inserts_into_picker_and_prompt_fields() {
    for kind in [Kind::Files, Kind::Search] {
        let mut e = Editor::new(Buffer::from_text("Needle\nneedle\n"));
        e.feed_text("yiw");
        e.open_picker(kind);
        if kind == Kind::Search {
            e.feed(strop_core::frontend_input::Key::CtrlR);
            assert!(e.picker.as_ref().unwrap().picker.replacement_visible);
            e.feed(strop_core::frontend_input::Key::CtrlR);
            assert!(!e.picker.as_ref().unwrap().picker.replacement_visible);
            e.feed(strop_core::frontend_input::Key::Esc);
            e.feed_text("p");
        } else {
            e.feed(strop_core::frontend_input::Key::CtrlR);
            e.feed_text("\"");
        }
        assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "Needle");
        assert_eq!(e.buf().text(), "Needle\nneedle\n");
        if kind == Kind::Search {
            continue;
        }
        e.feed(strop_core::frontend_input::Key::Esc);
        e.feed_text("0P");
        assert_eq!(
            e.picker.as_ref().unwrap().picker.input.text(),
            "NeedleNeedle"
        );
    }
    let mut e = Editor::new(Buffer::from_text("Needle\nneedle\n"));
    e.feed_text("yiw/");
    e.feed(strop_core::frontend_input::Key::CtrlR);
    e.feed_text("\"");
    assert_eq!(e.pending.text(), "/Needle");
    e.feed_text("<cr>");
    assert_eq!(e.head(), 7);
    e.set_register(Some('a'), crate::editor::Register::characterwise("é"));
    e.open_picker(Kind::Files);
    e.feed(strop_core::frontend_input::Key::CtrlR);
    e.feed_text("a");
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "é");
}

#[test]
fn multiline_register_refuses_without_truncating_picker_input() {
    let mut e = Editor::new(Buffer::from_text("first\nsecond\n"));
    e.feed_text("yj");
    e.open_picker(Kind::Files);
    e.feed_text("keep");
    e.feed(strop_core::frontend_input::Key::CtrlR);
    e.feed_text("\"");
    assert_eq!(e.picker.as_ref().unwrap().picker.input.text(), "keep");
    assert!(e.message.contains("newline"));
}
