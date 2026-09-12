//! `:help` (also `Space ?`): the keybinding table as a real readonly
//! buffer (0001 §4) — `/` searches it, motions walk it, `q` closes it.
//! Replaces the floating keybinds popup: a buffer you can search beats
//! a card you can only scroll.

use strop_core::Buffer;

use super::Editor;

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
            self.push_jump();
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
                .filter(|b| b.sections.contains(section))
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
            "\n",
            "\n[query]\n",
            "  One local-workspace qualifier language for Space f / Space / / Space R\n",
            "  language:rust           Include a language family (alias: language:rs)\n",
            "  scope                   Regular local files; directory links and special files are not searched\n",
            "  SSH/container listings  Hidden entries included; no ignore-file filtering (local search defaults do not apply)\n",
            "  path:src/               Literal substring in the workspace-relative path\n",
            "  glob:\"**/*.rs\"        Explicit path glob; quote values with spaces\n",
            "  -language: -glob: -path:  Exclude; exclusions win\n",
            "  hidden:include|exclude  Show/hide dotfiles (default: shown)\n",
            "  ignored:include|exclude Include/respect ignored files (default: respected)\n",
            "  case:smart|sensitive|ignore  Case behavior of the search expression\n",
            "  text:\"...\"             Explicit literal content expression (default for bare words)\n",
            "  regex:\"...\"            Explicit Rust-regex expression\n",
            "  ctrl-space              Manual qualifier/language/value suggestions\n",
            "  filter-only grep        Needs a search expression; file find lists matches\n",
            "  quoted text             Stays literal — never a filter (\"language:rust\" searches that text)\n",
            "  old -t / -g flags       Now ordinary literal text; use language: and glob:\n",
            "  file expressions        Bare words are fuzzy; quoted/text: are literal; regex: is explicit\n",
            "  quotes                  Single/double quotes; escape the active quote or backslash\n",
            "  With                    Always literal replacement text; $1 is not a capture expansion\n",
            "  ctrl-o in a results list  Open hits as an editable collection (edits write back)\n",
            "\n[collections]\n",
            "  Ctrl-O                  Collect the complete, currently listed results\n",
            "  + / -                   Expand/contract the current excerpt's context\n",
            "  ]e / [e                 Next/previous excerpt; ]f / [f move file cards\n",
            "  g<Space>                Open the real source; Ctrl-O restores the collection view\n",
            "  u / Ctrl-R              Scoped source undo/redo; typing is live and one Insert group\n",
            "  :w / :wq                Save source files, never the generated view\n",
            "\n[indentation]\n",
            "  :tab-size [N|auto]      Source width, 1-16; no argument opens the selector\n",
            "  :indent-style [spaces|tabs|auto]  Source style or configured/detected policy\n",
            "  Detection              Bounded open/reload sample; ambiguity keeps the configured fallback\n",
            "\n[review]\n",
            "  Enter in Replace        Prepare the exact review, without changing or saving sources\n",
            "  :apply-change           Apply to source buffers; results remain dirty until saved\n",
            "  :cancel-change          Cancel and restore the query/draft/exclusions\n",
            "  :save-change            Save files from the most recent applied operation\n",
            "  :undo-change            Undo verified targets; conflicted receipt members remain recoverable\n",
            "\n[documentation]\n",
            "  Enter on hover          Open full searchable documentation; Ctrl-O returns\n",
            "  :containers               Attach to a running container (read-only browse)\n",
        ));
        text.push_str("\nSupported language filters:\n  ");
        for (index, language) in strop_core::languages::language_names().enumerate() {
            if index > 0 {
                text.push_str(if index % 8 == 0 { "\n  " } else { " " });
            }
            text.push_str(language);
        }
        text.push('\n');
        let mut buf = Buffer::from_text(&text);
        buf.name = Some("help".into());
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from
        self.open_temporary_output(buf);
    }

    pub(crate) fn open_help_topic(&mut self, topic: &str) {
        self.open_help();
        let topic = topic.trim();
        if topic.is_empty() {
            return;
        }
        let heading = format!("[{topic}]");
        let line = self.buf().text().lines().position(|line| {
            line.chars()
                .take_while(|&c| c != '\n' && c != '\r')
                .eq(heading.chars())
        });
        if let Some(line) = line {
            self.set_head(self.buf().line_start(line));
            self.view_mut().view_top = line;
        } else {
            self.message = format!("no help topic {topic:?}; :help shows the index");
        }
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
