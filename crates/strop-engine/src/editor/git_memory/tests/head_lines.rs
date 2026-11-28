//! A permalink and a line history name HEAD lines (#290, #291): buffer
//! rows map through the unstaged and staged regions of the buffer's hunk
//! snapshot, a range holding an uncommitted change is refused with the
//! reason, and a request waits for the snapshot of exactly this revision.

use std::process::Command;

use super::{ready_url, set_remote};
use crate::editor::git_memory::HeadLines;
use crate::editor::test_support::git::{fixture, git_out, pump, pump_hunks, settle, settle_dives};
use crate::editor::{Editor, Surface};
use strop_core::frontend_input::Key;
use strop_core::Buffer;

const ORIGIN: &str = "https://github.com/stropdev/strop.git";

/// `fixture()` (HEAD's `f.rs` is two lines) with a web remote and three
/// lines inserted above them in the buffer, unsaved: rows 4 and 5 are
/// HEAD lines 1 and 2.
fn three_inserted() -> (tempfile::TempDir, Editor, String) {
    let (d, mut e) = fixture();
    let root = e.cwd.clone();
    set_remote(&mut e, &root, "origin", ORIGIN);
    for _ in 0..3 {
        e.feed_text("ggOx");
        e.feed(Key::Esc);
    }
    assert_eq!(
        e.buf().text().to_string(),
        "x\nx\nx\nfn a() {}\nfn b() {}\n"
    );
    let sha = git_out(&root, &["rev-parse", "HEAD"]);
    (d, e, sha)
}

fn link(sha: &str, line: usize) -> String {
    format!("https://github.com/stropdev/strop/blob/{sha}/f.rs#L{line}")
}

#[test]
fn permalink_maps_rows_through_uncommitted_edits() {
    let (_d, mut e, sha) = three_inserted();
    e.feed_text("G gy");
    pump_hunks(&mut e);
    assert_eq!(e.register(None).text, link(&sha, 2), "{}", e.message);
    // an inserted row has no HEAD line
    e.feed_text("gg gy");
    assert_eq!(e.message, "line 1 is not in HEAD (uncommitted change)");
    assert_eq!(e.register(None).text, link(&sha, 2), "nothing new copied");
    // a selection across the insertion is refused, not mapped by its ends
    let HeadLines::Refused(reason) = e.head_lines((2, 4)) else {
        panic!("a range holding inserted lines maps nowhere");
    };
    assert_eq!(
        reason,
        "lines 3–5 are not in HEAD (uncommitted changes in the selection)"
    );
    assert!(matches!(e.head_lines((3, 4)), HeadLines::Ready((1, 2))));
}

#[test]
fn permalink_maps_rows_through_staged_and_unstaged_edits() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(root)
            .env("GIT_CONFIG_GLOBAL", "/dev/null")
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    };
    git(&["init", "-q"]);
    std::fs::write(root.join("f.rs"), "a\nb\n").unwrap();
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@t.t",
        "commit",
        "-qm",
        "base",
    ]);
    // one staged insertion above, then one unstaged insertion above it
    std::fs::write(root.join("f.rs"), "s\na\nb\n").unwrap();
    git(&["add", "f.rs"]);
    let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
    e.cwd = root.to_path_buf();
    e.discover_git();
    settle(&mut e, |e| e.git.is_some());
    set_remote(&mut e, root, "origin", ORIGIN);
    e.feed_text("ggOu");
    e.feed(Key::Esc);
    assert_eq!(e.buf().text().to_string(), "u\ns\na\nb\n");
    let sha = git_out(root, &["rev-parse", "HEAD"]);
    e.feed_text("G gy");
    pump_hunks(&mut e);
    assert_eq!(e.register(None).text, link(&sha, 2), "{}", e.message);
    // the staged line is in the index, not in HEAD
    e.feed_text("2G gy");
    assert_eq!(e.message, "line 2 is not in HEAD (uncommitted change)");
}

#[test]
fn permalink_waits_for_the_hunk_snapshot() {
    let (_d, mut e, sha) = three_inserted();
    e.feed_text("G gy");
    assert!(
        e.register(None).text.is_empty(),
        "nothing copied before the diff lands"
    );
    assert!(e.osc52.is_none());
    assert_eq!(e.message, "waiting for the diff…");
    pump_hunks(&mut e);
    assert_eq!(e.register(None).text, link(&sha, 2));
    assert_eq!(e.osc52.as_deref(), Some(link(&sha, 2).as_str()));
    assert_eq!(e.message, "permalink copied");
    // an edit while the diff is in flight drops the request
    e.osc52 = None;
    e.feed_text("Goy");
    e.feed(Key::Esc);
    e.feed_text("4G gy");
    assert_eq!(e.message, "waiting for the diff…");
    e.feed_text("ix");
    e.feed(Key::Esc);
    pump_hunks(&mut e);
    assert_eq!(e.message, "buffer changed — permalink dropped");
    assert!(e.osc52.is_none(), "nothing copied for a moved buffer");
}

#[test]
fn permalink_from_hunk_preview_names_head_lines() {
    let (_d, mut e, sha) = three_inserted();
    pump_hunks(&mut e);
    e.feed_text("gg gp");
    settle_dives(&mut e);
    assert!(matches!(e.surface(), Some(Surface::Diff { .. })));
    // heading, hunk header, three added rows, then the two context rows
    e.feed_text("6G");
    assert_eq!(ready_url(&e), link(&sha, 1));
    e.feed_text("3G gy");
    assert_eq!(e.message, "line 1 is not in HEAD (uncommitted change)");
}

#[test]
fn line_history_maps_rows_through_uncommitted_edits() {
    let (_d, mut e, _sha) = three_inserted();
    e.open_line_history(5, 5);
    pump_hunks(&mut e);
    pump(&mut e);
    assert_eq!(e.buf().name.as_deref(), Some("git log · f.rs:2-2"));
    let text = e.buf().text().to_string();
    assert!(text.contains("add b"), "{text}");
    assert!(!text.contains("failed"), "{text}");
}

#[test]
fn line_history_refuses_uncommitted_lines() {
    let (_d, mut e, _sha) = three_inserted();
    pump_hunks(&mut e);
    e.open_line_history(1, 2);
    assert_eq!(
        e.message,
        "lines 1–2 are not in HEAD (uncommitted changes in the selection)"
    );
    assert!(e.surface().is_none(), "no history surface opens");
}
