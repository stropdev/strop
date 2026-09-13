use super::*;
use crate::editor::filesystem::tests::{command, fixture};
fn open_draft(editor: &mut Editor) -> DocumentId {
    command(editor, ":browse<cr>");
    let document = editor.current();
    command(editor, ":fs edit<cr>");
    assert!(
        editor.filename_draft(document).is_some_and(Draft::editable),
        "{}",
        editor.message
    );
    assert!(!editor.buf().readonly);
    document
}

#[test]
fn modal_names_review_cancel_and_apply_one_shared_filesystem_plan() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "A\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "B\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("ccrenamed.txt<esc>jddoadded.txt<esc>");
    assert!(
        editor.filename_draft(document).unwrap().error().is_none(),
        "{:?}",
        editor.filename_draft(document)
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "A\n"
    );
    assert!(root.path().join("b.txt").exists());
    assert!(!root.path().join("added.txt").exists());
    let draft_text = editor.buf().text().to_string();
    let draft_head = editor.head();
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_some(), "{}", editor.message);
    command(&mut editor, ":cancel-change<cr>");
    assert_eq!(editor.current(), document);
    assert!(editor.filename_draft(document).is_some_and(Draft::editable));
    assert_eq!(editor.buf().text().to_string(), draft_text);
    assert_eq!(editor.head(), draft_head);
    command(&mut editor, ":w<cr>");
    command(&mut editor, ":apply-change<cr>");
    assert!(!root.path().join("a.txt").exists());
    assert!(!root.path().join("b.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("renamed.txt")).unwrap(),
        "A\n"
    );
    assert_eq!(std::fs::read(root.path().join("added.txt")).unwrap(), b"");
    assert!(editor.filename_draft(document).is_none());
    assert!(editor.doc(document).buf.readonly);
}

#[test]
fn browsing_an_open_filename_draft_keeps_its_text_history_and_provenance() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "A\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("ccrenamed.txt<esc>");
    let text = editor.buf().text().to_string();
    command(
        &mut editor,
        &format!(":browse {}<cr>", root.path().display()),
    );
    assert_eq!(editor.current(), document);
    assert_eq!(editor.buf().text().to_string(), text);
    assert!(editor.buf().dirty);
    assert!(editor.filename_draft(document).is_some_and(Draft::editable));
    editor.feed(crate::editor::Key::Char('u'));
    assert_eq!(editor.buf().text().to_string(), "a.txt\n");
    editor.feed(crate::editor::Key::CtrlR);
    assert_eq!(editor.buf().text().to_string(), text);
    command(&mut editor, ":w<cr>");
    command(&mut editor, ":apply-change<cr>");
    assert!(!root.path().join("a.txt").exists());
    assert_eq!(
        std::fs::read(root.path().join("renamed.txt")).unwrap(),
        b"A\n"
    );
}

#[test]
fn whole_entry_yank_paste_keeps_copy_provenance_without_text_inference() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "source data\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("yypcccopy.txt<esc>");
    assert!(
        editor.filename_draft(document).unwrap().error().is_none(),
        "{:?}",
        editor.filename_draft(document)
    );
    command(&mut editor, ":w<cr>");
    assert_eq!(
        editor.filesystem.pending.as_ref().unwrap().batch.steps[0]
            .intent
            .kind,
        OperationKind::Copy
    );
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(
        std::fs::read_to_string(root.path().join("copy.txt")).unwrap(),
        "source data\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "source data\n"
    );
}

#[test]
fn delete_paste_and_history_restore_the_original_entry_identity() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "A\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "B\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("ddp");
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_none(), "{}", editor.message);
    assert_eq!(editor.current(), document);
    editor.feed_text("ccmoved.txt<esc>u<c-r>");
    command(&mut editor, ":w<cr>");
    let proposal = editor.filesystem.pending.as_ref().unwrap();
    assert_eq!(proposal.batch.steps.len(), 1);
    assert_eq!(proposal.batch.steps[0].intent.kind, OperationKind::Rename);
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(
        std::fs::read_to_string(root.path().join("moved.txt")).unwrap(),
        "A\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("b.txt")).unwrap(),
        "B\n"
    );
}

#[test]
fn ambiguous_join_refuses_then_undo_recovers_source_provenance() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "A\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "B\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("J");
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_none());
    assert!(editor.filename_draft(document).unwrap().error().is_some());
    editor.feed_text("u");
    assert!(editor.filename_draft(document).unwrap().error().is_none());
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_none());
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "A\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("b.txt")).unwrap(),
        "B\n"
    );
}

#[test]
fn refresh_keeps_edited_names_and_records_new_source_observations() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "A\n").unwrap();
    let document = open_draft(&mut editor);
    editor.feed_text("ccwanted.txt<esc>");
    let before = editor.buf().snapshot();
    std::fs::write(root.path().join("a.txt"), "external data\n").unwrap();
    command(&mut editor, ":fs refresh<cr>");
    assert_eq!(editor.buf().text(), &before);
    assert!(editor.filename_draft(document).unwrap().latest.is_some());
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_none());
    assert_eq!(editor.buf().text(), &before);
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "external data\n"
    );
}

#[test]
fn external_text_creates_a_name_not_a_copied_source() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "must not be copied\n").unwrap();
    open_draft(&mut editor);
    editor.paste_bracketed("new.txt\n");
    command(&mut editor, ":w<cr>");
    assert_eq!(
        editor.filesystem.pending.as_ref().unwrap().batch.steps[0]
            .intent
            .kind,
        OperationKind::CreateFile
    );
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(std::fs::read(root.path().join("new.txt")).unwrap(), b"");
    assert_eq!(
        std::fs::read_to_string(root.path().join("a.txt")).unwrap(),
        "must not be copied\n"
    );
}
