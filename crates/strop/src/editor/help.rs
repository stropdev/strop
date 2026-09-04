//! `:help` (also `Space ?`): the keybinding table as a real readonly
//! buffer (0001 §4) — `/` searches it, motions walk it, `q` closes it.
//! Replaces the floating keybinds popup: a buffer you can search beats
//! a card you can only scroll.

use strop_core::Buffer;

use super::{Document, Editor};

impl Editor {
    /// Open the generated help buffer (`:help` / `Space ?`).
    pub(crate) fn open_help(&mut self) {
        // already open → switch, don't stack copies
        let existing = self
            .docs
            .iter()
            .find(|(_, d)| d.buf.name.as_deref() == Some("help"))
            .map(|(id, _)| id);
        if let Some(i) = existing {
            self.current = i;
            self.touch_mru(i);
            self.cursor = 0;
            self.view_top = 0;
            return;
        }
        let mut text = String::from("strop help — / searches · q closes\n");
        for section in crate::keymap::SECTIONS {
            text.push_str(&format!("\n[{section}]\n"));
            let rows: Vec<_> = crate::keymap::BINDINGS
                .iter()
                .filter(|b| b.section == *section)
                .collect();
            let width = rows
                .iter()
                .map(|b| b.keys.chars().count())
                .max()
                .unwrap_or(0);
            for b in rows {
                let planned = if b.live { "" } else { "  (soon)" };
                text.push_str(&format!(
                    "  {:<width$}  {}{planned}\n",
                    b.keys,
                    b.desc,
                    width = width
                ));
            }
        }
        let mut buf = Buffer::from_text(&text);
        self.drop_stale_scratch();
        self.push_jump(); // opening help is a jumplist entry
        buf.readonly = true;
        buf.name = Some("help".into());
        let id = self.docs.insert(Document {
            buf,
            highlighter: None,
            surface: None,
        });
        self.current = id;
        self.touch_mru(id);
        self.cursor = 0;
        self.view_top = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn help_buffer_lists_every_section_and_searches() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(":help\r");
        assert_eq!(e.buf().name.as_deref(), Some("help"));
        assert!(e.buf().readonly);
        let text = e.buf().rope.to_string();
        for section in crate::keymap::SECTIONS {
            assert!(text.contains(&format!("[{section}]")), "missing {section}");
        }
        // it's a real buffer: / searches it
        e.feed_text("/undo-tree\r");
        assert!(e.cursor > 0, "search moved into the help text");
        // q closes back to the file
        e.feed_text("q");
        assert_eq!(e.buf().name.as_deref(), None);
    }

    #[test]
    fn space_question_opens_help() {
        let mut e = Editor::new(Buffer::from_text("x\n"));
        e.feed_text(" ?");
        assert_eq!(e.buf().name.as_deref(), Some("help"));
    }
}
