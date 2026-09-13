use super::*;
use crate::tests::{environment, intent, with_token};

#[test]
fn committed_rename_with_io_error_retains_trash_metadata_and_verifiable_recovery() {
    let root = tempfile::tempdir().unwrap();
    let source = root.path().join("original.txt");
    std::fs::write(&source, "original bytes").unwrap();
    with_token(|token| {
        let operation = crate::local::prepare(
            &intent(OperationKind::Trash, Some(&source), None),
            false,
            &environment(root.path()),
            &token,
        )
        .unwrap();
        let result = execute_with(&operation, &token, |from, name, to, destination| {
            rustix::fs::renameat_with(
                from,
                name,
                to,
                destination,
                rustix::fs::RenameFlags::NOREPLACE,
            )
            .map_err(crate::local::rename_error)?;
            Err(failure(
                FsFailureKind::Io,
                "injected lost rename acknowledgment",
            ))
        });
        assert!(!source.exists(), "the native move actually happened");
        let outcome = result.unwrap();
        let StepOutcome::Unconfirmed {
            recovery: Some(recovery),
            publication: Some(_),
            ..
        } = &outcome
        else {
            panic!("ambiguous publication lost its owned recovery evidence: {outcome:?}");
        };
        assert_eq!(std::fs::read(&recovery.path).unwrap(), b"original bytes");
        let info_name = format!(
            "{}.trashinfo",
            recovery.path.file_name().unwrap().to_str().unwrap()
        );
        let info = recovery
            .path
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("info")
            .join(info_name);
        let metadata = std::fs::read_to_string(info).unwrap();
        assert!(metadata.contains(&format!(
            "Path={}\n",
            strop_workspace::addr::uri::encode_path(&source)
        )));
        let receipt = strop_workspace::operation::StepReceipt {
            step: 0,
            operation,
            outcome,
        };
        assert!(matches!(
            crate::local::verify(&receipt, &token).unwrap(),
            strop_workspace::operation::VerifiedOutcome::Committed(_)
        ));
    });
}
