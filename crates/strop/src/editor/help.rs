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
            self.switch_to(i);
            self.set_head(0);
            self.view_mut().view_top = 0;
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
        text.push_str(concat!(
            "\n[remote]\n",
            "  Space o / :remote       Choose an SSH destination or add a host; Enter connects\n",
            "  - / Backspace           Visit parent directory and retain the selected child\n",
            "  :remote root / home     Browse remote root or negotiated home directory\n",
            "  :remote edit             Verify a full file snapshot and explicitly enable editing\n",
            "  :w / :wq                 Save an authorized remote file; no conflict bypass via !\n",
            "  :remote verify           Reconcile an unconfirmed save without blind overwrite\n",
            "  :tail [BYTES] [URI]     Read a bounded tail (URI defaults to current file)\n",
            "  :range START BYTES URI  Read a byte window; line numbers are window-relative\n",
            "  :follow [URI]           Follow a bounded tail; Escape stops following\n",
            "  :unfollow               Keep the snapshot and stop polling\n",
            "  :browse [URI]           Open a directory; Enter selects an entry or ../\n",
            "  :filter [TEXT]          Filter a directory; no text restores all entries\n",
            "  :remote connect URI     Hold a shared SSH connection explicitly\n",
            "  :remote disconnect URI  Close the endpoint's owned connection\n",
            "  :remote clear           Close all owned connections\n",
            "  :remote list            List authenticated connections in a buffer\n",
            "  :trust                  Authorize the pending endpoint/project command\n",
            "  Tab                     Complete an SSH URI without starting authentication\n",
            "  :e!                     Refresh the remote snapshot and revoke write authority\n",
            "  directory columns       kind, POSIX permissions, server bytes, name; ? = unknown\n",
            "  :explain                 Why: workspaces, LSP readiness/refusals, effective config\n",
            "  ctrl-o in a results list  Open hits as an editable collection (edits write back)\n",
        ));
        let mut buf = Buffer::from_text(&text);
        buf.name = Some("help".into());
        self.push_jump(); // opening help is a jumplist entry
        let id = self.docs.insert(Document::output(buf));
        self.drop_stale_scratch(id);
        self.switch_to(id);
        self.set_head(0);
        self.view_mut().view_top = 0;
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
        let text = e.buf().text().to_string();
        for section in crate::keymap::SECTIONS {
            assert!(text.contains(&format!("[{section}]")), "missing {section}");
        }
        // it's a real buffer: / searches it
        e.feed_text("/undo-tree\r");
        assert!(e.head() > 0, "search moved into the help text");
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
