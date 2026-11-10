//! `Space a` is one list of typed owners, titled "actions" (0090 §3).
use super::{Editor, Entry};
use strop_core::Buffer;
use strop_picker::Kind;

fn titles(editor: &Editor) -> Vec<String> {
    editor
        .picker
        .as_ref()
        .expect("actions list")
        .picker
        .items
        .iter()
        .map(|item| item.text.clone())
        .collect()
}

fn code(title: &str) -> strop_lsp::ProtoAction {
    strop_lsp::ProtoAction {
        title: title.into(),
        edits: None,
        has_external_command: true,
    }
}

fn file_editor() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let file = dir.path().join("notes.txt");
    std::fs::write(&file, "one\n").unwrap();
    let mut editor = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    editor.open_fixture(&file).unwrap();
    (dir, editor)
}

#[test]
fn space_a_on_a_file_lists_its_actions_under_one_title() {
    let (_dir, mut editor) = file_editor();
    editor.feed_text(" a");
    let glue = editor.picker.as_ref().expect("Space a opens the list");
    assert_eq!(glue.picker.kind, Kind::Actions);
    assert_eq!(glue.picker.kind.title().trim(), "actions");
    let rows = titles(&editor);
    assert!(rows.iter().any(|row| row == "Rename"), "{rows:?}");
    assert!(
        rows.iter().any(|row| row == "Copy resource URI"),
        "{rows:?}"
    );
    assert!(
        !rows.iter().any(|row| row == "Refresh directory"),
        "directory rows need a directory: {rows:?}"
    );
}

#[test]
fn code_actions_join_the_open_list_first() {
    let (_dir, mut editor) = file_editor();
    let entries = editor
        .filesystem_entries()
        .into_iter()
        .map(Entry::Filesystem)
        .collect();
    editor.open_actions(entries, true, "actions");
    assert!(editor.picker.as_ref().unwrap().picker.streaming);
    editor.deliver_code_actions(
        vec![code("Import Thing"), code("Inline variable")],
        strop_lsp::PositionEncoding::Utf16,
    );
    let rows = titles(&editor);
    assert_eq!(&rows[..2], ["Import Thing", "Inline variable"], "{rows:?}");
    assert!(rows.iter().any(|row| row == "Rename"), "{rows:?}");
    assert!(!editor.picker.as_ref().unwrap().picker.streaming);
    // accepting a code row runs the code action, not a filesystem row
    editor.accept_action(1);
    assert!(editor.picker.is_none());
    assert!(
        editor.message().contains("external command"),
        "{}",
        editor.message()
    );
}

#[test]
fn an_empty_answer_to_an_empty_list_says_so_and_a_late_one_is_dropped() {
    let (_dir, mut editor) = file_editor();
    editor.open_actions(Vec::new(), true, "actions");
    editor.deliver_code_actions(Vec::new(), strop_lsp::PositionEncoding::Utf16);
    assert!(editor.picker.is_none());
    assert_eq!(editor.message(), "no actions here");
    // a reply after the user closed the list never reopens it
    editor.deliver_code_actions(vec![code("Late")], strop_lsp::PositionEncoding::Utf16);
    assert!(editor.picker.is_none());
}

#[test]
fn a_filesystem_row_refuses_once_the_view_moved() {
    let (_dir, mut editor) = file_editor();
    let path = super::ACTIONS
        .iter()
        .position(|action| action.command == "path")
        .unwrap();
    editor.open_actions(vec![Entry::Filesystem(path)], false, "actions");
    editor.buf_mut().insert(0, "x");
    editor.accept_action(0);
    assert!(
        editor.message().contains("view changed"),
        "{}",
        editor.message()
    );
}
