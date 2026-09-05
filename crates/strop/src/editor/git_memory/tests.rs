
use std::process::Command;

use crate::editor::{Editor, GitJob, Key, Surface};
use strop_core::Buffer;
use strop_git::memory::LogRow;
use strop_git::LineOrigin;

/// Repo with two commits; second adds a line to f.rs.
fn fixture() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t.t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(root.join("f.rs"), "fn a() {}\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "first"]);
    std::fs::write(root.join("f.rs"), "fn a() {}\nfn b() {}\n").unwrap();
    git(&["commit", "-qam", "add b"]);
    let mut e = Editor::new(Buffer::open(root.join("f.rs").to_str().unwrap()).unwrap());
    e.cwd = root.to_path_buf();
    e.discover_git();
    (dir, e)
}

fn pump(e: &mut Editor) {
    // let job threads deliver (bounded, like headless settle)
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    loop {
        e.drain_git_jobs();
        let loaded = e.surface().is_some_and(
            |s| matches!(s, crate::editor::Surface::CommitLog { rows, .. } if !rows.is_empty()),
        );
        if loaded || std::time::Instant::now() > deadline {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

#[test]
fn commit_browser_dives_to_delta() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    let text = e.buf().rope.to_string();
    assert!(text.contains("add b"), "{text}");
    assert!(text.contains("first"), "{text}");
    assert!(e.buf().readonly, "browser is a readonly real buffer");

    // motions work on the surface
    e.feed_text("j");
    // Enter on a commit row → changed files
    e.feed_text("k");
    e.feed(Key::Enter);
    let text = e.buf().rope.to_string();
    assert!(text.contains("commit"), "{text}");
    assert!(text.contains("f.rs"), "{text}");
    assert!(matches!(e.surface(), Some(Surface::ChangedFiles { .. })));

    // Enter on the file row → the diff surface
    e.feed_text("j");
    e.feed_text("j");
    e.feed(Key::Enter);
    let text = e.buf().rope.to_string();
    assert!(text.contains("fn b() {}"), "{text}");
    assert!(text.starts_with("f.rs +1 -0\n"), "{text}");
    assert!(!text.contains("diff --git"), "no raw patch noise: {text}");
    assert!(text.contains("@@ -1,1 +1,2 @@"), "hunk header row: {text}");

    // edits refuse, q climbs out
    e.feed_text("x");
    assert!(e.message.contains("readonly"));
    e.feed_text("q");
    assert!(matches!(e.surface(), Some(Surface::ChangedFiles { .. })));
}

#[test]
fn diff_surface_rows_carry_line_numbers() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed_text("k"); // newest commit is row 0? feed j then k lands on 0
    e.feed(Key::Enter);
    e.feed_text("jj");
    e.feed(Key::Enter);
    let Some(Surface::Diff { hunks, .. }) = e.surface() else {
        panic!("not a diff surface");
    };
    let h = &hunks[0];
    let ctx = h
        .lines
        .iter()
        .find(|l| l.origin == LineOrigin::Context)
        .expect("context line");
    assert_eq!((ctx.old_lineno, ctx.new_lineno), (Some(1), Some(1)));
    let add = h
        .lines
        .iter()
        .find(|l| l.origin == LineOrigin::Addition)
        .expect("addition");
    assert_eq!(add.new_lineno, Some(2));
}

#[test]
fn blame_card_shows_commit() {
    let (_d, mut e) = fixture();
    e.feed_text("j"); // line 2 (fn b)
    e.blame_line();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while e.blame_card.is_none() && std::time::Instant::now() < deadline {
        e.drain_git_jobs();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    let card = e.blame_card.as_ref().expect("blame card");
    assert_eq!(card.summary, "add b");
    assert_eq!(card.author, "t");
}

#[test]
fn permalink_needs_remote() {
    let (_d, e) = fixture();
    // no remote configured → honest refusal
    assert_eq!(e.build_permalink().unwrap_err(), "no remote configured");
}

#[test]
fn permalink_resolves_sha_and_ssh_remote() {
    let (_d, mut e) = fixture();
    let root = e.cwd.clone();
    Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "remote",
            "add",
            "origin",
            "git@github.com:stropdev/strop.git",
        ])
        .output()
        .unwrap();
    e.discover_git();
    e.feed_text("j"); // line 2
    let url = e.build_permalink().expect("permalink");
    assert!(
        url.starts_with("https://github.com/stropdev/strop/blob/"),
        "{url}"
    );
    assert!(url.ends_with("/f.rs#L2"), "{url}");
    assert!(!url.contains("/main/"), "branch must resolve to SHA: {url}");
    e.yank_permalink();
    assert_eq!(e.register(None).0, url);
    assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
}

fn git_out(root: &std::path::Path, args: &[&str]) -> String {
    String::from_utf8_lossy(
        &Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap()
            .stdout,
    )
    .trim()
    .to_string()
}

fn pump_ready(e: &mut Editor, ready: impl Fn(&Editor) -> bool) {
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
    while !ready(e) && std::time::Instant::now() < deadline {
        e.drain_git_jobs();
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

/// `Space g b` toggles a per-buffer gutter; Enter dives into the
/// cursor line's commit, positioned at its sha (0011 §3).
#[test]
fn blame_gutter_toggles_and_dives() {
    let (dir, mut e) = fixture();
    let root = dir.path().to_path_buf();
    e.feed_text(" gb");
    assert_eq!(e.blame_gutters.len(), 1, "gutter on for the buffer");
    pump_ready(&mut e, |e| e.blame_gutter_for(e.first_doc()).is_some());
    let gutter = e
        .blame_gutter_for(e.first_doc())
        .expect("gutter data loaded");
    assert_eq!(gutter.lines.len(), 2, "one blame per file line");
    assert_eq!(
        gutter.lines[0].sha,
        git_out(&root, &["rev-parse", "HEAD~1"])
    );
    assert_eq!(gutter.lines[1].sha, git_out(&root, &["rev-parse", "HEAD"]));

    // cursor on line 1 → Enter dives into "first", landing on its row
    e.feed(Key::Enter);
    pump_ready(&mut e, |e| {
        e.surface().is_some_and(
            |s| matches!(s, crate::editor::Surface::CommitLog { rows, .. } if !rows.is_empty()),
        )
    });
    assert!(
        matches!(e.surface(), Some(Surface::CommitLog { .. })),
        "dive opened the browser"
    );
    assert_eq!(
        e.buf().line_of(e.head()),
        1,
        "cursor on the first-commit row"
    );
    assert_eq!(e.view_top(), 1, "view positioned at the focused sha");
    let text = e.buf().rope.to_string();
    assert!(text.contains("first"), "{text}");

    // q returns; the gutter survives; toggle off removes it
    e.feed_text("q");
    assert_eq!(e.blame_gutters.len(), 1, "gutter is per-buffer view state");
    e.feed_text(" gb");
    assert!(e.blame_gutters.is_empty(), "second toggle turns it off");
    e.feed(Key::Enter);
    assert!(
        !matches!(e.surface(), Some(Surface::CommitLog { .. })),
        "Enter without a gutter stays inert"
    );
}

/// The gutter refuses to dive after edits (stale pairing) and falls
/// back to the single-line card (0011 §3).
#[test]
fn stale_gutter_falls_back_to_card() {
    let (_d, mut e) = fixture();
    e.feed_text(" gb");
    // settle both spawned jobs (gutter + interim card): a sentinel
    // through the same FIFO channel proves everything before it
    // was delivered
    e.git_tx.send(GitJob::Error("\u{0}settled".into())).unwrap();
    pump_ready(&mut e, |e| e.message.contains('\u{0}'));
    e.message.clear();
    e.blame_card = None;
    // edit the buffer: line count changes, epoch bumps. Save so
    // the disk-blame card can speak about the new line at all
    e.feed_text("o");
    e.feed_text("fn c() {}");
    e.feed(Key::Esc);
    e.feed_text(":w<cr>");
    assert!(
        e.blame_gutter_for(e.first_doc()).is_none(),
        "edits void the line↔blame pairing"
    );
    e.blame_card = None;
    e.feed(Key::Enter);
    assert!(
        !matches!(e.surface(), Some(Surface::CommitLog { .. })),
        "no dive from stale data"
    );
    // the card is the fallback: it blames the cursor's own line
    // (wait for the *new* card — the toggle's line-1 card may
    // still be in flight)
    pump_ready(&mut e, |e| {
        e.blame_card.as_ref().is_some_and(|c| c.line == 3)
    });
}

/// The return point restores even when the origin buffer is not
/// the one the close would land on next (0011 §1).
#[test]
fn return_point_restores_when_origin_not_current() {
    let (dir, mut e) = fixture();
    let root = dir.path();
    e.feed_text("j$"); // line 2, end
    let want = e.head();
    e.open_log(false);
    pump(&mut e);
    std::fs::write(root.join("g.rs"), "other\n").unwrap();
    let origin = e.first_doc();
    e.open_buffer(root.join("g.rs").to_str().unwrap()).unwrap();
    assert_ne!(e.current(), origin, "switched away from the log's origin");
    let log_surface = e.mru.iter().copied().find(|&id| {
        e.doc(id)
            .buf
            .name
            .as_deref()
            .is_some_and(|n| n.contains("log"))
    });
    e.view_mut().doc = log_surface.expect("log surface in mru"); // back onto the log surface
    e.set_head(0);
    e.feed_text("q");
    assert_eq!(e.current(), origin, "closing switches back to the origin");
    assert_eq!(e.head(), want, "cursor restored, not line 1");
    assert_eq!(e.buf().line_of(e.head()), 1);
}

/// A log result for a dead surface cannot land in the buffer that
/// recycled its index (0011 §2).
#[test]
fn stale_log_results_are_dropped() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    let stale = e.generation;
    let dead_surface = e.current(); // the log surface's id
    e.feed_text("q"); // closes the surface; generation moves on
    assert_ne!(stale, e.generation);
    e.git_tx
        .send(GitJob::Log {
            buffer: dead_surface,
            generation: stale,
            rows: vec![LogRow {
                text: "POISON ROW".into(),
                sha: None,
            }],
        })
        .unwrap();
    e.drain_git_jobs();
    for (i, (_, d)) in e.docs.iter().enumerate() {
        let text = d.buf.rope.to_string();
        assert!(!text.contains("POISON"), "document {i} clobbered: {text}");
    }
    // the live path still delivers
    e.open_log(false);
    pump(&mut e);
    assert!(e.buf().rope.to_string().contains("add b"));
}

/// A late gutter result for a toggled-off buffer is dropped: the
/// entry is the toggle, not the job (0011 §2).
#[test]
fn gutter_result_dropped_after_toggle_off() {
    let (dir, mut e) = fixture();
    let key = dir.path().join("f.rs").canonicalize().unwrap();
    e.feed_text(" gb"); // on (job in flight)
    e.feed_text(" gb"); // off
    assert!(e.blame_gutters.is_empty());
    e.git_tx
        .send(GitJob::Gutter {
            path: key,
            generation: e.generation, // even a current generation
            lines: vec![strop_git::memory::BlameLine {
                sha: "deadbeef".into(),
                author: "nobody".into(),
                age: "1m".into(),
                ts: 0,
            }],
        })
        .unwrap();
    e.drain_git_jobs();
    assert!(
        e.blame_gutters.is_empty(),
        "a late job must not re-open a closed gutter"
    );
}

/// Two files in one fixture repo; the second commit touches both.
fn multi_file_fixture() -> (tempfile::TempDir, Editor) {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .current_dir(root)
            .output()
            .unwrap();
    };
    git(&["init", "-q"]);
    git(&["config", "user.email", "t@t.t"]);
    git(&["config", "user.name", "t"]);
    std::fs::write(root.join("a.rs"), "one\n").unwrap();
    std::fs::write(root.join("b.rs"), "uno\n").unwrap();
    git(&["add", "."]);
    git(&["commit", "-qm", "base"]);
    std::fs::write(root.join("a.rs"), "one\ntwo\n").unwrap();
    std::fs::write(root.join("b.rs"), "uno\ndos\n").unwrap();
    git(&["commit", "-qam", "touch both"]);
    let mut e = Editor::new(Buffer::open(root.join("a.rs").to_str().unwrap()).unwrap());
    e.cwd = root.to_path_buf();
    e.discover_git();
    (dir, e)
}

/// Dive to a file delta: the surface carries the commit's files,
/// and `]f`/`[f` walk them, wrapping (0011 §4).
#[test]
fn commit_file_nav_walks_files() {
    let (_d, mut e) = multi_file_fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter); // newest commit → changed files
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs → delta
    let (label, files) = match e.surface() {
        Some(Surface::Diff {
            label,
            commit: Some(cf),
            ..
        }) => (label.clone(), cf.files.len()),
        other => panic!("not a commit diff: {other:?}"),
    };
    assert_eq!(label, "a.rs");
    assert_eq!(files, 2, "the sidebar's data rides the surface");

    e.feed_text("]f");
    match e.surface() {
        Some(Surface::Diff { label, .. }) => assert_eq!(label, "b.rs"),
        other => panic!("surface lost: {other:?}"),
    }
    let text = e.buf().rope.to_string();
    assert!(text.starts_with("b.rs +1 -0\n"), "{text}");
    assert!(text.contains("dos"), "{text}");
    assert!(e.message.contains("b.rs · 2/2"), "{}", e.message);

    e.feed_text("[f");
    assert!(
        matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "a.rs"),
        "back to the first file"
    );
    e.feed_text("[f"); // wraparound
    assert!(
        matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "b.rs"),
        "wraparound to the last file"
    );
    assert_eq!(
        e.docs.len(),
        4,
        "]f rewrites the surface in place (no new buffers)"
    );
}

/// Tab hops focus between sidebar and diff; focused j/k steps
/// files (tuicr's model); Enter hops back (0011 §4).
#[test]
fn tab_cycles_focus_between_sidebar_and_diff() {
    let (_d, mut e) = multi_file_fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter); // changed files
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs delta
    assert!(!e.sidebar_focused());

    e.feed(crate::editor::Key::Tab);
    assert!(e.sidebar_focused(), "tab focuses the sidebar");
    e.feed_text("j"); // focused j steps to the next file
    assert!(
        matches!(e.surface(), Some(Surface::Diff { label, .. }) if label == "b.rs"),
        "j stepped to b.rs"
    );
    assert!(e.sidebar_focused(), "focus survives the file step");
    e.feed(crate::editor::Key::Enter);
    assert!(!e.sidebar_focused(), "enter hops back to the diff");
    e.feed(crate::editor::Key::Backtab);
    assert!(e.sidebar_focused(), "shift-tab focuses too");
}

/// `q` in a split closes the pane (buffer stays); the last pane's
/// `q` closes the buffer and restores the origin (0011 §1).
#[test]
fn q_in_split_closes_pane_then_buffer() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::CtrlW);
    e.feed_text("v"); // split: both panes show the log
    assert_eq!(e.panes.len(), 2);
    e.feed_text("q");
    assert_eq!(e.panes.len(), 1, "q closes the pane in a split");
    assert_eq!(e.docs.len(), 2, "the surface buffer survives");
    assert!(
        matches!(e.surface(), Some(Surface::CommitLog { .. })),
        "still on the log"
    );
    e.feed_text("q");
    assert_eq!(e.docs.len(), 1, "the last pane's q closes the buffer");
    assert_eq!(e.current(), e.first_doc(), "back on the origin buffer");
    assert!(e.surface().is_none());
}

/// Golden shape: the blame column renders per line; the commit
/// sidebar renders beside the delta with the current file marked.
#[test]
fn gutters_and_sidebar_render() {
    let (dir, mut e) = fixture();
    let root = dir.path().to_path_buf();
    e.feed_text(" gb");
    pump_ready(&mut e, |e| e.blame_gutter_for(e.first_doc()).is_some());
    let frame = crate::headless::frame_string(&mut e, 100, 10);
    let first_sha = git_out(&root, &["rev-parse", "HEAD~1"]);
    assert!(
        frame.contains(&format!("{} t ", &first_sha[..7])),
        "blame cell: {frame}"
    );
    assert!(
        frame.contains("fn a() {}"),
        "content still renders right of the gutter: {frame}"
    );

    let (_d, mut e) = multi_file_fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter);
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs delta
    let frame = crate::headless::frame_string(&mut e, 100, 12);
    assert!(frame.contains("▌a.rs"), "current file marked: {frame}");
    assert!(frame.contains(" b.rs"), "sibling files listed: {frame}");
    e.feed_text("]f");
    let frame = crate::headless::frame_string(&mut e, 100, 12);
    assert!(frame.contains("▌b.rs"), "marker follows ]f: {frame}");
}
