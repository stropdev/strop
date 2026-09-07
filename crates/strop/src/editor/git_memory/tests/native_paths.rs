//! Distinct native filenames may share a lossy label but never a navigation target.
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::process::Command;

use crate::editor::{Editor, Key, Surface};
use strop_core::Buffer;

use super::{pump, settle};

fn git(root: &Path, args: &[&str]) {
    let output = Command::new("git")
        .args(args)
        .current_dir(root)
        .env("HOME", root)
        .env("XDG_CONFIG_HOME", root)
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("GIT_CONFIG_GLOBAL", root.join("absent-global-config"))
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn current_file(editor: &Editor) -> Option<&Path> {
    match editor.surface() {
        Some(Surface::Diff {
            commit: Some(commit),
            ..
        }) => Some(&commit.current),
        _ => None,
    }
}

#[test]
fn commit_navigation_keeps_native_identity_when_display_labels_collide() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let first = PathBuf::from(OsStr::from_bytes(b"\xfe.rs"));
    let second = PathBuf::from(OsStr::from_bytes(b"\xff.rs"));
    assert_eq!(first.to_string_lossy(), second.to_string_lossy());
    git(root, &["init", "-q", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.test"]);
    std::fs::write(root.join("notes.txt"), "workspace\n").unwrap();
    std::fs::write(root.join(&first), "old first\n").unwrap();
    std::fs::write(root.join(&second), "old second\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join(&first), "new first\n").unwrap();
    std::fs::write(root.join(&second), "new second\n").unwrap();
    git(root, &["commit", "-qam", "both native files"]);

    let mut editor = Editor::new_in(
        Buffer::open(root.join("notes.txt")).unwrap(),
        root.to_owned(),
    );
    editor.discover_git();
    settle(&mut editor, |editor| editor.git.is_some());
    editor.open_log(false);
    pump(&mut editor);
    editor.feed(Key::Enter);
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    let Some(Surface::ChangedFiles { files, .. }) = editor.surface() else {
        unreachable!()
    };
    assert_eq!(files.len(), 2);
    assert!(files.iter().any(|file| file.path == first));
    assert!(files.iter().any(|file| file.path == second));
    editor.feed_text("jj");
    editor.feed(Key::Enter);
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    let initial = current_file(&editor).unwrap().to_owned();
    let initial_text = editor.buf().text().to_string();
    let expected = if initial == first {
        "new first"
    } else {
        "new second"
    };
    assert!(initial_text.contains(expected));

    editor.feed_text("]f");
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    let expected = if initial == first {
        "new second"
    } else {
        "new first"
    };
    assert!(editor.buf().text().to_string().contains(expected));
    editor.feed_text("[f");
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    assert_eq!(editor.buf().text().to_string(), initial_text);
}

#[test]
fn control_characters_in_filenames_keep_one_row_and_a_native_dive_target() {
    let directory = tempfile::tempdir().unwrap();
    let root = directory.path();
    let path = Path::new("odd\nname\t\u{1b}.txt");
    git(root, &["init", "-q", "--initial-branch=main"]);
    git(root, &["config", "user.name", "Fixture"]);
    git(root, &["config", "user.email", "fixture@example.test"]);
    std::fs::write(root.join("notes.txt"), "workspace\n").unwrap();
    std::fs::write(root.join(path), "before\n").unwrap();
    git(root, &["add", "."]);
    git(root, &["commit", "-qm", "base"]);
    std::fs::write(root.join(path), "after\n").unwrap();
    git(root, &["commit", "-qam", "control filename"]);

    let mut editor = Editor::new_in(
        Buffer::open(root.join("notes.txt")).unwrap(),
        root.to_owned(),
    );
    editor.discover_git();
    settle(&mut editor, |editor| editor.git.is_some());
    editor.open_log(false);
    pump(&mut editor);
    editor.feed(Key::Enter);
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    assert_eq!(
        editor.buf().last_content_line(),
        2,
        "one buffer row per file"
    );
    let start = editor.buf().line_start(2);
    let end = editor.buf().line_end(2);
    assert_eq!(
        editor.buf().text().byte_slice(start..end),
        "odd\u{fffd}name\u{fffd}\u{fffd}.txt"
    );
    editor.feed_text("jj");
    editor.feed(Key::Enter);
    settle(&mut editor, |editor| editor.dive_requests.is_empty());
    assert_eq!(current_file(&editor), Some(path), "{}", editor.message);
    assert!(editor
        .buf()
        .text()
        .to_string()
        .lines()
        .any(|line| line == "after"));
    assert_eq!(
        editor.buf().text().line(0).to_string(),
        "odd\u{fffd}name\u{fffd}\u{fffd}.txt +1 -1\n"
    );
}
