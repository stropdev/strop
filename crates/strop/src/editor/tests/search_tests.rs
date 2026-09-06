//! Search-line contracts (0027 §1, issue 13): live incsearch, the
//! fixed origin, abort semantics, and the overlays that must NOT fire
//! on a plain search.

use super::*;

#[test]
fn incsearch_tracks_typing_and_backspace() {
    // the cursor sits on the pattern's first match from the fixed
    // origin while the pattern grows AND shrinks (the issue-13 repro)
    let mut e = Editor::new(Buffer::from_text("aa foo\nbb foobar\n"));
    e.feed_text("/fo");
    let on_fo = e.head();
    assert_eq!(e.buf().line_of(on_fo), 0, "first match after the origin");
    e.feed_text("ob"); // "/foob" — the only match is on line 2 now
    assert_eq!(e.buf().line_of(e.head()), 1, "longer pattern re-jumps");
    // backspace re-resolves: "/foo" matches line 0 again
    e.feed(Key::Backspace);
    assert_eq!(e.buf().line_of(e.head()), 0, "bs walks the match back");
    assert_eq!(e.head(), on_fo, "same match as typing /fo");
}

#[test]
fn incsearch_parks_at_origin_when_no_match() {
    let mut e = Editor::new(Buffer::from_text("aa foo\nbb\n"));
    let origin = {
        e.feed_text("ll"); // byte 2
        e.head()
    };
    e.feed_text("/zz");
    assert_eq!(e.head(), origin, "no match: cursor never moves");
    e.feed(Key::Enter);
    assert_eq!(e.head(), origin, "Enter on a no-match keeps position");
    assert_eq!(e.message, "pattern not found", "vim's E486 equivalent");
}

#[test]
fn enter_commits_from_the_origin_not_the_jumped_cursor() {
    // origin sits before the first match: incsearch shows it, and
    // Enter must land on THE SHOWN match — resolving from the jumped
    // cursor would skip to the next one
    let mut e = Editor::new(Buffer::from_text("aa foo\nfoo two\n"));
    e.feed_text("/foo");
    let shown = e.head();
    assert_eq!(e.buf().line_of(shown), 0, "incsearch on the first match");
    e.feed(Key::Enter);
    assert_eq!(e.head(), shown, "Enter keeps the match incsearch showed");
    assert_eq!(
        e.last_search.as_ref().map(|ls| ls.pattern.as_str()),
        Some("foo"),
        "last_search commits"
    );
    assert!(e.pending.is_empty(), "line closed");
    assert!(e.search_origin.is_none(), "origin consumed");
}

#[test]
fn aborting_a_search_restores_the_origin() {
    let mut e = Editor::new(Buffer::from_text("aa foo\nbb foo\n"));
    let origin = {
        e.feed_text("ll");
        e.head()
    };
    e.feed_text("/foo");
    let jumped = e.head();
    assert_ne!(jumped, origin, "incsearch jumped");
    e.feed(Key::Esc); // first Esc: modal editing on the line
    assert!(e.pending_normal, "line still open");
    assert_eq!(e.head(), jumped, "still on the match");
    e.feed(Key::Esc); // second Esc: clear — abort restores
    assert_eq!(e.head(), origin, "cursor returns to the origin");
    assert!(e.search_origin.is_none());
}

#[test]
fn backspace_at_the_sigil_cancels_the_search_prompt() {
    let mut editor = Editor::new(Buffer::from_text("foo\n"));
    editor.feed_text("/");
    editor.feed(Key::Backspace);
    assert!(editor.pending.is_empty());
    assert!(editor.search_origin.is_none());
    editor.feed_text("x");
    assert_eq!(
        editor.buf().rope.to_string(),
        "oo\n",
        "next key belongs to normal mode"
    );
}

#[test]
fn backward_incsearch_jumps_backward() {
    let mut e = Editor::new(Buffer::from_text("aa yy\nzz\n"));
    e.feed_text("G$"); // end of the last line
    let origin = e.head();
    e.feed_text("?yy");
    assert_eq!(e.buf().line_of(e.head()), 0, "nearest match before origin");
    e.feed(Key::Backspace);
    e.feed(Key::Backspace); // pattern empty: park at the origin
    assert_eq!(e.head(), origin, "empty pattern parks at the origin");
}

#[test]
fn plain_search_never_previews_and_never_lights_find_candidates() {
    // issue 13's "stale match" look: the old code painted a delete
    // preview (cursor→match) during plain /pat, and lit the leap-style
    // find overlay whenever the pattern ended in f/F/t/T
    let mut e = Editor::new(Buffer::from_text("alpha const void*);\nbeta\n"));
    e.feed_text("/const");
    assert!(e.preview().is_none(), "no operator, no preview");
    assert!(
        e.find_candidates().is_none(),
        "a search pattern is not a pending find"
    );
    e.pending.clear();
    // the walker owns the find state: `df` awaiting its char
    e.feed_text("df");
    assert_eq!(
        e.find_candidates(),
        Some(FindPending {
            ch: 'f',
            backward: false
        }),
        "df awaits its char"
    );
    e.feed_text("t"); // dft completes — candidates gone
    assert!(e.find_candidates().is_none());
}

#[test]
fn search_pattern_reads_only_search_lines() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.pending = "|sed s/a/b/".into();
    assert!(e.search_pattern().is_none(), "a pipe body is not a pattern");
    e.pending = ":w /etc".into();
    assert!(e.search_pattern().is_none(), "an ex body is not a pattern");
    e.pending = "/pat".into();
    assert_eq!(e.search_pattern(), Some("pat"));
    e.pending = "?pat".into();
    assert_eq!(e.search_pattern(), Some("pat"), "? is a search sigil");
    e.pending = "/".into();
    assert!(e.search_pattern().is_none(), "empty pattern");
}

#[test]
fn search_card_covers_backward_search() {
    let mut e = Editor::new(Buffer::from_text("foo\nbar\n"));
    e.feed_text("?ba");
    let frame = crate::headless::frame_string(&mut e, 40, 8);
    assert!(
        frame.contains("search"),
        "the ? line gets the card: {frame}"
    );
    assert!(frame.contains("1 match"), "live count: {frame}");
}

#[test]
fn clipboard_yank_composes_through_the_pending_line() {
    // Space y plants the synthetic `"+y line; the motion completes it
    // through the same per-keystroke resolve (0027 kept that contract)
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text(" yw");
    assert_eq!(e.register(Some('+')).0, "hello ");
    assert!(e.osc52.is_some(), "OSC52 payload staged for the TUI");
    // backspace mid-composition pops the motion text, not the register
    let mut e = Editor::new(Buffer::from_text("hello world\n"));
    e.feed_text(" y");
    e.feed(Key::Backspace);
    assert_eq!(
        e.pending, "\"+",
        "bs pops the motion char; the synthetic line survives"
    );
}

#[test]
fn search_line_edits_at_its_caret_and_reresolves() {
    let mut editor = Editor::new(Buffer::from_text("a fx then fox\n"));
    editor.feed_text("/fox<esc>hi<bs>");
    assert_eq!(editor.pending, "/fx");
    assert_eq!(
        editor.head(),
        2,
        "deleting inside the prompt resolves the shorter match"
    );
    editor.feed_text("o");
    assert_eq!(editor.pending, "/fox");
    assert_eq!(
        editor.head(),
        10,
        "inserting at the same caret restores the longer match"
    );
}

#[test]
fn counted_search_preview_matches_committed_operator_range() {
    let mut editor = Editor::new(Buffer::from_text("a hit b hit c\n"));
    editor.feed_text("2d/hi");
    let preview = editor.preview().unwrap().0;
    assert_eq!(preview, vec![strop_core::Range::charwise(0, 8)]);
    editor.feed(Key::Enter);
    assert_eq!(editor.register(None).0, "a hit b ");
    assert_eq!(editor.buf().rope.to_string(), "hit c\n");
}

#[test]
fn wrapping_incsearch_and_enter_land_on_the_same_match() {
    let mut editor = Editor::new(Buffer::from_text("hit then end\n"));
    editor.feed_text("$");
    editor.feed_text("/hit");
    assert_eq!(editor.head(), 0);
    editor.feed(Key::Enter);
    assert_eq!(editor.head(), 0);
    editor.feed(Key::CtrlO);
    assert_eq!(
        editor.head(),
        11,
        "search origin remains the jumplist entry"
    );
}
