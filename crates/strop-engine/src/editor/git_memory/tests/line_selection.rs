//! Git commands on a Visual selection, through keys (#289).

use super::{set_remote, suppress_native};
use crate::editor::io::native::Operation;
use crate::editor::test_support::git::{fixture, git_out, pump, settle};
use crate::editor::{Editor, Mode, Surface};
use strop_core::frontend_input::Key;

/// The fixture's `f.rs` (two lines) with a GitHub remote: the link
/// prefix every permalink here starts with.
fn with_remote() -> (tempfile::TempDir, Editor, String) {
    let (dir, mut e) = fixture();
    let root = e.cwd.clone();
    set_remote(
        &mut e,
        &root,
        "origin",
        "https://github.com/stropdev/strop.git",
    );
    let sha = git_out(&root, &["rev-parse", "HEAD"]);
    let base = format!("https://github.com/stropdev/strop/blob/{sha}/f.rs");
    (dir, e, base)
}

/// Feed `keys`, then `Space g y`: the copied link, and the editor back
/// in Normal with nothing pending.
fn yank_after(keys: &str) -> (String, Editor) {
    let (_dir, mut e, base) = with_remote();
    e.feed_text(keys);
    e.feed_text(" gy");
    assert_eq!(e.mode, Mode::Normal, "{keys}: Space g y leaves Visual");
    assert!(e.walker.is_ground(), "{keys}: nothing left pending");
    assert_eq!(e.message, "permalink copied", "{keys}");
    let url = e.register(None).text;
    assert_eq!(e.osc52.as_deref(), Some(url.as_str()), "{keys}");
    (url.strip_prefix(&base).unwrap_or(&url).to_owned(), e)
}

#[test]
fn visual_permalink_yanks_the_selected_range() {
    for (keys, anchor) in [
        ("ggVj", "#L1-L2"),    // linewise
        ("ggvj", "#L1-L2"),    // charwise
        ("gg\x16j", "#L1-L2"), // blockwise
        ("GVk", "#L1-L2"),     // reversed
        ("jV", "#L2"),         // one row: no range
        ("ggvl", "#L1"),       // charwise within one row
    ] {
        assert_eq!(yank_after(keys).0, anchor, "{keys:?}");
    }
}

/// A count typed in Visual before the leader is consumed by the row:
/// it neither changes the link nor leaks into the next command.
#[test]
fn a_count_before_the_leader_does_not_leak() {
    let (anchor, mut e) = yank_after("ggVj3");
    assert_eq!(anchor, "#L1-L2");
    e.feed_text("gg");
    e.feed_text("j");
    assert_eq!(e.buf().line_of(e.head()), 1, "j moves one line");
}

#[test]
fn visual_permalink_open_requests_the_browser_once() {
    let (_dir, mut e, base) = with_remote();
    suppress_native(&mut e);
    e.feed_text("ggVj go");
    assert_eq!(e.mode, Mode::Normal);
    let opened: Vec<String> =
        e.io.native_tickets()
            .into_iter()
            .filter_map(|ticket| match ticket.key.operation {
                Operation::Browser { url } => Some(url),
                _ => None,
            })
            .collect();
    assert_eq!(opened, [format!("{base}#L1-L2")]);
    assert!(e.register(None).text.is_empty(), "open never yanks");
    assert!(e.osc52.is_none());
}

/// Visual `Space y` still yanks the selection to the clipboard
/// register and leaves Visual.
#[test]
fn visual_clipboard_yank_leaves_visual() {
    let (_dir, mut e) = fixture();
    e.feed_text("ggVj y");
    assert_eq!(e.mode, Mode::Normal);
    assert_eq!(e.register(Some('+')).text, "fn a() {}\nfn b() {}\n");
}

#[test]
fn visual_line_history_lands_in_normal() {
    let (_dir, mut e) = fixture();
    e.feed_text("GV gh");
    assert_eq!(e.mode, Mode::Normal, "the log opens in Normal");
    assert_eq!(e.buf().name.as_deref(), Some("git log · f.rs:2-2"));
    pump(&mut e);
    let text = e.buf().text().to_string();
    assert!(text.contains("add b"), "{text}");
    e.feed(Key::Enter);
    settle(&mut e, |e| {
        matches!(e.surface(), Some(Surface::ChangedFiles { .. }))
    });
    e.feed_text("q");
    assert!(matches!(e.surface(), Some(Surface::CommitLog { .. })));
    e.feed_text("q");
    assert!(e.surface().is_none(), "q climbs back to the file");
    assert!(e
        .buf()
        .path
        .as_deref()
        .is_some_and(|path| path.ends_with("f.rs")));
    assert_eq!(e.mode, Mode::Normal);
}
