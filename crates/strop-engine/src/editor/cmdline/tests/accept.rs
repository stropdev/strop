//! Running and cancelling: the origin comes back, Visual's range is
//! typed, a pipe keeps its range.

use super::editor;
use crate::editor::cmdline::{Context, Line, Origin};
use crate::editor::Mode;
use strop_core::Range;

#[test]
fn cancelling_restores_the_view_and_incsearch_cursor() {
    let mut e = editor("one\ntwo\nthree\n");
    e.feed_text("/thr");
    assert_eq!(e.buf().line_of(e.head()), 2, "incsearch moved the cursor");
    e.feed_text("<esc>");
    assert!(!e.cmdline.is_open());
    assert_eq!(e.head(), 0, "Esc puts the cursor back");
    e.feed_text("/thr<c-c>");
    assert_eq!(e.head(), 0, "Ctrl-C puts the cursor back");
}

#[test]
fn visual_colon_opens_with_the_range_and_ends_visual() {
    let mut e = editor("a\nb\nc\n");
    e.feed_text("Vj:");
    assert_eq!(e.cmdline.text(), ":'<,'>");
    assert_eq!(e.mode, Mode::Normal);
    e.feed_text("d<cr>");
    assert_eq!(e.buf().text().to_string(), "c\n");
}

#[test]
fn a_pipe_line_returns_its_range_and_literal_command() {
    let mut e = editor("first\nsecond\n");
    let origin = Origin {
        pane_index: e.active_pane,
        pane: e.view().clone(),
        revision: e.buf().revision(),
    };
    let line = Line::with_body(
        Context::Pipe {
            origin,
            range: Range::charwise(0, 13),
            visual: true,
        },
        "cat",
    );
    assert_eq!(line.text(), "|cat");
    assert_eq!(line.body(), "cat");
    match line.context() {
        Context::Pipe { range, visual, .. } => {
            assert_eq!(*range, Range::charwise(0, 13));
            assert!(*visual);
        }
        _ => panic!("lost pipe context"),
    }
    e.cmdline.open(line);
    assert_eq!(e.cmdline_sigil(), Some('|'));
}

#[test]
fn a_line_whose_document_changed_closes_before_its_next_key() {
    let mut e = editor("one\n");
    e.feed_text(":w");
    e.buf_mut().edit().insert(0, "x").unwrap();
    e.feed_text("q");
    assert!(!e.cmdline.is_open());
    assert_eq!(e.message, "input cancelled: document or pane changed");
}
