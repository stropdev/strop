#![cfg(any(target_os = "linux", target_os = "macos"))]
use super::*;
use std::path::Path;
use strop_workspace::operation::*;
use strop_workspace::ResourceLocation;

pub(super) fn with_token<T>(work: impl FnOnce(strop_core::worker::CancelToken) -> T) -> T {
    let (send, receive) = std::sync::mpsc::channel();
    let (release, wait) = std::sync::mpsc::channel::<()>();
    let owner = strop_core::worker::spawn(
        "fs-test-token",
        |_| {},
        move |token| {
            send.send(token).unwrap();
            let _ = wait.recv();
            strop_core::worker::Outcome::Success(())
        },
    );
    let result = work(receive.recv().unwrap());
    drop(release);
    drop(owner);
    result
}
pub(super) fn environment(root: &Path) -> Environment {
    Environment {
        home: Some(root.join("home")),
        data_home: Some(root.join("data")),
    }
}
pub(super) fn intent(
    kind: OperationKind,
    source: Option<&Path>,
    destination: Option<&Path>,
) -> OperationIntent {
    OperationIntent {
        kind,
        source: source.map(|path| ResourceLocation::local(path.to_path_buf())),
        destination: destination.map(|path| ResourceLocation::local(path.to_path_buf())),
        copy_version: CopyVersion::Stored,
        expected_content: None,
    }
}
fn prepare(
    root: &Path,
    intent: &OperationIntent,
    token: &strop_core::worker::CancelToken,
) -> PreparedOperation {
    local::prepare(intent, false, &environment(root), token).unwrap()
}

#[test]
fn move_verification_requires_an_owned_after_version_not_just_inode_occupancy() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("source");
    let target = root.path().join("target");
    std::fs::write(&source, "owned\n").unwrap();
    with_token(|token| {
        let operation = prepare(
            root.path(),
            &intent(OperationKind::Rename, Some(&source), Some(&target)),
            &token,
        );
        let outcome = local::execute(&operation, None, &[], &token);
        let StepOutcome::Committed {
            destination_after,
            publication,
            ..
        } = outcome
        else {
            panic!("rename failed");
        };
        let mut receipt = StepReceipt {
            step: 0,
            operation,
            outcome: StepOutcome::Unconfirmed {
                detail: "lost acknowledgment".into(),
                observed_destination: destination_after,
                recovery: None,
                publication: None,
            },
        };
        assert!(matches!(
            local::verify(&receipt, &token).unwrap(),
            VerifiedOutcome::Unknown { .. }
        ));
        if let StepOutcome::Unconfirmed {
            publication: owned, ..
        } = &mut receipt.outcome
        {
            *owned = publication;
        }
        assert!(matches!(
            local::verify(&receipt, &token).unwrap(),
            VerifiedOutcome::Committed(_)
        ));
    });
}

#[test]
fn a_directory_emptied_through_checked_operations_can_be_removed() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let directory = root.path().join("directory");
        std::fs::create_dir(&directory).unwrap();
        let child = directory.join("child");
        for request in [
            intent(OperationKind::CreateFile, None, Some(&child)),
            intent(OperationKind::Remove, Some(&child), None),
            intent(OperationKind::Remove, Some(&directory), None),
        ] {
            let operation = prepare(root.path(), &request, &token);
            let outcome = local::execute(&operation, None, &[], &token);
            assert!(outcome.is_committed(), "{outcome:?}");
        }
        assert!(!directory.exists());
    });
}

#[test]
fn missing_parents_are_visible_steps_and_only_created_on_apply() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let target = root.path().join("one/two/new");
        let plan = batch::prepare(
            &[intent(OperationKind::CreateFile, None, Some(&target))],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(plan.refused.is_empty(), "{:?}", plan.refused);
        assert_eq!(
            plan.steps
                .iter()
                .map(|step| step.intent.kind)
                .collect::<Vec<_>>(),
            [
                OperationKind::CreateDirectory,
                OperationKind::CreateDirectory,
                OperationKind::CreateFile
            ]
        );
        assert!(!root.path().join("one").exists());
        let receipts = batch::execute(&plan, &Default::default(), &token);
        assert!(
            receipts
                .iter()
                .all(|receipt| receipt.outcome.is_committed()),
            "{receipts:?}"
        );
        assert_eq!(std::fs::read(target).unwrap(), b"");
    });
}

#[test]
fn rename_graph_orders_vacancies_and_refuses_cycles_before_mutation() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let a = root.path().join("a");
        let b = root.path().join("b");
        let c = root.path().join("c");
        std::fs::write(&a, "A").unwrap();
        std::fs::write(&b, "B").unwrap();
        let cycle = batch::prepare(
            &[
                intent(OperationKind::Rename, Some(&a), Some(&b)),
                intent(OperationKind::Rename, Some(&b), Some(&a)),
            ],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(cycle.steps.is_empty());
        assert_eq!(cycle.refused.len(), 2);
        assert_eq!(std::fs::read(&a).unwrap(), b"A");
        assert_eq!(std::fs::read(&b).unwrap(), b"B");
        let chain = batch::prepare(
            &[
                intent(OperationKind::Rename, Some(&a), Some(&b)),
                intent(OperationKind::Rename, Some(&b), Some(&c)),
            ],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(chain.refused.is_empty(), "{:?}", chain.refused);
        let receipts = batch::execute(&chain, &Default::default(), &token);
        assert!(
            receipts
                .iter()
                .all(|receipt| receipt.outcome.is_committed()),
            "{receipts:?}"
        );
        assert!(!a.exists());
        assert_eq!(std::fs::read(&b).unwrap(), b"A");
        assert_eq!(std::fs::read(&c).unwrap(), b"B");
    });
}

#[test]
fn exclusive_creation_and_destination_races_preserve_other_files() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let file = root.path().join("new.txt");
        let plan = prepare(
            root.path(),
            &intent(OperationKind::CreateFile, None, Some(&file)),
            &token,
        );
        std::fs::write(&file, "other actor\n").unwrap();
        assert!(matches!(
            local::execute(&plan, None, &[], &token),
            StepOutcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "other actor\n");
        let another = root.path().join("empty.txt");
        let plan = prepare(
            root.path(),
            &intent(OperationKind::CreateFile, None, Some(&another)),
            &token,
        );
        assert!(local::execute(&plan, None, &[], &token).is_committed());
        assert_eq!(std::fs::read(another).unwrap(), b"");
    });
}

#[test]
fn rename_directory_and_conflicts_use_no_replace_semantics() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let source = root.path().join("a");
        let target = root.path().join("b");
        std::fs::create_dir(&source).unwrap();
        std::fs::write(source.join("child"), "kept\n").unwrap();
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Rename, Some(&source), Some(&target)),
            &token,
        );
        assert!(local::execute(&plan, None, &[], &token).is_committed());
        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(target.join("child")).unwrap(),
            "kept\n"
        );
        let occupied = root.path().join("occupied");
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Rename, Some(&target), Some(&occupied)),
            &token,
        );
        std::fs::create_dir(&occupied).unwrap();
        assert!(matches!(
            local::execute(&plan, None, &[], &token),
            StepOutcome::Refused(_)
        ));
        assert_eq!(
            std::fs::read_to_string(target.join("child")).unwrap(),
            "kept\n"
        );
    });
}

#[test]
fn stored_and_buffer_copies_publish_exact_bytes_without_changing_source() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let source = root.path().join("source");
        let stored = root.path().join("stored");
        let live = root.path().join("live");
        std::fs::write(&source, "disk\n").unwrap();
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Copy, Some(&source), Some(&stored)),
            &token,
        );
        let outcome = local::execute(&plan, None, &[], &token);
        assert!(outcome.is_committed(), "{outcome:?}");
        let mut request = intent(OperationKind::Copy, Some(&source), Some(&live));
        request.copy_version = CopyVersion::Buffer;
        let plan = local::prepare(&request, false, &environment(root.path()), &token).unwrap();
        let outcome = local::execute(
            &plan,
            Some(&ropey::Rope::from_str("unsaved\n")),
            &[],
            &token,
        );
        assert!(outcome.is_committed(), "{outcome:?}");
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "disk\n");
        assert_eq!(std::fs::read_to_string(stored).unwrap(), "disk\n");
        assert_eq!(std::fs::read_to_string(live).unwrap(), "unsaved\n");
        assert!(std::fs::read_dir(root.path()).unwrap().all(|entry| !entry
            .unwrap()
            .file_name()
            .as_encoded_bytes()
            .starts_with(b".strop-fs-")));
    });
}

#[test]
fn permanent_remove_refuses_nonempty_directories_and_changed_sources() {
    let root = tempfile::tempdir().unwrap();
    with_token(|token| {
        let directory = root.path().join("dir");
        std::fs::create_dir(&directory).unwrap();
        std::fs::write(directory.join("child"), "safe\n").unwrap();
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Remove, Some(&directory), None),
            &token,
        );
        assert!(matches!(
            local::execute(&plan, None, &[], &token),
            StepOutcome::Refused(_)
        ));
        assert_eq!(
            std::fs::read_to_string(directory.join("child")).unwrap(),
            "safe\n"
        );
        let file = root.path().join("file");
        std::fs::write(&file, "before").unwrap();
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Remove, Some(&file), None),
            &token,
        );
        std::fs::write(&file, "after changed").unwrap();
        assert!(matches!(
            local::execute(&plan, None, &[], &token),
            StepOutcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(file).unwrap(), "after changed");
    });
}

#[test]
fn trash_is_recoverable_and_restore_refuses_occupied_names() {
    let root = tempfile::tempdir().unwrap();
    std::fs::create_dir(root.path().join("home")).unwrap();
    with_token(|token| {
        let source = root.path().join("a name.txt");
        std::fs::write(&source, "recover me\n").unwrap();
        let plan = prepare(
            root.path(),
            &intent(OperationKind::Trash, Some(&source), None),
            &token,
        );
        let outcome = local::execute(&plan, None, &[], &token);
        let StepOutcome::Committed {
            recovery: Some(recovery),
            ..
        } = outcome
        else {
            panic!("{outcome:?}")
        };
        assert!(!source.exists());
        assert_eq!(
            std::fs::read_to_string(&recovery.path).unwrap(),
            "recover me\n"
        );
        let restore = prepare(
            root.path(),
            &intent(OperationKind::Restore, Some(&recovery.path), Some(&source)),
            &token,
        );
        std::fs::write(&source, "new occupant\n").unwrap();
        assert!(matches!(
            local::execute(&restore, None, &[], &token),
            StepOutcome::Refused(_)
        ));
        assert_eq!(std::fs::read_to_string(&source).unwrap(), "new occupant\n");
        std::fs::remove_file(&source).unwrap();
        assert!(local::execute(&restore, None, &[], &token).is_committed());
        assert_eq!(std::fs::read_to_string(source).unwrap(), "recover me\n");
    });
}
