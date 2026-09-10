use std::process::Command;

use crate::editor::io::native::{NativeResult, Operation};
use crate::editor::io::IoEvent;
use crate::editor::permalink::{PermalinkIntent, PermalinkOutcome};
use crate::editor::test_support::git::{
    fixture, git_out, multi_file_fixture, pump, pump_ready, settle,
};
use crate::editor::{Editor, GitJob, Key, Surface};
use strop_core::worker::{Completion, FailureKind, Outcome};
use strop_core::Buffer;
use strop_git::memory::LogRow;
use strop_git::LineOrigin;
use strop_trace::replay::Tape;

#[test]
fn commit_browser_dives_to_delta() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    pump(&mut e);
    let text = e.buf().text().to_string();
    assert!(text.contains("add b"), "{text}");
    assert!(text.contains("first"), "{text}");
    assert!(e.buf().readonly, "browser is a readonly real buffer");

    // motions work on the surface
    e.feed_text("j");
    // Enter on a commit row → changed files (the dive fetches async)
    e.feed_text("k");
    e.feed(Key::Enter);
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    let text = e.buf().text().to_string();
    assert!(text.contains("commit"), "{text}");
    assert!(text.contains("f.rs"), "{text}");
    assert!(matches!(e.surface(), Some(Surface::ChangedFiles { .. })));

    // Enter on the file row → the diff surface
    e.feed_text("j");
    e.feed_text("j");
    e.feed(Key::Enter);
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::Diff { .. }))
    });
    let text = e.buf().text().to_string();
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
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed_text("jj");
    e.feed(Key::Enter);
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::Diff { .. }))
    });
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
    settle(&mut e, |e| e.git.is_some());
    e.feed_text("j"); // line 2 (fn b)
    e.blame_line();
    settle(&mut e, |e| e.blame_card.is_some());
    let card = e.blame_card.as_ref().expect("blame card");
    assert_eq!(card.summary, "add b");
    assert_eq!(card.author, "t");
}

#[test]
fn permalink_needs_remote() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    // no remote configured → honest refusal
    assert_eq!(
        e.build_permalink(PermalinkIntent::Yank).unwrap_err(),
        "no remote configured"
    );
}

/// Configure a remote, then refresh the cached context so the pure
/// builder sees it.
fn set_remote(e: &mut Editor, root: &std::path::Path, name: &str, url: &str) {
    Command::new("git")
        .args([
            "-C",
            &root.display().to_string(),
            "remote",
            "add",
            name,
            url,
        ])
        .output()
        .unwrap();
    // an explicit discovery refreshes the cached context's remotes
    e.discover_git();
    settle(e, |e| {
        e.git
            .as_ref()
            .is_some_and(|c| c.remotes.iter().any(|(n, _)| n == name))
    });
}

/// A fixture tape suppresses native launches (hermetic: no real ssh,
/// no real HOME, no network) while recording the request, so tests
/// answer with a crafted completion through the production handler.
fn suppress_native(e: &mut Editor) {
    e.tape = std::rc::Rc::new(Tape::fixture(|_, _| {
        Err(std::io::Error::other("unexpected native observation"))
    }));
}

fn ready_url(e: &Editor) -> String {
    match e.build_permalink(PermalinkIntent::Yank) {
        Ok(PermalinkOutcome::Url(url)) => url,
        Ok(PermalinkOutcome::Alias(_)) => panic!("expected a ready URL, got an alias"),
        Err(message) => panic!("expected a permalink: {message}"),
    }
}

/// The reviewer's defect (0033 finding 1): the whole FQDN authority
/// and the nested repository path must reach the link verbatim.
#[test]
fn permalink_preserves_fqdn_authority_and_nested_repo_path() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    let root = e.cwd.clone();
    set_remote(
        &mut e,
        &root,
        "origin",
        "https://bbgithub.dev.bloomberg.com/acme/nested/demo.git",
    );
    e.feed_text("j"); // line 2
    let sha = git_out(&root, &["rev-parse", "HEAD"]);
    let expected =
        format!("https://bbgithub.dev.bloomberg.com/acme/nested/demo/blob/{sha}/f.rs#L2");
    assert_eq!(ready_url(&e), expected);
    e.yank_permalink();
    assert_eq!(e.register(None).text, expected);
    assert_eq!(e.osc52.as_deref(), Some(expected.as_str()));
}

#[test]
fn permalink_pins_sha_and_yanks() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    let root = e.cwd.clone();
    set_remote(
        &mut e,
        &root,
        "origin",
        "https://github.com/stropdev/strop.git",
    );
    e.feed_text("j"); // line 2
    let sha = git_out(&root, &["rev-parse", "HEAD"]);
    let url = ready_url(&e);
    assert_eq!(
        url,
        format!("https://github.com/stropdev/strop/blob/{sha}/f.rs#L2")
    );
    assert!(!url.contains("/main/"), "branch must resolve to SHA: {url}");
    e.yank_permalink();
    assert_eq!(e.register(None).text, url);
    assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
}

/// An SSH alias remote resolves through the IO worker: OpenSSH
/// answers, and only then does the copy happen — exactly once, with
/// the alias's effective host.
#[test]
fn permalink_ssh_alias_resolves_on_io_worker() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    let root = e.cwd.clone();
    set_remote(&mut e, &root, "origin", "git@bbgithub.alias:acme/demo.git");
    e.feed_text("j"); // line 2

    suppress_native(&mut e);
    e.yank_permalink();
    assert!(
        e.register(None).text.is_empty(),
        "nothing is copied before OpenSSH answers"
    );
    assert!(
        e.osc52.is_none(),
        "no clipboard payload before OpenSSH answers"
    );

    let tickets = e.io.native_tickets();
    assert_eq!(tickets.len(), 1, "one ssh -G evaluation in flight");
    let sha = git_out(&root, &["rev-parse", "HEAD"]);
    e.handle_io(IoEvent::Native(Box::new(Completion {
        ticket: tickets.into_iter().next().unwrap(),
        outcome: Outcome::Success(NativeResult::SshHost("bbgithub.dev.bloomberg.com".into())),
    })));
    let expected = format!("https://bbgithub.dev.bloomberg.com/acme/demo/blob/{sha}/f.rs#L2");
    assert_eq!(e.register(None).text, expected);
    assert_eq!(e.osc52.as_deref(), Some(expected.as_str()));
}

/// A failed or unresolved alias publishes nothing — no guessed URL in
/// the register, no clipboard payload — and the failure says which
/// alias and why.
#[test]
fn permalink_alias_failure_publishes_nothing() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    let root = e.cwd.clone();
    set_remote(&mut e, &root, "origin", "git@bbgithub:acme/demo.git");
    e.feed_text("j");

    suppress_native(&mut e);
    e.yank_permalink();
    let ticket = e.io.native_tickets().pop().unwrap();
    e.handle_io(IoEvent::Native(Box::new(Completion {
        ticket,
        outcome: Outcome::failed(
            FailureKind::Exit,
            "ssh alias \"bbgithub\": cannot run ssh: not found",
        ),
    })));
    assert!(e.message.contains("bbgithub"), "{}", e.message);
    assert!(e.message.contains("ssh"), "{}", e.message);
    assert!(e.register(None).text.is_empty(), "no guessed URL is copied");
    assert!(e.osc52.is_none(), "no clipboard payload on failure");
}

/// `space g o` on an alias: the browser is only requested after
/// OpenSSH answers, and open never touches the register.
#[test]
fn permalink_alias_open_waits_for_resolution() {
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    let root = e.cwd.clone();
    set_remote(&mut e, &root, "origin", "git@bbgithub:acme/demo.git");
    e.feed_text("j");

    suppress_native(&mut e);
    e.open_permalink();
    let ticket = e.io.native_tickets().pop().unwrap();
    e.handle_io(IoEvent::Native(Box::new(Completion {
        ticket,
        outcome: Outcome::Success(NativeResult::SshHost("bbgithub.dev.bloomberg.com".into())),
    })));
    assert!(
        e.io.native_tickets()
            .iter()
            .any(|ticket| matches!(ticket.key.operation, Operation::Browser { .. })),
        "the opener request follows the resolved URL"
    );
    assert!(
        e.register(None).text.is_empty(),
        "open never touches the register"
    );
    assert!(e.osc52.is_none(), "open never stages a clipboard payload");
}

/// `Space g b` toggles a per-buffer gutter; Enter dives into the
/// cursor line's commit, positioned at its sha (0011 §3).
#[test]
fn blame_gutter_toggles_and_dives() {
    let (dir, mut e) = fixture();
    let root = dir.path().to_path_buf();
    settle(&mut e, |e| e.git.is_some());
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
        e.surface()
            .is_some_and(|s| matches!(s, Surface::CommitLog { rows, .. } if !rows.is_empty()))
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
    let text = e.buf().text().to_string();
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
    settle(&mut e, |e| e.git.is_some());
    e.feed_text(" gb");
    // settle the gutter AND the interim card request: both must be
    // terminal before the edit voids the pairing
    pump_ready(&mut e, |e| {
        e.blame_gutter_for(e.first_doc()).is_some() && e.card_request.is_none()
    });
    e.message.clear();
    e.blame_card = None;
    // edit the buffer: line count changes, revision bumps. Save so
    // the disk-blame card can speak about the new line at all
    e.feed_text("o");
    e.feed_text("fn c() {}");
    e.feed(Key::Esc);
    e.feed_text(":w<cr>");
    pump_ready(&mut e, |e| !e.doc(e.first_doc()).buf.dirty);
    assert!(
        e.blame_gutter_for(e.first_doc()).is_none(),
        "edits void the line↔blame pairing"
    );
    e.blame_card = None;
    assert_eq!(
        e.buf().line_of(e.head()),
        1,
        "the inserted line remains current after saving"
    );
    e.feed(Key::Enter);
    assert!(
        !matches!(e.surface(), Some(Surface::CommitLog { .. })),
        "no dive from stale data"
    );
    // the card is the fallback: it blames the cursor's own line
    pump_ready(&mut e, |e| {
        e.blame_card.as_ref().is_some_and(|c| c.line == 2)
    });
}

/// The return point restores even when the origin buffer is not
/// the one the close would land on next (0011 §1).
#[test]
fn return_point_restores_when_origin_not_current() {
    let (dir, mut e) = fixture();
    let root = dir.path();
    settle(&mut e, |e| e.git.is_some());
    e.feed_text("j$"); // line 2, end
    let want = e.head();
    e.open_log(false);
    pump(&mut e);
    std::fs::write(root.join("g.rs"), "other\n").unwrap();
    let origin = e.first_doc();
    e.open_fixture(&root.join("g.rs")).unwrap();
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
/// recycled its index (0011 §2) — the request registry is the gate.
#[test]
fn stale_log_results_are_dropped() {
    let (_d, mut e) = fixture();
    e.open_log(false);
    let dead_surface = e.current(); // the log surface's id
    let stale_ticket = e
        .log_requests
        .get(&dead_surface)
        .cloned()
        .expect("request registered");
    e.feed_text("q"); // closes the surface
    e.handle_git_job(GitJob::Log(strop_core::worker::Completion {
        ticket: stale_ticket,
        outcome: strop_core::worker::Outcome::Success(vec![LogRow {
            text: "POISON ROW".into(),
            sha: None,
        }]),
    }));
    for (i, (_, d)) in e.docs.iter().enumerate() {
        let text = d.buf.text().to_string();
        assert!(!text.contains("POISON"), "document {i} clobbered: {text}");
    }
    // the live path still delivers
    e.open_log(false);
    pump(&mut e);
    assert!(e.buf().text().to_string().contains("add b"));
}

/// A late gutter result for a toggled-off buffer is dropped: the
/// entry is the toggle, not the job (0011 §2).
#[test]
fn gutter_result_dropped_after_toggle_off() {
    let (_dir, mut e) = fixture();
    let key = e.current();
    settle(&mut e, |e| e.git.is_some());
    e.feed_text(" gb"); // on (job in flight)
    let ticket = e
        .blame_gutters
        .get(&key)
        .and_then(|g| g.request.clone())
        .expect("gutter owns its request");
    e.feed_text(" gb"); // off: entry removed, request cancelled
    assert!(e.blame_gutters.is_empty());
    e.handle_git_job(GitJob::Gutter(strop_core::worker::Completion {
        ticket,
        outcome: strop_core::worker::Outcome::Success(vec![strop_git::memory::BlameLine {
            sha: "deadbeef".into(),
            author: "nobody".into(),
            age: "1m".into(),
            ts: 0,
        }]),
    }));
    assert!(
        e.blame_gutters.is_empty(),
        "a late job must not re-open a closed gutter"
    );
}

/// Dive to a file delta: the surface carries the commit's files,
/// and `]f`/`[f` walk them, wrapping (0011 §4).
#[test]
fn commit_file_nav_walks_files() {
    let (_d, mut e) = multi_file_fixture();
    e.open_log(false);
    pump(&mut e);
    e.feed(Key::Enter); // newest commit → changed files
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs → delta
    settle(&mut e, |e| {
        matches!(
            e.surface(),
            Some(Surface::Diff {
                commit: Some(_),
                ..
            })
        )
    });
    let (label, files) = match e.surface() {
        Some(Surface::Diff {
            hunks,
            commit: Some(cf),
            ..
        }) => (hunks.label().to_owned(), cf.files.len()),
        other => panic!("not a commit diff: {other:?}"),
    };
    assert_eq!(label, "a.rs");
    assert_eq!(files, 2, "the sidebar's data rides the surface");

    e.feed_text("]f");
    settle(
        &mut e,
        |e| matches!(e.surface(), Some(Surface::Diff { hunks, .. }) if hunks.label() == "b.rs"),
    );
    let text = e.buf().text().to_string();
    assert!(text.starts_with("b.rs +1 -0\n"), "{text}");
    assert!(text.contains("dos"), "{text}");
    assert!(e.message.contains("b.rs · 2/2"), "{}", e.message);

    e.feed_text("[f");
    settle(
        &mut e,
        |e| matches!(e.surface(), Some(Surface::Diff { hunks, .. }) if hunks.label() == "a.rs"),
    );
    e.feed_text("[f"); // wraparound
    settle(
        &mut e,
        |e| matches!(e.surface(), Some(Surface::Diff { hunks, .. }) if hunks.label() == "b.rs"),
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
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed_text("jj");
    e.feed(Key::Enter); // a.rs delta
    settle(&mut e, |e| {
        matches!(
            e.surface(),
            Some(Surface::Diff {
                commit: Some(_),
                ..
            })
        )
    });
    assert!(!e.sidebar_focused());

    e.feed(crate::editor::Key::Tab);
    assert!(e.sidebar_focused(), "tab focuses the sidebar");
    e.feed_text("j"); // focused j steps to the next file
    settle(
        &mut e,
        |e| matches!(e.surface(), Some(Surface::Diff { hunks, .. }) if hunks.label() == "b.rs"),
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

#[test]
fn hunk_discard_undoes_byte_exact() {
    // 0020 §7: discard creates a committed undo step; u restores the
    // exact pre-discard worktree text
    let (_d, mut e) = fixture();
    settle(&mut e, |e| e.git.is_some());
    // the edit happens IN the editor (live buffer = the worktree state)
    e.feed_text("Gofn c() {}");
    e.feed(crate::editor::Key::Esc);
    let before = e.buf().text().to_string();
    e.refresh_hunks();
    // the gutter is async: pump the job like the event loop
    settle(&mut e, |e| {
        !matches!(e.hunk_load, strop_core::worker::Load::Running(_))
    });
    assert!(!e.hunks.is_empty(), "the worktree edit shows as a hunk");
    e.undo_hunk();
    assert_eq!(e.buf().text().to_string(), "fn a() {}\nfn b() {}\n");
    // u restores the exact pre-discard text
    e.feed_text("u");
    assert_eq!(e.buf().text().to_string(), before);
    // and redo discards again
    e.feed(crate::editor::Key::CtrlR);
    assert_eq!(e.buf().text().to_string(), "fn a() {}\nfn b() {}\n");
}

#[cfg(unix)]
mod native_paths;
mod ownership;
