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

use std::process::Command;

use super::*;
use crate::editor::test_support::git::pump_hunks;
use crate::editor::Surface;
use strop_core::frontend_input::Key;
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
    assert_eq!(e.sign_at(3), Some(Sign::Add), "an added line signs add");
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
    e.discard_hunk();
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
    settle(&mut e, |e| e.dive_requests.is_empty());
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
    settle(&mut e, |e| e.dive_requests.is_empty());
    assert!(e.buf().readonly);
    assert!(e.buf().text().to_string().contains("fn c() {}"));
    // motions work on the hunk surface
    e.feed_text("j");
    e.feed_text("j");
    assert_eq!(e.buf().line_of(e.head()), 2);
    // undo acts on the origin file buffer, not the surface
    e.feed_text(" gX");
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
    settle(&mut e, |e| !e.doc(e.first_doc()).buf.is_dirty());
    e.feed_text(" gs");
    settle(&mut e, |e| e.git_work.mutation.is_none());
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
    settle(&mut e, |e| e.dive_requests.is_empty());
    // edit the origin document: the revision moves, the preview goes stale
    // (the active pane's document IS the current one — the surface —
    // so point a second pane at the file for the cursor-keep branch)
    e.panes.push(crate::editor::Pane {
        terminal_input: false,
        doc: e.first_doc(),
        sels: strop_core::selection::SelectionSet::default(),
        view_top: 0,
        hscroll: strop_core::id::DisplayColumn::new(0),
        desired_column: None,
        focused: 0,
        jumps: Default::default(),
    });
    pump_hunks(&mut e);
    e.doc_mut(e.first_doc())
        .buf
        .edit()
        .insert(0, "// touched\n")
        .unwrap();
    e.feed_text(" gX");
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
    settle(&mut e, |e| !e.doc(e.first_doc()).buf.is_dirty());
    pump_hunks(&mut e);
    e.feed_text("]c gs");
    settle(&mut e, |e| e.git_work.mutation.is_none());
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
    e.feed_text(" gu");
    settle(&mut e, |e| e.git_work.mutation.is_none());
    assert!(e.message.contains("unstaged"), "{}", e.message);
    let staged = Command::new("git")
        .args(["diff", "--cached", "--stat"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert!(staged.stdout.is_empty(), "index back to HEAD");
    // and now the same line reads as unstaged again
    pump_hunks(&mut e);
    assert_eq!(e.sign_at(3), Some(Sign::Add));
}

#[test]
fn directory_alias_and_physical_path_stage_the_same_native_hunk() {
    use std::ffi::OsStr;
    use std::os::unix::{ffi::OsStrExt, fs::symlink};
    use std::path::Path;

    let dir = tempfile::tempdir().unwrap();
    let project = dir.path().join("project");
    let alias = dir.path().join("alias");
    std::fs::create_dir(&project).unwrap();
    symlink(&project, &alias).unwrap();
    let git = |args: &[&str]| {
        let out = Command::new("git")
            .current_dir(&project)
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
        out.stdout
    };
    let rel = Path::new(OsStr::from_bytes(b"text-\xff\\name"));
    let old = b"one\ntwo\nthree\n";
    let edit = b"ONE\ntwo\nthree\n";
    std::fs::write(project.join(rel), old).unwrap();
    git(&["init", "-q"]);
    git(&["add", "."]);
    git(&[
        "-c",
        "user.name=t",
        "-c",
        "user.email=t@example.org",
        "commit",
        "-qm",
        "base",
    ]);
    std::fs::write(project.join(rel), edit).unwrap();
    let repo = strop_git::Repo::discover(&project).unwrap();

    for cwd in [&project, &alias] {
        let path = cwd.join(rel);
        let mut e = Editor::new(Buffer::open(&path).unwrap());
        e.cwd = cwd.clone();
        e.discover_git();
        pump_hunks(&mut e);
        assert_eq!(
            e.sign_lines().map(|(line, _)| line).collect::<Vec<_>>(),
            [1],
            "{path:?}: {}",
            e.message
        );
        e.feed_text(" gs");
        settle(&mut e, |e| e.git_work.mutation.is_none());
        assert_eq!(e.message, "hunk staged");
        assert_eq!(
            git(&["diff", "--cached", "--name-only", "-z"]),
            b"text-\xff\\name\0"
        );
        assert_eq!(repo.index_bytes(rel).unwrap(), edit);
        pump_hunks(&mut e);
        assert_eq!(e.sign_at(1), None);
        assert!(e.sign_at_staged(1));
        e.feed_text(" gu");
        settle(&mut e, |e| e.git_work.mutation.is_none());
        assert_eq!(e.message, "hunk unstaged");
        assert_eq!(repo.index_bytes(rel).unwrap(), old);
        assert!(git(&["diff", "--cached", "--name-only", "-z"]).is_empty());
    }
}

const NOTE: &str = "one\ntwo\nthree\n";

/// #240: `note` committed as `base`, then rewritten on disk as
/// `worktree` before the editor opens it (a clean buffer, so staging
/// needs no save).
fn note_fixture(base: &str, worktree: &str) -> (tempfile::TempDir, Editor, strop_git::Repo) {
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
    std::fs::write(root.join("note"), base).unwrap();
    git(&["init", "-q"]);
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
    std::fs::write(root.join("note"), worktree).unwrap();
    let mut e = Editor::new(Buffer::open(root.join("note").to_str().unwrap()).unwrap());
    e.discover_git();
    pump_hunks(&mut e);
    let repo = strop_git::Repo::discover(root).unwrap();
    (dir, e, repo)
}

fn index_note(repo: &strop_git::Repo) -> Vec<u8> {
    repo.index_bytes(std::path::Path::new("note")).unwrap()
}

fn caret_row(e: &Editor) -> usize {
    e.buf().line_of(e.head()) + 1
}

/// Feed `keys`, settle the Git mutation, and take its message.
fn mutate(e: &mut Editor, keys: &str) -> String {
    e.feed_text(keys);
    settle(e, |e| e.git_work.mutation.is_none());
    std::mem::take(&mut e.message)
}

#[test]
fn emptied_file_is_marked_and_stageable_from_its_only_row() {
    let (d, mut e, repo) = note_fixture(NOTE, "");
    assert_eq!(e.sign_lines().collect::<Vec<_>>(), [(1, Sign::DeleteAfter)]);
    assert_eq!(mutate(&mut e, " gs"), "hunk staged");
    assert_eq!(index_note(&repo), b"", "the index holds the empty file");
    let listed = Command::new("git")
        .args(["ls-files", "note"])
        .current_dir(d.path())
        .output()
        .unwrap();
    assert_eq!(listed.stdout, b"note\n", "the path stays tracked");
    pump_hunks(&mut e);
    assert_eq!(mutate(&mut e, " gu"), "hunk unstaged");
    assert_eq!(index_note(&repo), NOTE.as_bytes());
}

#[test]
fn emptied_file_discards_and_bounds_hunk_jumps() {
    let (_d, mut e, _repo) = note_fixture(NOTE, "");
    for keys in ["]c", "[c"] {
        e.feed_text(keys);
        assert_eq!(e.message, "no more hunks", "{keys}");
        assert_eq!(caret_row(&e), 1, "{keys}");
        e.message.clear();
    }
    e.feed_text(" gX");
    assert_eq!(e.buf().text().to_string(), NOTE);
    assert_eq!(caret_row(&e), 1);
}

#[test]
fn eof_deletion_marks_the_last_content_row() {
    let (_d, mut e, repo) = note_fixture(NOTE, "one\ntwo\n");
    assert_eq!(e.sign_lines().collect::<Vec<_>>(), [(2, Sign::DeleteAfter)]);
    e.feed_text("]c");
    assert_eq!(caret_row(&e), 2, "]c stops on the last content row");
    assert_eq!(e.buf().line_of(e.head()), e.buf().last_content_line());
    e.feed_text("ggG");
    assert_eq!(mutate(&mut e, " gs"), "hunk staged");
    assert_eq!(index_note(&repo), b"one\ntwo\n");
    pump_hunks(&mut e);
    assert_eq!(mutate(&mut e, " gu"), "hunk unstaged");
    assert_eq!(index_note(&repo), NOTE.as_bytes());
    pump_hunks(&mut e);
    e.feed_text(" gX");
    assert_eq!(e.buf().text().to_string(), NOTE);
}

#[test]
fn replacement_shows_change_and_stages() {
    let (_d, mut e, repo) = note_fixture(NOTE, "one\nTWO\nthree\n");
    assert_eq!(e.sign_lines().collect::<Vec<_>>(), [(2, Sign::Change)]);
    e.feed_text("]c");
    assert_eq!(caret_row(&e), 2);
    e.feed_text("]c");
    assert_eq!(caret_row(&e), 2, "nothing past the only hunk");
    assert_eq!(mutate(&mut e, " gs"), "hunk staged");
    assert_eq!(index_note(&repo), b"one\nTWO\nthree\n");
}

/// #287: discarding a pure deletion puts the caret on the first restored
/// line, wherever the deletion sits.
#[test]
fn discard_of_a_deletion_lands_on_the_restored_lines() {
    const FIVE: &str = "one\ntwo\nthree\nfour\nfive\n";
    for (name, worktree, keys, restored) in [
        ("top", "three\nfour\nfive\n", "gg", 1),
        ("middle", "one\nfour\nfive\n", "2G", 2),
        ("end", "one\ntwo\nthree\n", "G", 4),
    ] {
        let (_d, mut e, _repo) = note_fixture(FIVE, worktree);
        e.feed_text(keys);
        e.feed_text(" gX");
        assert_eq!(e.buf().text().to_string(), FIVE, "{name}");
        assert_eq!(caret_row(&e), restored, "{name}");
    }
}

/// #293: staged hunks are in index lines. After three lines are inserted
/// above a staged change, its mark and `Space g u` follow it to its buffer
/// row, and the row it used to have unstages nothing.
#[test]
fn unstage_finds_the_staged_hunk_below_unstaged_inserts() {
    const TEN: &str = "1\n2\n3\n4\n5\n6\n7\n8\n9\n10\n";
    let (_d, mut e, repo) = note_fixture(TEN, "1\n2\n3\n4\n5\n6\n7\nEIGHT\n9\n10\n");
    e.feed_text("8G");
    assert_eq!(mutate(&mut e, " gs"), "hunk staged");
    pump_hunks(&mut e);
    assert!(e.sign_at_staged(8));
    e.feed_text("ggOa<esc>Ob<esc>Oc<esc>");
    e.feed_text(":w\r");
    settle(&mut e, |e| !e.buf().is_dirty());
    pump_hunks(&mut e);
    assert!(e.sign_at_staged(11), "the mark follows the staged line");
    assert!(!e.sign_at_staged(8), "the row it had shows line 5");
    e.feed_text("8G");
    assert_eq!(mutate(&mut e, " gu"), "no staged hunk here");
    e.feed_text("11G");
    assert_eq!(mutate(&mut e, " gu"), "hunk unstaged");
    assert_eq!(index_note(&repo), TEN.as_bytes());
}
