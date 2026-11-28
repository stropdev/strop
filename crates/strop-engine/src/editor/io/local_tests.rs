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
    use std::os::unix::fs::PermissionsExt;
    permissions.set_mode(0o444);
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

#[test]
fn a_file_larger_than_one_read_opens_byte_for_byte() {
    // the rope is built from the file in reads: a BOM, CRLF breaks and
    // multibyte chars straddling read edges all survive unchanged
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("big.txt");
    let mut text = String::from("\u{feff}");
    for line in 0..120_000 {
        text.push_str(&format!("{line:05} é界🙂 line\r\n"));
    }
    assert!(text.len() > 2 << 20, "spans several reads");
    std::fs::write(&path, &text).unwrap();
    let mut editor = Editor::new(Buffer::from_text(""));
    editor.open_fixture(&path).unwrap();
    assert!(editor.buf().text() == text.as_str());
}

#[test]
fn a_file_that_is_not_utf8_is_refused_and_the_buffer_kept() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("binary.log");
    let mut bytes = "text\n".repeat(400_000).into_bytes();
    bytes.push(0xff);
    std::fs::write(&path, bytes).unwrap();
    let mut editor = Editor::new(Buffer::from_text("origin\n"));
    editor.request_open(path, OpenIntent::Switch { readonly: false });
    let _ = editor.wait_io();
    assert_eq!(editor.buf().text(), "origin\n");
    assert!(
        editor.message.contains("UTF-8"),
        "typed and visible: {}",
        editor.message
    );
}

#[test]
fn a_cancelled_read_stops_without_a_rope() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("note.txt");
    std::fs::write(&path, "local\n").unwrap();
    let (token, handle) = strop_core::worker::CancelToken::standalone();
    handle.cancel(strop_core::worker::CancelReason::OwnerClosed);
    let read = crate::editor::namespace::LocalFs::default()
        .read_rope(&token, &strop_workspace::ResourceLocation::local(path));
    assert_eq!(
        read.map(|_| ()).unwrap_err().kind,
        strop_workspace::operation::FsFailureKind::Cancelled
    );
}
