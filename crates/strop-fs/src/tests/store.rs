//! Protected Store, conditional baselines and receiptless reconciliation.
use super::*;

/// A Store intent as the editor's save path builds it (0058 WK09):
/// baseline-mtime evidence, the frozen content digest and the
/// conditional policy.
fn store_intent(
    destination: &Path,
    baseline: Option<strop_workspace::FileTime>,
    force: bool,
    expect_absent: bool,
    content: &[u8],
) -> OperationIntent {
    use sha2::Digest;
    OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(ResourceLocation::local(destination.to_path_buf())),
        copy_version: CopyVersion::Stored,
        expected_content: Some(sha2::Sha256::digest(content).into()),
        store: Some(StorePolicy {
            baseline,
            baseline_object: None,
            baseline_attributes: None,
            force,
            expect_absent,
            displayed: None,
        }),
    }
}

fn modified(path: &Path) -> Option<strop_workspace::FileTime> {
    observation::metadata(&std::fs::symlink_metadata(path).unwrap()).modified
}

/// The worker's real save path: batch prepare, then apply with the
/// frozen content stream.
fn store_apply(
    context: &ExecutionContext,
    root: &Path,
    intent: &OperationIntent,
    content: &str,
    token: &strop_core::worker::CancelToken,
) -> StepReceipt {
    let plan = batch::prepare(
        context,
        std::slice::from_ref(intent),
        &environment(root),
        token,
    )
    .unwrap();
    assert!(plan.refused.is_empty(), "{:?}", plan.refused);
    assert_eq!(plan.steps.len(), 1);
    let mut contents = std::collections::HashMap::new();
    contents.insert(0, ropey::Rope::from_str(content));
    let receipts = batch::execute(context, &plan, &contents, token);
    assert_eq!(receipts.len(), 1);
    receipts.into_iter().next().unwrap()
}

#[test]
fn store_commits_preserving_permissions_and_publishes_a_witness() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o640)).unwrap();
    }
    with_token(|context, token| {
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, modified(&file), false, false, b"after\n"),
            "after\n",
            &token,
        );
        let StepOutcome::Committed {
            destination_after: Some(ref after),
            publication: Some(witness),
            ..
        } = receipt.outcome
        else {
            panic!("store must commit: {receipt:?}")
        };
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "after\n");
        assert!(witness.content.is_some(), "the witness names the bytes");
        assert_eq!(
            after.identity,
            Some(witness.identity),
            "the committed observation is the published object"
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777,
                0o640,
                "permissions survive an atomic save"
            );
        }
        // The committed receipt re-verifies as committed.
        assert!(matches!(
            local::verify(&receipt, context, &token).unwrap(),
            VerifiedOutcome::Committed(_)
        ));
    });
}

#[test]
fn store_refuses_a_moved_baseline_and_force_overwrites() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    with_token(|context, token| {
        let baseline = modified(&file);
        // The baseline moved: another writer landed meanwhile.
        std::fs::write(&file, "theirs\n").unwrap();
        std::fs::File::options()
            .write(true)
            .open(&file)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(42)),
            )
            .unwrap();
        let plan = batch::prepare(
            context,
            &[store_intent(&file, baseline, false, false, b"mine\n")],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert_eq!(plan.refused[0].failure.kind, FsFailureKind::Conflict);
        assert!(
            plan.refused[0]
                .failure
                .detail
                .contains("file changed on disk"),
            "{:?}",
            plan.refused[0].failure
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "theirs\n");
        // :w! overwrites the moved baseline.
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, baseline, true, false, b"mine\n"),
            "mine\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "mine\n");
    });
}

#[test]
fn protected_store_refuses_same_size_external_edits_even_when_mtime_is_restored() {
    use sha2::Digest as _;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"aaaa\n").unwrap();
    let before = std::fs::metadata(&file).unwrap().modified().unwrap();
    let baseline = modified(&file);
    let mut intent = store_intent(&file, baseline, false, false, b"cccc\n");
    intent.store.as_mut().unwrap().displayed = Some(sha2::Sha256::digest(b"aaaa\n").into());
    let change_preserving_time = || {
        std::fs::write(&file, b"bbbb\n").unwrap();
        std::fs::File::open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(before))
            .unwrap();
    };
    with_token(|context, token| {
        change_preserving_time();
        let plan = batch::prepare(
            context,
            &[intent.clone()],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.refused[0].failure.kind, FsFailureKind::Conflict);
        assert_eq!(std::fs::read(&file).unwrap(), b"bbbb\n");

        std::fs::write(&file, b"aaaa\n").unwrap();
        std::fs::File::open(&file)
            .unwrap()
            .set_times(std::fs::FileTimes::new().set_modified(before))
            .unwrap();
        let plan = batch::prepare(context, &[intent], &environment(root.path()), &token).unwrap();
        assert_eq!(plan.steps.len(), 1);
        change_preserving_time();
        let outcome = local::execute(
            &plan.steps[0],
            Some(&ropey::Rope::from_str("cccc\n")),
            &[],
            context,
            &token,
        );
        assert!(matches!(
            outcome,
            StepOutcome::Refused(FsFailure {
                kind: FsFailureKind::Conflict,
                ..
            })
        ));
        assert_eq!(std::fs::read(&file).unwrap(), b"bbbb\n");
    });
}

#[test]
fn protected_store_refuses_replaced_inode_with_identical_content_and_mtime() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    with_token(|context, token| {
        let observed = observation::observe(&file, true, true, &token)
            .unwrap()
            .unwrap();
        let mut intent = store_intent(&file, observed.modified, false, false, b"after\n");
        let policy = intent.store.as_mut().unwrap();
        policy.baseline_object = observed.identity;
        policy.baseline_attributes = observed.attributes;
        policy.displayed = observed.digest;

        let replacement = root.path().join("replacement");
        std::fs::write(&replacement, b"before\n").unwrap();
        std::fs::File::open(&replacement)
            .unwrap()
            .set_times(
                std::fs::FileTimes::new()
                    .set_modified(std::fs::metadata(&file).unwrap().modified().unwrap()),
            )
            .unwrap();
        std::fs::rename(&replacement, &file).unwrap();
        let plan = batch::prepare(context, &[intent], &environment(root.path()), &token).unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.refused[0].failure.kind, FsFailureKind::Conflict);
        assert_eq!(std::fs::read(&file).unwrap(), b"before\n");
    });
}

#[test]
fn store_save_as_refuses_an_occupied_name_unless_forced() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("target.txt");
    std::fs::write(&file, "occupied\n").unwrap();
    with_token(|context, token| {
        let plan = batch::prepare(
            context,
            &[store_intent(&file, None, false, true, b"new\n")],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.refused.len(), 1);
        assert!(
            plan.refused[0].failure.detail.contains("file exists"),
            "{:?}",
            plan.refused[0].failure
        );
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "occupied\n");
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, None, true, true, b"new\n"),
            "new\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "new\n");
    });
}

#[test]
fn store_same_size_change_verifies_after_a_lost_receipt() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "aaaa\n").unwrap();
    with_token(|context, token| {
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, modified(&file), false, false, b"bbbb\n"),
            "bbbb\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "bbbb\n");
        // The receipt was lost before its witness arrived: verification
        // still proves the intended same-size bytes landed, by digest.
        let lost = StepReceipt {
            step: receipt.step,
            operation: receipt.operation.clone(),
            outcome: StepOutcome::Unconfirmed {
                detail: "lost acknowledgment".into(),
                observed_destination: None,
                recovery: None,
                publication: None,
            },
        };
        assert!(matches!(
            local::verify(&lost, context, &token).unwrap(),
            VerifiedOutcome::Committed(_)
        ));
        use std::os::unix::fs::PermissionsExt as _;
        let original = std::fs::metadata(&file).unwrap().permissions().mode() & 0o7777;
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(original ^ 0o100)).unwrap();
        assert!(matches!(
            local::verify(&lost, context, &token).unwrap(),
            VerifiedOutcome::Unknown { .. }
        ));
        std::fs::set_permissions(&file, std::fs::Permissions::from_mode(original)).unwrap();
        // A foreign state is never "committed".
        std::fs::write(&file, "cccc\n").unwrap();
        assert!(matches!(
            local::verify(&lost, context, &token).unwrap(),
            VerifiedOutcome::Unknown { .. }
        ));
    });
}

#[cfg(target_os = "linux")]
#[test]
fn receiptless_store_recovery_refuses_changed_extended_attributes() {
    use std::ffi::OsStr;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    let name = OsStr::new("user.strop.proof");
    let set_attribute = |value: &[u8]| {
        let fd = std::fs::File::options().write(true).open(&file).unwrap();
        rustix::fs::fsetxattr(&fd, name, value, rustix::fs::XattrFlags::empty()).unwrap();
    };
    set_attribute(b"original");
    with_token(|context, token| {
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, modified(&file), false, false, b"after\n"),
            "after\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        let lost = StepReceipt {
            step: receipt.step,
            operation: receipt.operation,
            outcome: StepOutcome::Unconfirmed {
                detail: "reply lost".into(),
                observed_destination: None,
                recovery: None,
                publication: None,
            },
        };
        assert!(matches!(
            local::verify(&lost, context, &token).unwrap(),
            VerifiedOutcome::Committed(_)
        ));
        set_attribute(b"foreign");
        assert!(matches!(
            local::verify(&lost, context, &token).unwrap(),
            VerifiedOutcome::Unknown { .. }
        ));
        assert_eq!(std::fs::read(&file).unwrap(), b"after\n");
    });
}

#[test]
fn recovered_store_can_be_observed_but_old_write_authority_stays_revoked() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    with_token(|original, token| {
        let receipt = store_apply(
            original,
            root.path(),
            &store_intent(&file, modified(&file), false, false, b"after\n"),
            "after\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        let recovered = ExecutionContext::native();
        assert!(local::verify(&receipt, &recovered, &token).is_err());
        let observed = batch::verify_recovered(&recovered, &receipt, &token).unwrap();
        assert!(matches!(observed, VerifiedOutcome::Committed(_)));
        let old_write = local::execute(
            &receipt.operation,
            Some(&ropey::Rope::from_str("after\n")),
            &[],
            &recovered,
            &token,
        );
        assert!(matches!(
            old_write,
            StepOutcome::Refused(FsFailure {
                kind: FsFailureKind::Conflict,
                ..
            })
        ));
        assert_eq!(std::fs::read(&file).unwrap(), b"after\n");
    });
}

#[test]
fn store_verify_distinguishes_unchanged_from_landed() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    with_token(|context, token| {
        let intent = store_intent(&file, modified(&file), false, false, b"after\n");
        let plan = batch::prepare(context, &[intent], &environment(root.path()), &token).unwrap();
        assert_eq!(plan.steps.len(), 1);
        // The apply never ran; the frozen attempt verifies as unchanged.
        let attempt = StepReceipt {
            step: 0,
            operation: plan.steps[0].clone(),
            outcome: StepOutcome::Unconfirmed {
                detail: "transport lost before the receipt".into(),
                observed_destination: None,
                recovery: None,
                publication: None,
            },
        };
        assert!(matches!(
            local::verify(&attempt, context, &token).unwrap(),
            VerifiedOutcome::Unchanged
        ));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "before\n");
    });
}

#[test]
fn store_creates_an_absent_document_file() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("new.txt");
    with_token(|context, token| {
        let receipt = store_apply(
            context,
            root.path(),
            &store_intent(&file, None, false, false, b"created\n"),
            "created\n",
            &token,
        );
        assert!(receipt.outcome.is_committed(), "{receipt:?}");
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "created\n");
    });
}

#[test]
fn store_refuses_missing_parents_hardlinks_links_and_directories() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "before\n").unwrap();
    with_token(|context, token| {
        let missing = root.path().join("absent/child.txt");
        let plan = batch::prepare(
            context,
            &[store_intent(&missing, None, false, false, b"x\n")],
            &environment(root.path()),
            &token,
        )
        .unwrap();
        assert!(plan.steps.is_empty(), "no silent parent synthesis");
        assert_eq!(plan.refused.len(), 1);

        #[cfg(unix)]
        {
            let alias = root.path().join("alias.txt");
            std::os::unix::fs::symlink(&file, &alias).unwrap();
            let linked = root.path().join("linked.txt");
            std::fs::hard_link(&file, &linked).unwrap();
            let directory = root.path().join("dir");
            std::fs::create_dir(&directory).unwrap();
            for target in [&alias, &linked, &directory] {
                let plan = batch::prepare(
                    context,
                    &[store_intent(target, None, true, false, b"x\n")],
                    &environment(root.path()),
                    &token,
                )
                .unwrap();
                assert!(plan.steps.is_empty(), "{target:?} must refuse");
                assert_eq!(
                    plan.refused[0].failure.kind,
                    FsFailureKind::Unsupported,
                    "{:?}",
                    plan.refused[0].failure
                );
            }
        }
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "before\n");
    });
}

#[test]
fn store_edit_admission_proves_the_displayed_bytes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, "displayed\n").unwrap();
    with_token(|context, token| {
        use sha2::Digest;
        let mut admitted = store_intent(&file, None, false, false, b"");
        admitted.expected_content = None;
        let policy = admitted.store.as_mut().unwrap();
        policy.displayed = Some(sha2::Sha256::digest(b"displayed\n").into());
        let operation = prepare(context, root.path(), &admitted, &token);
        let observed = operation
            .destination
            .as_ref()
            .unwrap()
            .value
            .as_ref()
            .unwrap();
        assert_eq!(
            observed.digest,
            Some(sha2::Sha256::digest(b"displayed\n").into()),
            "admission evidence carries the proven digest"
        );
        // A stale snapshot is a conflict, never an admission.
        let mut stale = admitted.clone();
        stale.store.as_mut().unwrap().displayed = Some(sha2::Sha256::digest(b"stale\n").into());
        let plan = batch::prepare(context, &[stale], &environment(root.path()), &token).unwrap();
        assert!(plan.steps.is_empty());
        assert_eq!(plan.refused[0].failure.kind, FsFailureKind::Conflict);
    });
}

#[test]
fn failed_rename_keeps_original_and_private_stage_until_explicit_verification() {
    use std::os::unix::fs::PermissionsExt as _;
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    with_token(|context, token| {
        let receipt =
            local::test_support::with_fault(local::test_support::Fault::BeforeRename, || {
                store_apply(
                    context,
                    root.path(),
                    &store_intent(&file, modified(&file), false, false, b"after\n"),
                    "after\n",
                    &token,
                )
            });
        assert!(receipt.outcome.is_unconfirmed(), "{receipt:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"before\n");
        let stage = std::fs::read_dir(root.path())
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .find(|path| {
                path.file_name()
                    .is_some_and(|name| name.as_encoded_bytes().starts_with(b".strop-fs-"))
            })
            .expect("uncertain rename retains its private stage");
        assert_eq!(
            std::fs::metadata(stage).unwrap().permissions().mode() & 0o7777,
            0o700
        );
        assert!(matches!(
            local::verify(&receipt, context, &token).unwrap(),
            VerifiedOutcome::Unchanged
        ));
    });
}

#[test]
fn post_commit_directory_sync_failure_verifies_durable_intended_bytes() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    with_token(|context, token| {
        let receipt = local::test_support::with_fault(
            local::test_support::Fault::BeforeDirectorySync,
            || {
                store_apply(
                    context,
                    root.path(),
                    &store_intent(&file, modified(&file), false, false, b"after\n"),
                    "after\n",
                    &token,
                )
            },
        );
        assert!(receipt.outcome.is_unconfirmed(), "{receipt:?}");
        assert_eq!(std::fs::read(&file).unwrap(), b"after\n");
        assert!(matches!(
            local::verify(&receipt, context, &token).unwrap(),
            VerifiedOutcome::Committed(_)
        ));
    });
}

#[test]
fn metadata_restore_failure_refuses_before_publication_and_cleans_stage() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    with_token(|context, token| {
        let receipt = local::test_support::with_fault(local::test_support::Fault::Metadata, || {
            store_apply(
                context,
                root.path(),
                &store_intent(&file, modified(&file), false, false, b"after\n"),
                "after\n",
                &token,
            )
        });
        assert!(
            matches!(receipt.outcome, StepOutcome::Refused(_)),
            "{receipt:?}"
        );
        assert_eq!(std::fs::read(&file).unwrap(), b"before\n");
        assert!(!std::fs::read_dir(root.path()).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .as_encoded_bytes()
                .starts_with(b".strop-fs-")
        }));
    });
}

#[test]
fn third_party_write_during_private_stage_refuses_without_overwriting_it() {
    let root = tempfile::tempdir().unwrap();
    let file = root.path().join("note.txt");
    std::fs::write(&file, b"before\n").unwrap();
    with_token(|context, token| {
        let receipt =
            local::test_support::with_fault(local::test_support::Fault::ExternalWrite, || {
                store_apply(
                    context,
                    root.path(),
                    &store_intent(&file, modified(&file), false, false, b"after\n"),
                    "after\n",
                    &token,
                )
            });
        assert!(matches!(
            receipt.outcome,
            StepOutcome::Refused(FsFailure {
                kind: FsFailureKind::Conflict,
                ..
            })
        ));
        assert_eq!(std::fs::read(&file).unwrap(), b"third-party\n");
    });
}
