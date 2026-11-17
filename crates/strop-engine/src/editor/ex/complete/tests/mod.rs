//! Completion, by concern: `split` (where the cursor is), `commands`
//! (the menu cannot lie), `cycle` (Tab and friends), `arguments`
//! (buffers, words, themes, topics), `paths` (listing jobs).

use crate::editor::Editor;
use strop_core::Buffer;

mod alloc;
mod arguments;
mod commands;
mod cycle;

mod split;

fn editor() -> Editor {
    Editor::new(Buffer::from_text("x\n"))
}

/// The menu's row texts after typing `keys`; empty when it is closed.
fn menu(e: &Editor) -> Vec<String> {
    e.cmdline_menu()
        .map(|menu| {
            menu.rows()
                .iter()
                .map(|row| row.text().to_owned())
                .collect()
        })
        .unwrap_or_default()
}
