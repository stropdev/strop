//! Local opens run in process through the checked observation and read
//! family: readonly/new-file semantics are the kernel's observation
//! evidence.

use super::OpenIntent;
use super::*;
use strop_core::Buffer;

#[test]
fn local_open_reads_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "local\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "local\n");
    assert!(!editor.buf().readonly_reason.is_some());
}

#[test]
fn missing_file_opens_empty() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.txt");
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "");
    assert_eq!(editor.buf().path.as_deref(), Some(path.as_path()));
}

#[test]
fn readonly_evidence_is_the_observation() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("locked.txt");
    std::fs::write(&path, "read only\n").unwrap();
    let mut permissions = std::fs::metadata(&path).unwrap().permissions();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        permissions.set_mode(0o444);
    }
    std::fs::set_permissions(&path, permissions).unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "read only\n");
    assert_eq!(
        editor.buf().readonly_reason,
        Some(strop_core::ReadonlyReason::Filesystem),
        "the observed permissions"
    );
}

#[test]
fn a_broken_symlink_is_a_typed_failure_not_a_new_file() {
    let directory = tempfile::tempdir().unwrap();
    let link = directory.path().join("dangling");
    std::os::unix::fs::symlink(directory.path().join("absent"), &link).unwrap();
    let mut editor = Editor::new(Buffer::from_text("origin\n"));
    editor.request_open(link, OpenIntent::Switch { readonly: false });
    let _ = editor.wait_io();
    assert_eq!(editor.buf().text(), "origin\n");
    assert!(
        editor.message.contains("symlink target is unavailable"),
        "typed and visible: {}",
        editor.message
    );
}
