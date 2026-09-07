/// Settle async work like the event loop would: drain until `done`
/// or the deadline (real threads need real pumping).
fn settle(e: &mut Editor, done: impl Fn(&Editor) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while !done(e) {
        if e.io_pending() {
            e.wait_io().unwrap();
        }
        e.drain_git_jobs();
        if done(e) {
            break;
        }
        let remaining = deadline.saturating_duration_since(std::time::Instant::now());
        let event = e
            .git_rx
            .as_ref()
            .unwrap()
            .recv_timeout(remaining)
            .expect("git job completes");
        e.handle_git_job(event);
    }
}

/// The gutter is async: settle discovery, then refresh and settle
/// the diff to a terminal Load state.
fn pump_hunks(e: &mut Editor) {
    settle(e, |e| e.git.is_some());
    e.refresh_hunks();
    let terminal = |e: &Editor| !matches!(e.hunk_load, strop_core::worker::Load::Running(_));
    settle(e, terminal);
    e.refresh_hunks(); // a failed load re-enqueues only via commands
    settle(e, terminal);
}

use std::process::Command;

use super::*;
use crate::editor::Key;
use crate::editor::Surface;
use strop_core::Buffer;

/// A git repo with one committed file, edited in-memory.
fn fixture() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    for args in [
        vec!["init", "-q"],
        vec!["config", "user.email", "t@t.t"],
        vec!["config", "user.name", "t"],
    ] {
        Command::new("git")
            .args(&args)
            .current_dir(root)
            .output()
            .unwrap();
    }
    std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    Command::new("git")
        .args(["add", "."])
        .current_dir(root)
        .output()
        .unwrap();
    Command::new("git")
        .args(["commit", "-qm", "init"])
        .current_dir(root)
        .output()
        .unwrap();
    let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
    e.discover_git();
    (dir, e)
}

#[test]
fn gutter_tracks_live_edits() {
    let (_d, mut e) = fixture();
    pump_hunks(&mut e);
    assert_eq!(e.sign_at(1), None, "clean buffer has no signs");
    e.feed_text("G");
    e.feed_text("ofn c() {}");
    e.feed_text("<esc>");
    pump_hunks(&mut e);
    assert_eq!(e.sign_at(3), Some('+'), "added line signs +");
    assert_eq!(e.sign_at(1), None);
}

#[test]
fn hunk_nav_and_undo() {
    let (_d, mut e) = fixture();
    e.feed_text("Go");
    e.feed_text("fn c() {}");
    e.feed_text("<esc>");
    e.feed_text("gg");
    pump_hunks(&mut e);
    e.jump_hunk(true);
    assert_eq!(e.buf().line_of(e.head()) + 1, 3, "]c lands on the hunk");
    e.undo_hunk();
    assert_eq!(e.buf().text().to_string(), "fn a() {}\nfn b() {}\n");
}

#[test]
fn space_g_namespace_dispatches() {
    let (_d, mut e) = fixture();
    e.feed_text("G");
    e.feed_text("o");
    for c in "fn new() {}".chars() {
        e.feed(Key::Char(c));
    }
    e.feed(Key::Esc);
    pump_hunks(&mut e);
    e.feed_text(" gp"); // Space, g, p
    assert!(
        matches!(e.surface(), Some(Surface::Diff { .. })),
        "Space g p opens the hunk surface (buffer: {})",
        e.buf().text()
    );
}

/// The hunk surface is a real readonly buffer you can move in, and
/// ` g u` from it restores the origin buffer (0010 §2).
#[test]
fn hunk_surface_moves_and_undoes() {
    let (_d, mut e) = fixture();
    e.feed_text("Go");
    e.feed_text("fn c() {}");
    e.feed_text("<esc>");
    pump_hunks(&mut e);
    e.feed_text("]c"); // like the tape: jump onto the hunk first
    e.feed_text(" gp");
    assert!(e.buf().readonly);
    assert!(e.buf().text().to_string().contains("fn c() {}"));
    // motions work on the hunk surface
    e.feed_text("j");
    e.feed_text("j");
    assert_eq!(e.buf().line_of(e.head()), 2);
    // undo acts on the origin file buffer, not the surface
    e.feed_text(" gu");
    let text = e.doc(e.first_doc()).buf.text().to_string();
    assert_eq!(text, "fn a() {}\nfn b() {}\n", "hunk restored: {text}");
    e.feed_text("q");
    assert_eq!(e.current(), e.first_doc());
}

/// 0014 P0: staging must never silently write unrelated unsaved
/// edits — a dirty buffer refuses with a pointer to :w.
#[test]
fn stage_refuses_a_dirty_buffer() {
    let (d, mut e) = fixture();
    e.feed_text("Go");
    e.feed_text("fn c() {}");
    e.feed_text("<esc>gg");
    pump_hunks(&mut e);
    e.feed_text("]c");
    e.feed_text(" gs");
    assert!(
        e.message.contains(":w first"),
        "dirty stage refuses: {}",
        e.message
    );
    let staged = Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(
        staged.stdout.is_empty(),
        "nothing reached the index: {}",
        String::from_utf8_lossy(&staged.stdout)
    );
    // after an explicit save, staging works (the mutation settles
    // off the input path; the message reports the outcome)
    e.feed_text(":w\r");
    settle(&mut e, |e| !e.doc(e.first_doc()).buf.dirty);
    e.feed_text(" gs");
    settle(&mut e, |e| e.git_mutation.is_none());
    assert!(e.message.contains("staged"), "{}", e.message);
}

/// A stale preview refuses honestly: edits after opening it change
/// the revision, and applying the stored region would cut wrong.
#[test]
fn stale_hunk_surface_refuses() {
    let (_d, mut e) = fixture();
    e.feed_text("Go");
    e.feed_text("fn c() {}");
    e.feed_text("<esc>");
    pump_hunks(&mut e);
    e.feed_text("gg]c gp");
    // edit the origin document: the revision moves, the preview goes stale
    // (the active pane's document IS the current one — the surface —
    // so point a second pane at the file for the cursor-keep branch)
    e.panes.push(crate::editor::Pane {
        doc: e.first_doc(),
        sels: strop_core::selection::SelectionSet::default(),
        view_top: 0,
        hscroll: strop_core::id::DisplayColumn::new(0),
        desired_column: None,
    });
    pump_hunks(&mut e);
    e.doc_mut(e.first_doc())
        .buf
        .edit()
        .insert(0, "// touched\n")
        .unwrap();
    e.feed_text(" gu");
    assert!(
        e.message.contains("buffer changed"),
        "stale preview must refuse: {}",
        e.message
    );
}

/// 0014 wave 4: the full edge dance by keys — edit, save, stage,
/// unstage; the index is the witness.
#[test]
fn stage_and_unstage_name_their_edges() {
    let (d, mut e) = fixture();
    e.feed_text("Gofn c() {}");
    e.feed_text("<esc>");
    e.feed_text(":w\r");
    settle(&mut e, |e| !e.doc(e.first_doc()).buf.dirty);
    pump_hunks(&mut e);
    e.feed_text("]c gs");
    settle(&mut e, |e| e.git_mutation.is_none());
    assert!(e.message.contains("staged"), "{}", e.message);
    let staged = Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(!staged.stdout.is_empty(), "hunk in the index");
    // staged set drives the gutter's committed-adjacent tint
    pump_hunks(&mut e);
    assert!(e.sign_at_staged(3), "staged line marked");
    assert!(e.sign_at(3).is_none(), "not also unstaged");
    e.feed_text(" gS");
    settle(&mut e, |e| e.git_mutation.is_none());
    assert!(e.message.contains("unstaged"), "{}", e.message);
    let staged = Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(staged.stdout.is_empty(), "index back to HEAD");
    // and now the same line reads as unstaged again
    pump_hunks(&mut e);
    assert_eq!(e.sign_at(3), Some('+'));
}
