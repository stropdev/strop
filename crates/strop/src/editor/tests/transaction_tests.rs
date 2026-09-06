use super::*;

#[test]
fn ranged_ex_delete_maps_marks_through_the_gateway() {
    let mut editor = Editor::new(Buffer::from_text("remove\nTARGET\n"));
    editor.feed_text("jma:1d<cr>");
    assert_eq!(editor.buf().rope.to_string(), "TARGET\n");
    assert_eq!(
        editor.marks[&'a'].1, 0,
        "mark follows its text across ranged Ex deletion"
    );
}

#[test]
fn delayed_pipe_rejects_offsets_inside_new_multibyte_text() {
    let mut editor = Editor::new(Buffer::from_text("ab\n"));
    let origin = editor.current();
    editor.buf_mut().replace_all_system("éx\n");
    editor.handle_shell_result(ShellResult::Pipe {
        buffer: origin,
        start: 1,
        end: 2,
        original: "b".into(),
        output: "replacement".into(),
        ok: true,
        err: String::new(),
    });
    assert_eq!(editor.buf().rope.to_string(), "éx\n");
    assert!(
        !editor.message.is_empty(),
        "stale result has visible feedback"
    );
}

#[test]
fn accepted_pipe_maps_anchors_and_is_undoable() {
    let mut editor = Editor::new(Buffer::from_text("prefix TARGET\n"));
    editor.feed_text("wma");
    editor.handle_shell_result(ShellResult::Pipe {
        buffer: editor.current(),
        start: 0,
        end: 7,
        original: "prefix ".into(),
        output: "x ".into(),
        ok: true,
        err: String::new(),
    });
    assert_eq!(editor.buf().rope.to_string(), "x TARGET\n");
    assert_eq!(editor.marks[&'a'].1, 2);
    editor.feed_text("u");
    assert_eq!(editor.buf().rope.to_string(), "prefix TARGET\n");
}
