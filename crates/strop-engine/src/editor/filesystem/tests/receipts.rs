//! Review, receipts and recovery: what a filesystem mutation leaves behind.
use super::*;

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
    command(&mut editor, ":fs apply<cr>");
    editor.feed_text("q");
    assert_eq!(editor.current(), document);
    assert!(!path.exists());
    assert_eq!(editor.buf().text(), "unsaved kept in memory\n");
    assert!(editor.buf().is_dirty());
    assert!(editor.buf().path.is_none());
    command(&mut editor, ":w!<cr>");
    assert!(!path.exists());
    assert!(editor.buf().is_dirty());
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
        store: None,
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
    command(&mut editor, ":fs apply<cr>");
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
    command(&mut editor, ":fs apply<cr>");
    let operation = editor
        .filesystem
        .history
        .back()
        .unwrap_or_else(|| panic!("no history after :fs apply; message: {:?}", editor.message))
        .ticket
        .request;
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
    command(&mut editor, ":fs apply<cr>");
    let operation = editor.filesystem.history.back().unwrap().ticket.request;
    assert!(!source.exists());
    editor.recover_filesystem_step(operation, 0).unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":fs apply<cr>");
    assert_eq!(std::fs::read_to_string(source).unwrap(), "stored\n");
    assert_eq!(editor.doc(document).buf.text(), "unsaved stored\n");
    assert!(editor.doc(document).buf.path.is_none());
    assert!(editor.doc(document).buf.is_dirty());
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
    command(&mut editor, ":fs apply<cr>");
    assert!(!root.path().join("abandoned.txt").exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("accepted.txt")).unwrap(),
        "kept\n"
    );
}

#[test]
fn committed_mutation_outcome_survives_focus_change() {
    let (root, mut editor) = fixture();
    let old = root.path().join("old.txt");
    let other = root.path().join("other.txt");
    std::fs::write(&old, "kept\n").unwrap();
    std::fs::write(&other, "other\n").unwrap();
    let source = editor.open_fixture(&old).unwrap();
    let elsewhere = editor.open_fixture(&other).unwrap();
    editor.switch_to(source);
    command(&mut editor, ":fs rename new.txt<cr>");
    let review = editor.current();
    assert!(editor.apply_filesystem_review());
    // Focus leaves both the report and the source before the worker's
    // receipt arrives; the admitted outcome must reconcile regardless.
    editor.switch_to(elsewhere);
    editor.wait_io().unwrap();
    assert_eq!(editor.current(), elsewhere);
    let attempt = editor.filesystem.history.back().unwrap();
    assert_eq!(attempt.report, review);
    assert!(attempt.receipts[0].outcome.is_committed());
    assert_eq!(
        editor.doc(source).buf.file_identity(),
        Some(root.path().join("new.txt").as_path())
    );
    assert!(!old.exists());
    assert_eq!(
        std::fs::read_to_string(root.path().join("new.txt")).unwrap(),
        "kept\n"
    );
}

#[cfg(unix)]
#[test]
fn committed_move_retires_alias_spelled_preview_state() {
    let (root, mut editor) = fixture();
    let real = root.path().join("real");
    let alias = root.path().join("alias");
    std::fs::create_dir(&real).unwrap();
    std::os::unix::fs::symlink(&real, &alias).unwrap();
    std::fs::write(real.join("victim.txt"), "cached\n").unwrap();
    // A picker preview cached under the ALIAS spelling: the same resource
    // incarnation at a pathname the resolved operation never names
    // (guard::resolve canonicalizes ancestors, never the final entry).
    let alias_victim = ResourceLocation::local(alias.join("victim.txt"));
    editor.previews.insert(
        alias_victim.clone(),
        crate::editor::picker::PreviewEntry {
            rope: ropey::Rope::from_str("cached\n"),
        },
    );
    let key = crate::editor::picker::PreviewKey {
        picker: crate::editor::picker::PickerId(editor.worker_ids.allocate().unwrap()),
        path: alias_victim.clone(),
    };
    editor
        .preview_loads
        .insert(alias_victim.clone(), worker::Load::Ready(key));
    let document = editor.open_fixture(&real.join("victim.txt")).unwrap();
    let renamed = root.path().join("renamed.txt");
    editor
        .prepare_filesystem(
            vec![OperationIntent {
                kind: OperationKind::Rename,
                source: Some(ResourceLocation::local(alias.join("victim.txt"))),
                destination: Some(ResourceLocation::local(renamed.clone())),
                copy_version: CopyVersion::Stored,
                expected_content: None,
                store: None,
            }],
            None,
        )
        .unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":fs apply<cr>");
    assert!(!real.join("victim.txt").exists());
    assert_eq!(std::fs::read_to_string(&renamed).unwrap(), "cached\n");
    assert_eq!(
        editor.doc(document).buf.file_identity(),
        Some(renamed.as_path())
    );
    assert!(
        !editor.previews.contains_key(&alias_victim),
        "an alias-spelled preview of the moved incarnation must retire"
    );
    assert!(
        !editor.preview_loads.contains_key(&alias_victim),
        "an alias-spelled preview load record must retire with its cache"
    );
}

#[test]
fn a_redelivered_applied_receipt_reconciles_once() {
    let (root, mut editor) = fixture();
    let old = root.path().join("old.txt");
    std::fs::write(&old, "kept\n").unwrap();
    let source = editor.open_fixture(&old).unwrap();
    command(&mut editor, ":fs rename new.txt<cr>");
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
    let redelivery = completion.clone();
    editor.handle_filesystem(FsEvent::Applied(completion));
    assert!(editor.filesystem.history.back().unwrap().receipts[0]
        .outcome
        .is_committed());
    // The same operation identity delivered again is a transport duplicate,
    // never a second reconciliation.
    editor.handle_filesystem(FsEvent::Applied(redelivery));
    assert_eq!(editor.filesystem.history.len(), 1);
    assert_eq!(editor.filesystem.history.back().unwrap().receipts.len(), 1);
    assert!(editor.filesystem.running.is_none());
    assert_eq!(
        editor.doc(source).buf.file_identity(),
        Some(root.path().join("new.txt").as_path())
    );
    assert!(!old.exists());
}

#[test]
fn an_unconfirmed_outcome_blocks_conflicting_authority_until_verified() {
    let (root, mut editor) = fixture();
    let old = root.path().join("old.txt");
    std::fs::write(&old, "kept\n").unwrap();
    let document = editor.open_fixture(&old).unwrap();
    command(&mut editor, ":fs rename new.txt<cr>");
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
    let operation = completion.ticket.request;
    let Outcome::Success(receipts) = completion.outcome else {
        panic!("native rename must complete");
    };
    // The mutation ran and published; its acknowledgment was lost in
    // transport. The receipt keeps the post-publication evidence — the
    // outcome is unconfirmed, never silently committed or rolled back.
    let receipts: Vec<StepReceipt> = receipts
        .into_iter()
        .map(|mut receipt| {
            if let StepOutcome::Committed {
                destination_after,
                recovery,
                publication,
                ..
            } = receipt.outcome
            {
                receipt.outcome = StepOutcome::Unconfirmed {
                    detail: "transport lost the receipt".into(),
                    observed_destination: destination_after,
                    recovery,
                    publication,
                };
            }
            receipt
        })
        .collect();
    editor.handle_filesystem(FsEvent::Applied(Box::new(Completion {
        ticket: completion.ticket,
        outcome: Outcome::Failed {
            failure: worker::Failure::new(FailureKind::Io, "transport lost the receipt"),
            partial: Some(receipts),
        },
    })));
    assert!(editor.filesystem.history.back().unwrap().receipts[0]
        .outcome
        .is_unconfirmed());
    assert!(editor.filesystem_shutdown_error().is_some());
    // Conflicting authority is refused while the outcome is pending.
    let blocked = editor.prepare_filesystem(
        vec![OperationIntent {
            kind: OperationKind::Rename,
            source: Some(ResourceLocation::local(root.path().join("new.txt"))),
            destination: Some(ResourceLocation::local(root.path().join("other.txt"))),
            copy_version: CopyVersion::Stored,
            expected_content: None,
            store: None,
        }],
        None,
    );
    assert!(blocked.is_err(), "{:?}", editor.message);
    assert!(
        !editor.request_save_document(document, None, false, false),
        "a save through the blocked binding must refuse"
    );
    assert!(!old.exists(), "no automatic rollback of the lost receipt");
    // Explicit verification settles the receipt against current names and
    // releases the block; the rename is never re-executed.
    editor.verify_filesystem_step(operation, 0).unwrap();
    editor.wait_io().unwrap();
    assert!(
        editor.filesystem.history.back().unwrap().receipts[0]
            .outcome
            .is_committed(),
        "outcome after verify: {:?}; message: {}",
        editor.filesystem.history.back().unwrap().receipts[0].outcome,
        editor.message
    );
    assert!(editor.filesystem_shutdown_error().is_none());
    assert_eq!(
        editor.doc(document).buf.file_identity(),
        Some(root.path().join("new.txt").as_path())
    );
    editor
        .prepare_filesystem(
            vec![OperationIntent {
                kind: OperationKind::Rename,
                source: Some(ResourceLocation::local(root.path().join("new.txt"))),
                destination: Some(ResourceLocation::local(root.path().join("other.txt"))),
                copy_version: CopyVersion::Stored,
                expected_content: None,
                store: None,
            }],
            None,
        )
        .unwrap();
    editor.wait_io().unwrap();
    command(&mut editor, ":fs cancel<cr>");
}
