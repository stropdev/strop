//! The command line's behaviour, by concern: `keys` (editing keys and
//! their `?` rows), `accept` (running, cancelling, origins), `history`.

use crate::editor::Editor;
use strop_core::Buffer;

mod accept;
mod errors;
mod history;
mod keys;
mod window;

fn editor(text: &str) -> Editor {
    Editor::new(Buffer::from_text(text))
}

/// The line after typing `keys` (strop key notation) on a fresh editor.
fn line_after(keys: &str) -> String {
    let mut e = editor("one two\n");
    e.feed_text(keys);
    e.cmdline.text().to_owned()
}
