//! Document save (`:w`) is a Store intent through the checked kernel:
//! conflict (baseline moved), permission preservation, same-size content
//! change and uncertainty→verify semantics, with frozen evidence retained
//! while unconfirmed.

use super::*;
use strop_core::Buffer;
use strop_workspace::operation::PreparedOperation;
use strop_workspace::operation::{OperationIntent, OperationKind, StepOutcome, StepReceipt};

fn store_intent(editor: &Editor, path: &std::path::Path) -> OperationIntent {
    use sha2::Digest;
    let buffer = &editor.buf();
    let mut bytes = Vec::new();
    for chunk in buffer.snapshot().chunks() {
        bytes.extend_from_slice(chunk.as_bytes());
    }
    let digest: [u8; 32] = sha2::Sha256::digest(&bytes).into();
    let baseline = buffer.disk_stamp().map(super::open::systemtime_to_filetime);
    OperationIntent {
        kind: OperationKind::Store,
        source: None,
        destination: Some(strop_workspace::ResourceLocation::local(path.to_path_buf())),
        copy_version: strop_workspace::operation::CopyVersion::Stored,
        expected_content: Some(digest),
        store: Some(strop_workspace::operation::StorePolicy {
            baseline,
            baseline_object: None,
            baseline_attributes: None,
            force: false,
            expect_absent: false,
            displayed: None,
        }),
    }
}

/// Insert a frozen unconfirmed attempt as if the receipt had been lost.
fn inject_unconfirmed(
    editor: &mut Editor,
    document: DocumentId,
    path: &std::path::Path,
) -> PreparedOperation {
    let (token, _handle) = strop_core::worker::CancelToken::standalone();
    let operation = editor
        .filesystem
        .local()
        .prepare_store(&token, store_intent(editor, path))
        .expect("the store prepares");
    let attempt = StoreAttempt {
        revision: editor.buf().revision(),
        origin: editor.buf().path.clone(),
        target: path.to_path_buf(),
        write_target: path.to_path_buf(),
        close: false,
        force: false,
        focus: editor.focus_epoch,
        receipt: StepReceipt {
            step: 0,
            operation: operation.clone(),
            outcome: StepOutcome::Unconfirmed {
                detail: "simulated lost receipt".into(),
                observed_destination: None,
                recovery: None,
                publication: None,
            },
        },
    };
    editor.io.store_attempts.insert(document, attempt);
    let revision = editor.buf().revision();
    editor.note_unconfirmed_save(
        document,
        revision,
        "simulated lost receipt; verify before any rewrite".into(),
    );
    operation
}

#[test]
fn local_save_is_a_store_intent() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Oafter<esc>");
    assert!(editor.buf().is_dirty());
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\nbefore\n");
    assert!(
        !editor.buf().is_dirty(),
        "the committed store retires the revision"
    );
    assert_eq!(editor.message, "written");
}

#[test]
fn save_conflict_on_a_moved_baseline_and_force_overwrites() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Omine<esc>");
    // The baseline moved: another writer landed meanwhile.
    std::fs::write(&path, "theirs\n").unwrap();
    std::fs::File::options()
        .write(true)
        .open(&path)
        .unwrap()
        .set_times(
            std::fs::FileTimes::new()
                .set_modified(std::time::UNIX_EPOCH + std::time::Duration::from_secs(42)),
        )
        .unwrap();
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert!(
        editor.message.contains("file changed on disk"),
        "typed conflict, dirty preserved: {}",
        editor.message
    );
    assert!(editor.buf().is_dirty());
    assert!(editor.feedback()[0]
        .detail()
        .contains("file changed on disk"));
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "theirs\n");
    // :w! overwrites the moved baseline.
    assert!(editor.request_save_document(document, None, true, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\nbefore\n");
    assert!(!editor.buf().is_dirty());
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        crate::editor::feedback::FeedbackLifecycle::Resolved
    );
}

#[test]
fn save_as_refuses_an_occupied_name_unless_forced() {
    let directory = tempfile::tempdir().unwrap();
    let occupied = directory.path().join("occupied.txt");
    std::fs::write(&occupied, "keep\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor
        .open_fixture(&directory.path().join("note.txt"))
        .unwrap();
    editor.feed_text("Odata<esc>");
    assert!(editor.request_save_document(document, Some(occupied.clone()), false, false));
    editor.wait_io().unwrap();
    assert!(
        editor.message.contains("file exists"),
        "occupied save-as refuses: {}",
        editor.message
    );
    assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "keep\n");
    assert!(editor.request_save_document(document, Some(occupied.clone()), true, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&occupied).unwrap(), "data\n");
    assert_eq!(
        editor.buf().path.as_deref(),
        Some(occupied.as_path()),
        "a committed save-as rebinds the document"
    );
}

#[cfg(unix)]
#[test]
fn save_preserves_permissions_through_the_atomic_replace() {
    use std::os::unix::fs::PermissionsExt;
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("locked.txt");
    std::fs::write(&path, "before\n").unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o640)).unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Oafter<esc>");
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "after\nbefore\n");
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o7777,
        0o640,
        "permissions survive the store"
    );
}

#[test]
fn same_size_content_change_commits_and_verifies() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "aaaa\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("ccbbbb<esc>");
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "bbbb\n");
    assert!(!editor.buf().is_dirty());
}

#[test]
fn unconfirmed_save_verifies_before_any_rewrite() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Omine<esc>");
    let _ = inject_unconfirmed(&mut editor, document, &path);
    // The next :w verifies instead of rewriting; the write never landed,
    // so the original is unchanged and dirty text is preserved.
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(
        editor.message, "the original is unchanged on disk; :w writes again",
        "verification reconciles without writing"
    );
    assert!(editor.buf().is_dirty());
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
    assert!(
        editor.io.store_attempts.is_empty(),
        "a verified-unchanged attempt clears"
    );
    // The explicit retry writes fresh.
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "mine\nbefore\n");
    assert!(!editor.buf().is_dirty());
}

#[test]
fn dismissed_unconfirmed_save_still_offers_verification_and_resolves() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Omine<esc>");
    let _ = inject_unconfirmed(&mut editor, document, &path);
    let id = editor.feedback()[0].id();
    editor.invoke_feedback_action(id, 1);
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        crate::editor::feedback::FeedbackLifecycle::Dismissed
    );
    assert!(editor.io.store_attempt(document).is_some());

    editor.feed_text("gg/unrelated<esc>");
    editor.context_actions();
    let picker = &editor.picker.as_ref().unwrap().picker;
    assert!(picker
        .items
        .iter()
        .any(|item| item.text.contains("Verify the unconfirmed save")));
    assert!(!picker
        .items
        .iter()
        .any(|item| item.text.contains("Retry the failed save")));
    editor.accept_action(0);
    editor.wait_io().unwrap();

    assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
    assert!(editor.buf().is_dirty());
    assert_eq!(
        editor.feedback()[0].lifecycle(),
        crate::editor::feedback::FeedbackLifecycle::Resolved
    );
    editor.open_explain();
    let text = editor.buf().text().to_string();
    assert!(text.contains("simulated lost receipt"), "{text}");
    assert!(text.contains("local StoreAttempt"), "{text}");
}

#[test]
fn a_late_committed_store_reconciles_the_source_once() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Omine<esc>");
    let operation = inject_unconfirmed(&mut editor, document, &path);
    let (token, _handle) = strop_core::worker::CancelToken::standalone();
    let receipt = editor.filesystem.local().apply_store(
        &token,
        operation,
        ropey::Rope::from_str("mine\nbefore\n"),
    );
    assert!(matches!(receipt.outcome, StepOutcome::Committed { .. }));
    assert!(editor.request_save_document(document, None, false, false));
    editor.wait_io().unwrap();
    assert_eq!(
        editor.message, "written",
        "verified committed: {}",
        editor.message
    );
    assert!(
        !editor.buf().is_dirty(),
        "the late committed receipt retires the saved revision"
    );
    assert!(editor.io.store_attempts.is_empty());
}

#[test]
fn a_forced_rewrite_cannot_bypass_an_unverified_outcome() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "before\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    let document = editor.open_fixture(&path).unwrap();
    editor.feed_text("Omine<esc>");
    let _ = inject_unconfirmed(&mut editor, document, &path);
    // :w! verifies too — force is never a blind-rewrite bypass.
    assert!(editor.request_save_document(document, None, true, false));
    editor.wait_io().unwrap();
    assert_eq!(std::fs::read_to_string(&path).unwrap(), "before\n");
    assert!(editor.buf().is_dirty());
}
