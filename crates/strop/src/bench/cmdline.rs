//! The command line's per-key cost (plan 0110 §4): on a 100k-line buffer,
//! each key of `:%s/foo/bar/g` (the preview resolves off the input path)
//! and `:wri` with three Tabs, each key followed by a full frame.
//! Measurement, like the other buffer scenarios.

use strop_core::frontend_input::Key;
use strop_core::Buffer;

use super::{gen_source, Op};
use crate::editor::Editor;

pub(super) fn run() {
    let text = gen_source(100_000).replace("value", "foo");
    let mut substitute = Op::new("key+frame :%s");
    let mut complete = Op::new("key+frame :wri tab");
    for _ in 0..4 {
        let mut e = Editor::new(Buffer::from_text(&text));
        for key in ":%s/foo/bar/g".chars() {
            substitute.time(|| {
                e.feed(Key::Char(key));
                let _ = crate::headless::frame_string(&mut e, 120, 40);
            });
        }
        e.feed(Key::Esc);
        let keys = ":wri"
            .chars()
            .map(Key::Char)
            .chain([Key::Tab, Key::Tab, Key::Tab]);
        for key in keys {
            complete.time(|| {
                e.feed(key);
                let _ = crate::headless::frame_string(&mut e, 120, 40);
            });
        }
        e.feed(Key::Esc);
    }
    println!("cmdline (100k lines, 120x40)");
    substitute.report();
    complete.report();
}
