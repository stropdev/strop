//! WK04 pins: the default local path IS the worker client — opens,
//! listings and observations ride the lease (in-process transport under
//! test, the spawned `--worker-stdio` binary in production), and the
//! lease is observable; readonly/new-file semantics are the worker's
//! observation evidence, not in-process stat calls.

use super::OpenIntent;
use super::*;
use strop_core::Buffer;

#[test]
fn local_open_rides_the_worker_lease() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "via worker\n").unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    assert!(
        editor.filesystem.worker().session().is_none(),
        "the worker spawns lazily, never at editor construction"
    );
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "via worker\n");
    assert!(
        editor.filesystem.worker().session().is_some(),
        "the worker served the open"
    );
    assert!(!editor.buf().readonly_reason.is_some());
}

#[test]
fn missing_file_opens_empty_through_the_worker() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("new.txt");
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    assert_eq!(editor.buf().text(), "");
    assert_eq!(editor.buf().path.as_deref(), Some(path.as_path()));
    assert!(editor.filesystem.worker().session().is_some());
}

#[test]
fn readonly_evidence_is_the_worker_observation() {
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
        "the worker's observed permissions, not a local guess"
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
