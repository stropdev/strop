use super::*;
use strop_workspace::Filesystem;
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
    let admitted = editor.prepare_filesystem(
        vec![OperationIntent {
            kind: OperationKind::Rename,
            source: Some(ResourceLocation::local(source.clone())),
            destination: Some(ResourceLocation::local(destination.clone())),
            copy_version: CopyVersion::Stored,
            expected_content: None,
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
    editor.open_fixture(&source).unwrap();
    command(&mut editor, &format!(":fs move {}<cr>", real.display()));
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
fn reverse_move_recovery_refuses_a_changed_recorded_version() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    let target = root.path().join("target.txt");
    std::fs::write(&original, "original\n").unwrap();
    editor.open_fixture(&original).unwrap();
    command(&mut editor, ":fs rename target.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    let operation = editor.filesystem.history.back().unwrap().ticket.request;
    std::fs::write(&target, "later contents must remain at this name\n").unwrap();
    editor.recover_filesystem_step(operation, 0).unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":apply-change<cr>");
    assert!(!original.exists());
    assert_eq!(
        std::fs::read_to_string(target).unwrap(),
        "later contents must remain at this name\n"
    );
}

#[test]
fn cleanup_failure_keeps_observed_receipts_and_their_binding_updates() {
    let (root, mut editor) = fixture();
    let original = root.path().join("original.txt");
    let renamed = root.path().join("renamed.txt");
    std::fs::write(&original, "stored\n").unwrap();
    let document = editor.open_fixture(&original).unwrap();
    editor.feed_text("ccdirty<esc>");
    command(&mut editor, ":fs rename renamed.txt<cr>");
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
    editor.switch_to(document);
    command(&mut editor, ":w<cr>");
    assert!(!original.exists());
    assert_eq!(std::fs::read_to_string(renamed).unwrap(), "dirty\n");
}

#[test]
fn filesystem_destinations_use_source_parent_without_changing_ex_open_scope() {
    let (root, mut editor) = fixture();
    let nested = root.path().join("nested");
    std::fs::create_dir(&nested).unwrap();
    let source = nested.join("source.txt");
    std::fs::write(&source, "source\n").unwrap();
    std::fs::write(root.path().join("root.txt"), "workspace\n").unwrap();
    let document = editor.open_fixture(&source).unwrap();
    command(&mut editor, ":fs copy stored sibling.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(
        std::fs::read(nested.join("sibling.txt")).unwrap(),
        b"source\n"
    );
    assert!(!root.path().join("sibling.txt").exists());
    editor.switch_to(document);
    command(&mut editor, ":e root.txt<cr>");
    assert_eq!(editor.buf().text().to_string(), "workspace\n");
    assert_eq!(editor.cwd, root.path());
}

#[test]
fn directory_refresh_keeps_the_renamed_entry_selected_after_resorting() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "selected\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "other\n").unwrap();
    command(&mut editor, ":browse<cr>");
    command(&mut editor, ":fs rename z.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    command(
        &mut editor,
        &format!(":browse {}<cr>", root.path().display()),
    );
    command(&mut editor, ":fs refresh<cr>");
    editor.feed(super::super::Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().file_identity(),
        Some(root.path().join("z.txt").as_path())
    );
    assert_eq!(editor.buf().text().to_string(), "selected\n");
}

#[test]
fn directory_restoration_tracks_the_original_entry_across_name_reuse() {
    let (root, mut editor) = fixture();
    std::fs::write(root.path().join("a.txt"), "selected\n").unwrap();
    std::fs::write(root.path().join("b.txt"), "other\n").unwrap();
    command(&mut editor, ":browse<cr>");
    command(&mut editor, ":fs rename staging.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    for (from, to) in [("b.txt", "a.txt"), ("staging.txt", "b.txt")] {
        let uri = ResourceLocation::local(root.path().join(from))
            .uri()
            .unwrap();
        command(&mut editor, &format!(":e {uri}<cr>"));
        command(&mut editor, &format!(":fs rename {to}<cr>"));
        command(&mut editor, ":apply-change<cr>");
    }
    command(
        &mut editor,
        &format!(":browse {}<cr>", root.path().display()),
    );
    command(&mut editor, ":fs refresh<cr>");
    editor.feed(super::super::Key::Enter);
    editor.wait_io().unwrap();
    assert_eq!(
        editor.buf().file_identity(),
        Some(root.path().join("b.txt").as_path())
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
    command(&mut editor, ":fs rename renamed.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
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
    command(&mut editor, ":fs rename renamed.txt<cr>");
    assert!(editor.apply_filesystem_review());
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
                container: None,
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
fn exclusive_creation_reviews_every_parent_then_opens_the_committed_file() {
    let (root, mut editor) = fixture();
    command(&mut editor, ":fs create one/two/file.txt<cr>");
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
    command(&mut editor, ":apply-change<cr>");
    let target = root.path().join("one/two/file.txt");
    assert_eq!(editor.buf().file_identity(), Some(target.as_path()));
    command(&mut editor, "icontent<esc>:w<cr>");
    assert_eq!(std::fs::read_to_string(target).unwrap(), "content");
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
    command(&mut editor, ":fs rename new.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    editor.feed_text("q");
    assert_eq!(editor.current(), document);
    assert!(editor.buf().dirty);
    assert_eq!(editor.buf().file_identity(), Some(new.as_path()));
    command(&mut editor, ":w<cr>");
    assert!(!old.exists());
    assert_eq!(std::fs::read_to_string(&new).unwrap(), "unsaved base\n");
    assert!(!editor.buf().dirty);
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
    editor
        .prepare_filesystem(
            vec![OperationIntent {
                kind: OperationKind::Rename,
                source: Some(ResourceLocation::local(tree.clone())),
                destination: Some(ResourceLocation::local(destination.clone())),
                copy_version: CopyVersion::Stored,
                expected_content: None,
            }],
            None,
        )
        .unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":apply-change<cr>");
    assert!(!tree.exists());
    for (document, suffix, text) in [
        (first, "a.txt", "one A\n"),
        (second, "sub/b.txt", "two B\n"),
    ] {
        let entry = editor.docs.get(document).unwrap();
        assert!(entry.buf.dirty);
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
fn closing_the_review_cannot_drop_an_admitted_mutation_receipt() {
    let (root, mut editor) = fixture();
    let old = root.path().join("old.txt");
    std::fs::write(&old, "kept\n").unwrap();
    editor.open_fixture(&old).unwrap();
    let source = editor.current();
    command(&mut editor, ":fs rename new.txt<cr>");
    let review = editor.current();
    assert!(editor.apply_filesystem_review());
    assert!(editor.close_buffer(false));
    assert!(editor.docs.get(review).is_none());
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), source);
    assert_eq!(
        editor.buf().file_identity(),
        Some(root.path().join("new.txt").as_path())
    );
    assert!(editor.filesystem.history.back().unwrap().receipts[0]
        .outcome
        .is_committed());
}

#[test]
fn permanent_removal_detaches_dirty_text_and_cannot_recreate_the_old_binding() {
    let (root, mut editor) = fixture();
    let path = root.path().join("removed.txt");
    std::fs::write(&path, "kept in memory\n").unwrap();
    editor.open_fixture(&path).unwrap();
    let document = editor.current();
    editor.feed_text("iunsaved <esc>");
    command(&mut editor, ":fs remove<cr>");
    command(&mut editor, ":apply-change<cr>");
    editor.feed_text("q");
    assert_eq!(editor.current(), document);
    assert!(!path.exists());
    assert_eq!(editor.buf().text(), "unsaved kept in memory\n");
    assert!(editor.buf().dirty);
    assert!(editor.buf().path.is_none());
    command(&mut editor, ":w!<cr>");
    assert!(!path.exists());
    assert!(editor.buf().dirty);
    assert_eq!(
        editor
            .doc(document)
            .file_target(root.path())
            .and_then(|target| target.resource_location())
            .map(|location| location.filesystem),
        None::<Filesystem>
    );
}

#[test]
fn mixed_copy_versions_use_the_reviewed_snapshot_for_each_destination() {
    let (root, mut editor) = fixture();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "stored\n").unwrap();
    editor.open_fixture(&source).unwrap();
    editor.feed_text("iunsaved <esc>");
    let make = |version, name: &str| OperationIntent {
        kind: OperationKind::Copy,
        source: Some(ResourceLocation::local(source.clone())),
        destination: Some(ResourceLocation::local(root.path().join(name))),
        copy_version: version,
        expected_content: None,
    };
    editor
        .prepare_filesystem(
            vec![
                make(CopyVersion::Stored, "stored.txt"),
                make(CopyVersion::Buffer, "buffer.txt"),
            ],
            None,
        )
        .unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(
        std::fs::read_to_string(root.path().join("stored.txt")).unwrap(),
        "stored\n"
    );
    assert_eq!(
        std::fs::read_to_string(root.path().join("buffer.txt")).unwrap_or_else(|error| {
            panic!(
                "{error}; receipts: {:#?}",
                editor
                    .filesystem
                    .history
                    .back()
                    .map(|attempt| &attempt.receipts)
            )
        }),
        "unsaved stored\n"
    );
    assert_eq!(std::fs::read_to_string(source).unwrap(), "stored\n");
}

#[test]
fn copy_recovery_cannot_remove_contents_changed_after_publication() {
    let (root, mut editor) = fixture();
    let source = root.path().join("source.txt");
    let copy = root.path().join("copy.txt");
    std::fs::write(&source, "before\n").unwrap();
    editor.open_fixture(&source).unwrap();
    command(&mut editor, ":fs copy copy.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    let operation = editor.filesystem.history.back().unwrap().ticket.request;
    std::fs::write(&copy, "after!\n").unwrap();
    editor.recover_filesystem_step(operation, 0).unwrap();
    editor.wait_io().unwrap();
    assert!(editor
        .filesystem
        .pending
        .as_ref()
        .unwrap()
        .batch
        .steps
        .is_empty());
    assert_eq!(std::fs::read_to_string(copy).unwrap(), "after!\n");
}

#[test]
fn native_trash_restore_cleans_metadata_and_preserves_the_detached_draft() {
    let (root, mut editor) = fixture();
    let source = root.path().join("original.txt");
    std::fs::write(&source, "stored\n").unwrap();
    editor.open_fixture(&source).unwrap();
    let document = editor.current();
    editor.feed_text("iunsaved <esc>");
    command(&mut editor, ":fs trash<cr>");
    command(&mut editor, ":apply-change<cr>");
    let operation = editor.filesystem.history.back().unwrap().ticket.request;
    assert!(!source.exists());
    editor.recover_filesystem_step(operation, 0).unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":apply-change<cr>");
    assert_eq!(std::fs::read_to_string(source).unwrap(), "stored\n");
    assert_eq!(editor.doc(document).buf.text(), "unsaved stored\n");
    assert!(editor.doc(document).buf.path.is_none());
    assert!(editor.doc(document).buf.dirty);
    #[cfg(target_os = "linux")]
    {
        assert_eq!(
            std::fs::read_dir(root.path().join("data/Trash/files"))
                .unwrap()
                .count(),
            0
        );
        assert_eq!(
            std::fs::read_dir(root.path().join("data/Trash/info"))
                .unwrap()
                .count(),
            0
        );
    }
}

#[test]
fn closing_an_unapplied_review_releases_only_its_preparation() {
    let (root, mut editor) = fixture();
    let source = root.path().join("source.txt");
    std::fs::write(&source, "kept\n").unwrap();
    editor.open_fixture(&source).unwrap();
    command(&mut editor, ":fs rename abandoned.txt<cr>");
    editor.feed_text("q");
    command(&mut editor, ":fs rename accepted.txt<cr>");
    command(&mut editor, ":apply-change<cr>");
    assert!(!root.path().join("abandoned.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("accepted.txt")).unwrap(),
        "kept\n"
    );
}
