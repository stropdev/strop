//! The in-buffer search budget (plan 2026-11-28 D4): a lower-case `/`
//! search, which smartcase folds, whose only match is the last line of a
//! 500 MB buffer. Past its budget is an error, like `nav_100k /needle`.
//! The pattern arrives as one paste, so the figure is its one preview
//! and the jump: typed key by key, this inline editor would also run a
//! whole-buffer preview per prefix, which the live editor runs off the
//! input path and supersedes.

use std::io;
use std::time::Duration;

use strop_core::frontend_input::Key;
use strop_core::Buffer;

use super::find::within;
use super::{gen_source, Op};
use crate::editor::events::AppEvent;
use crate::editor::Editor;

const LAST_LINE: Duration = Duration::from_secs(1);
/// About 500 MB of source-shaped lines.
const LINES: usize = 7_800_000;

pub(super) fn run() -> io::Result<()> {
    let mut text = gen_source(LINES);
    text.push_str("needle_zzz\n");
    let megabytes = text.len() as f64 / 1e6;
    let mut e = Editor::new(Buffer::from_text(&text));
    drop(text);
    let mut search = Op::new("/needle");
    for _ in 0..3 {
        e.feed_text("gg/");
        search.time(|| {
            e.handle_app_event(AppEvent::Paste("needle_zzz".into()));
            e.feed(Key::Enter);
        });
        if e.buf().line_of(e.head()) != LINES {
            return Err(io::Error::other("search_500mb did not reach the last line"));
        }
    }
    println!("search_500mb ({} lines, {megabytes:.1} MB)", LINES + 1);
    search.report();
    within(&search, LAST_LINE)
}
