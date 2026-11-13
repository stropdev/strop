use super::*;
pub(super) fn fixture() -> (tempfile::TempDir, Editor) {
    let root = tempfile::Builder::new()
        .tempdir_in(std::fs::canonicalize(std::env::temp_dir()).unwrap())
        .unwrap();
    let mut editor = Editor::new_in(strop_core::Buffer::from_text(""), root.path().to_owned());
    editor.filesystem.environment = strop_fs::Environment {
        home: Some(root.path().join("home")),
        data_home: Some(root.path().join("data")),
    };
    (root, editor)
}
pub(super) fn command(editor: &mut Editor, text: &str) {
    editor.feed_text(text);
    editor.wait_io().unwrap();
}
/// Open `folder` the way `Space e` does (a dirty buffer stays open
/// behind it) and put the cursor on `name`'s row.
pub(super) fn folder_at(editor: &mut Editor, folder: &std::path::Path, name: &str) {
    if editor
        .directory()
        .is_none_or(|source| source.location.path != folder)
    {
        editor
            .browse_directory(&folder.display().to_string())
            .unwrap();
        editor.wait_io().unwrap();
    }
    let line = editor
        .directory()
        .unwrap()
        .line_for(&ResourceLocation::local(folder.join(name)))
        .unwrap_or_else(|| panic!("{name} is not listed; message: {:?}", editor.message));
    editor.set_head(editor.buf().line_start(line.get()));
}
/// Rename `from` to `to` in `folder` and `:w`: the confirm buffer opens
/// with the prepared batch, not yet applied.
pub(super) fn rename(editor: &mut Editor, folder: &std::path::Path, from: &str, to: &str) {
    folder_at(editor, folder, from);
    editor.feed_text(&format!("cc{to}<esc>"));
    command(editor, ":w<cr>");
    assert!(
        editor.filesystem.pending.is_some(),
        "no confirm buffer; message: {:?}",
        editor.message
    );
}

#[test]
fn pending_save_as_blocks_a_move_from_an_unrelated_document() {
    let (root, mut editor) = fixture();
    let source = root.path().join("source.txt");
    let writer = root.path().join("writer.txt");
    let destination = root.path().join("destination.txt");
    std::fs::write(&source, "source\n").unwrap();
    std::fs::write(&writer, "writer\n").unwrap();
    let source_doc = editor.open_fixture(&source).unwrap();
    let writer_doc = editor.open_fixture(&writer).unwrap();
    assert!(editor.request_save_document(writer_doc, Some(destination.clone()), true, false));
    editor.switch_to(source_doc);
    let admitted = editor.filesystem_admission(
        &[OperationIntent {
            kind: OperationKind::Rename,
            source: Some(ResourceLocation::local(source.clone())),
            destination: Some(ResourceLocation::local(destination.clone())),
            copy_version: CopyVersion::Stored,
            expected_content: None,
            store: None,
        }],
        None,
    );
    editor.wait_io().unwrap();
    assert!(
        admitted.is_err(),
        "save-as target must reserve the mutation namespace"
    );
    assert_eq!(std::fs::read(source).unwrap(), b"source\n");
    assert_eq!(std::fs::read(destination).unwrap(), b"writer\n");
}

#[cfg(unix)]
#[test]
fn an_admitted_move_blocks_save_as_through_a_parent_alias() {
    let (root, mut editor) = fixture();
    let real = root.path().join("real");
    let alias = root.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    let source = root.path().join("source.txt");
    let writer = root.path().join("writer.txt");
    std::fs::write(&source, "source\n").unwrap();
    std::fs::write(&writer, "writer\n").unwrap();
    let writer_doc = editor.open_fixture(&writer).unwrap();
    rename(&mut editor, root.path(), "source.txt", "real/source.txt");
    assert!(editor.apply_filesystem_review());
    let admitted =
        editor.request_save_document(writer_doc, Some(alias.join("source.txt")), true, false);
    editor.wait_io().unwrap();
    assert!(
        !admitted,
        "an unresolved save-as alias must not bypass the filesystem reservation"
    );
    assert!(!source.exists());
    assert_eq!(std::fs::read(real.join("source.txt")).unwrap(), b"source\n");
}

#[test]
fn cleanup_failure_keeps_observed_receipts_and_their_binding_updates() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    let renamed = root.path().join("renamed.txt");
    std::fs::write(&original, "stored\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    editor.feed_text("ccdirty<esc>");
    rename(&mut editor, root.path(), "original.txt", "renamed.txt");
    assert!(editor.apply_filesystem_review());
    let event = editor
        .io
        .rx
        .as_ref()
        .unwrap()
        .recv_timeout(std::time::Duration::from_secs(30))
        .unwrap();
    let IoEvent::Filesystem(event) = event else {
        panic!("expected native filesystem completion");
    };
    let FsEvent::Applied(completion) = *event else {
        panic!("expected applied receipt");
    };
    let Completion { ticket, outcome } = *completion;
    let Outcome::Success(receipts) = outcome else {
        panic!("native rename must complete");
    };
    editor.handle_filesystem(FsEvent::Applied(Box::new(Completion {
        ticket,
        outcome: Outcome::Failed {
            failure: worker::Failure::new(FailureKind::Io, "injected cancellation cleanup failure"),
            partial: Some(receipts),
        },
    })));
    editor.wait_io().unwrap();
    editor.switch_to(document);
    command(&mut editor, ":w<cr>");
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(renamed).unwrap(), "dirty\n");
}

#[test]
fn a_folder_copy_lands_beside_its_source_without_changing_ex_open_scope() {
    let (root, mut editor) = fixture();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    std::fs::write(nested.join("source.txt"), "source\n").unwrap();
    std::fs::write(root.path().join("root.txt"), "workspace\n").unwrap();
    folder_at(&mut editor, &nested, "source.txt");
    editor.feed_text("yypccsibling.txt<esc>");
    command(&mut editor, ":w<cr>");
    command(&mut editor, "y");
    assert_eq!(
        std::fs::read(nested.join("sibling.txt"))
            .unwrap_or_else(|error| { panic!("{error}; message: {:?}", editor.message) }),
        b"source\n"
    );
    assert!(!root.path().join("sibling.txt").exists());
    command(&mut editor, ":e root.txt<cr>");
    assert_eq!(editor.buf().text().to_string(), "workspace\n");
    assert_eq!(editor.cwd, root.path());
}

#[test]
fn a_copy_of_a_file_with_unsaved_changes_is_refused() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("main.rs"), "fn main() {}\n").unwrap();
    editor.open_fixture(&root.path().join("main.rs")).unwrap();
    editor.feed_text("ccdirty<esc>");
    folder_at(&mut editor, root.path(), "main.rs");
    editor.feed_text("yypcccopy.rs<esc>");
    command(&mut editor, ":w<cr>");
    assert!(editor.filesystem.pending.is_none());
    assert_eq!(
        editor.message,
        "main.rs has unsaved changes; write it first"
    );
    assert!(!root.path().join("copy.rs").exists());
}

#[test]
fn the_cursor_stays_on_a_renamed_entry_after_resorting() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "selected\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "other\n").unwrap();
    rename(&mut editor, root.path(), "a.txt", "z.txt");
    command(&mut editor, "y");
    assert_eq!(editor.buf().text().to_string(), "b.txt\nz.txt\n");
    assert_eq!(editor.buf().line_of(editor.head()), 1);
    editor.feed(super::super::Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().file_identity(),
        Some(root.path().join("z.txt").as_path())
    );
    assert_eq!(editor.buf().text().to_string(), "selected\n");
}

#[test]
fn the_cursor_tracks_the_original_entry_across_name_reuse() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "selected\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "other\n").unwrap();
    rename(&mut editor, root.path(), "a.txt", "staging.txt");
    command(&mut editor, "y");
    // One write: b takes a's old name, and the cursor's entry takes b's.
    folder_at(&mut editor, root.path(), "b.txt");
    editor.feed_text("cca.txt<esc>");
    folder_at(&mut editor, root.path(), "staging.txt");
    editor.feed_text("ccb.txt<esc>");
    command(&mut editor, ":w<cr>");
    command(&mut editor, "y");
    editor.feed(super::super::Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().file_identity(),
        Some(root.path().join("b.txt").as_path()),
        "message: {:?}",
        editor.message
    );
    assert_eq!(editor.buf().text().to_string(), "selected\n");
}

#[test]
fn pending_format_then_save_cannot_be_retargeted_by_a_file_move() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    std::fs::write(&original, "source\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    editor.lsp_state.after_format = Some(crate::editor::lsp::state::AfterFormat::Save {
        document,
        close: false,
        force: false,
        request: strop_lsp::RequestStamp {
            request: strop_lsp::RequestId::new(1),
            server: strop_lsp::ServerId::new(1),
            document,
            revision: editor.buf().revision(),
        },
    });
    folder_at(&mut editor, root.path(), "original.txt");
    editor.feed_text("ccrenamed.txt<esc>");
    command(&mut editor, ":w<cr>");
    command(&mut editor, "y");
    assert_eq!(std::fs::read_to_string(&original).unwrap(), "source\n");
    assert!(!root.path().join("renamed.txt").exists());
}

#[test]
fn committed_rename_retires_completion_candidates_for_the_old_name() {
    use crate::editor::remote_completion::{
        CandidateSource, RemoteCandidate, RemoteCompletionKey, RemoteCompletionQuery,
        RemoteCompletionResult,
    };
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    let renamed = root.path().join("renamed.txt");
    std::fs::write(&original, "source\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    rename(&mut editor, root.path(), "original.txt", "renamed.txt");
    assert!(
        editor.apply_filesystem_review(),
        "message: {:?}",
        editor.message
    );
    editor.switch_to(document);
    editor.feed_text(":e orig");
    let prompt = editor.pending.text().to_string();
    let ticket = Ticket {
        request: editor.worker_ids.allocate().unwrap(),
        key: RemoteCompletionKey {
            focus: editor.focus_epoch,
            document,
            revision: editor.buf().revision(),
            text: prompt.clone(),
            cursor: prompt.len(),
            query: RemoteCompletionQuery::Directory {
                location: ResourceLocation::local(root.path().to_path_buf()),
                segment: b"orig".to_vec(),
                typed: None,
            },
            prefix_body: "e ".into(),
        },
    };
    editor.remote_completion.pending = Some(ticket.clone());
    editor.wait_io().unwrap();
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "source\n");
    editor.handle_remote_completion(Completion {
        ticket,
        outcome: Outcome::Success(RemoteCompletionResult::Candidates {
            items: vec![RemoteCandidate {
                uri: ResourceLocation::local(original).uri().unwrap(),
                directory: false,
            }],
            source: CandidateSource::Directory,
            notes: Vec::new(),
            listed_directory: None,
        }),
    });
    assert_eq!(editor.pending.text(), prompt);
}

#[test]
fn a_nested_new_name_creates_every_parent_on_write() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("keep.txt"), "").unwrap();
    folder_at(&mut editor, root.path(), "keep.txt");
    editor.feed_text("oone/two/file.txt<esc>");
    command(&mut editor, ":w<cr>");
    assert!(!root.path().join("one").exists());
    assert_eq!(
        editor
            .filesystem
            .pending
            .as_ref()
            .unwrap()
            .batch
            .steps
            .len(),
        3
    );
    command(&mut editor, "y");
    assert_eq!(
        std::fs::read_to_string(root.path().join("one/two/file.txt")).unwrap(),
        ""
    );
    assert_eq!(editor.buf().text().to_string(), "one/\nkeep.txt\n");
    assert_eq!(editor.message, "3 changes applied");
}

#[test]
fn dirty_file_rename_preserves_document_undo_and_writes_the_new_name() {
    let (root, mut editor) = fixture();
    let old = root.path().join("old.txt");
    let new = root.path().join("new.txt");
    std::fs::write(&old, "base\n").unwrap();
    editor.open_fixture(&old).unwrap();
    let document = editor.current();
    editor.feed_text("iunsaved <esc>");
    rename(&mut editor, root.path(), "old.txt", "new.txt");
    command(&mut editor, "y");
    editor.switch_to(document);
    assert!(editor.buf().is_dirty());
    assert_eq!(
        editor.buf().file_identity(),
        Some(new.as_path()),
        "message: {:?}",
        editor.message
    );
    command(&mut editor, ":w<cr>");
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "unsaved base\n");
    assert!(!editor.buf().is_dirty());
    editor.feed_text("u");
    assert_eq!(editor.buf().text(), "base\n");
}

#[test]
fn directory_move_keeps_dirty_descendants_and_their_document_ids() {
    let (root, mut editor) = fixture();
    let tree = root.path().join("tree");
    std::fs::create_dir_all(tree.join("sub")).unwrap();
    std::fs::write(tree.join("a.txt"), "A\n").unwrap();
    std::fs::write(tree.join("sub/b.txt"), "B\n").unwrap();
    editor.open_fixture(&tree.join("a.txt")).unwrap();
    let first = editor.current();
    editor.feed_text("ione <esc>");
    editor.open_fixture(&tree.join("sub/b.txt")).unwrap();
    let second = editor.current();
    editor.feed_text("itwo <esc>");
    let destination = root.path().join("moved");
    rename(&mut editor, root.path(), "tree", "moved/");
    command(&mut editor, "y");
    assert!(!tree.exists());
    for (document, suffix, text) in [
        (first, "a.txt", "one A\n"),
        (second, "sub/b.txt", "two B\n"),
    ] {
        let entry = editor.docs.get(document).unwrap();
        assert!(entry.buf.is_dirty());
        assert_eq!(
            entry.buf.file_identity(),
            Some(destination.join(suffix).as_path())
        );
        assert_eq!(entry.buf.text(), text);
        editor.switch_to(document);
        command(&mut editor, ":w<cr>");
        assert_eq!(
            std::fs::read_to_string(destination.join(suffix)).unwrap(),
            text
        );
    }
}

#[test]
fn an_open_folder_that_moves_lists_again_at_its_new_path() {
    let (root, mut editor) = fixture();
    std::fs::create_dir(root.path().join("child")).unwrap();
    std::fs::write(root.path().join("child/a.txt"), "").unwrap();
    std::fs::write(root.path().join("child/b.txt"), "").unwrap();
    folder_at(&mut editor, &root.path().join("child"), "a.txt");
    let child = editor.current();
    rename(&mut editor, root.path(), "child", "moved/");
    command(&mut editor, "y");
    let moved = editor.doc(child).directory_metadata_ref().unwrap();
    assert_eq!(moved.location.path, root.path().join("moved"));
    assert_eq!(editor.doc(child).buf.text().to_string(), "a.txt\nb.txt\n");
    assert!(!editor.doc(child).buf.readonly);
}

#[test]
fn another_folder_with_pending_edits_holds_its_paths() {
    let (root, mut editor) = fixture();
    std::fs::create_dir(root.path().join("child")).unwrap();
    std::fs::write(root.path().join("child/a.txt"), "").unwrap();
    folder_at(&mut editor, &root.path().join("child"), "a.txt");
    editor.feed_text("ccb.txt<esc>");
    folder_at(&mut editor, root.path(), "child");
    editor.feed_text("ccmoved/<esc>");
    command(&mut editor, ":w<cr>");
    command(&mut editor, "y");
    assert!(
        editor
            .message
            .ends_with("has pending edits; write (:w) or drop (:e!) them first"),
        "{}",
        editor.message
    );
    assert!(root.path().join("child/a.txt").exists());
}
