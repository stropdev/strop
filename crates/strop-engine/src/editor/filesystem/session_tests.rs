use strop_workspace::operation::{FsFailureKind, StepOutcome};

#[test]
fn old_review_cannot_mutate_after_a_new_worker_session_is_ready() {
    let (root, mut editor) = super::tests::fixture();
    let source = root.path().join("original.txt");
    let destination = root.path().join("renamed.txt");
    std::fs::write(&source, "kept\n").unwrap();
    editor.open_fixture(&source).unwrap();
    super::tests::command(&mut editor, ":fs rename renamed.txt<cr>");
    let prepared = editor
        .filesystem
        .pending
        .as_ref()
        .unwrap()
        .batch
        .worker_session
        .unwrap();
    let worker = editor.filesystem.worker().clone();
    worker.shutdown().unwrap();
    let (token, _owner) = strop_core::worker::CancelToken::standalone();
    worker.health(&token).unwrap();
    assert_ne!(worker.session(), Some(prepared));

    super::tests::command(&mut editor, ":apply-change<cr>");
    assert_eq!(std::fs::read(&source).unwrap(), b"kept\n");
    assert!(
        !destination.exists(),
        "old review must not execute on the new worker"
    );
    let attempt = editor.filesystem.history.back().unwrap();
    assert!(matches!(
        &attempt.receipts[0].outcome,
        StepOutcome::Refused(error) if error.kind == FsFailureKind::Conflict
    ));
    worker.shutdown().unwrap();
}
