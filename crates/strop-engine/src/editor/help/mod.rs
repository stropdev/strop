//! `:help` (also `Space ?`): the keybinding table as a real readonly
//! buffer (0001 §4) — `/` searches it, motions walk it, `q` closes it.
//! Laid out in the keymap's curated section order (0088 U10): a header,
//! then per section a `─ Title ─` rule and one row per binding with the
//! keys in a padded column the renderer paints as a keycap. Replaces
//! the floating keybinds popup: a buffer you can search beats a card you
//! can only scroll. `page` writes the text, `content` holds what the
//! binding table and the command registry do not generate.

mod content;
mod page;

pub use page::{Cell, HelpPage, LineKind};

use strop_core::Buffer;

use super::document::Document;
use super::Editor;
use content::Row;
use page::{Builder, Entry};

fn rows(rows: &[Row]) -> Vec<Entry<'_>> {
    rows.iter()
        .map(|row| Entry {
            cell: row.cell,
            keys: row.keys,
            desc: row.desc,
            planned: false,
        })
        .collect()
}

fn generated(rows: &[content::Generated]) -> Vec<Entry<'_>> {
    rows.iter()
        .map(|row| Entry {
            cell: row.cell,
            keys: &row.keys,
            desc: &row.desc,
            planned: false,
        })
        .collect()
}

/// The help text: the keymap sections, the command families, the query
/// language, the hand-written parts, the LSP servers.
fn build() -> (String, HelpPage) {
    let mut page = Builder::new();
    for section in crate::keymap::SECTIONS {
        let entries: Vec<Entry<'_>> = crate::keymap::BINDINGS
            .iter()
            .filter(|b| b.sections.contains(&section.id))
            .map(|b| Entry {
                cell: page::Cell::Keys,
                keys: b.keys,
                desc: b.desc,
                planned: !b.live,
            })
            .collect();
        page.part(section.title, &entries);
    }
    // One row per command, written from the registry (0110 §3.1).
    for family in super::ex::Family::ALL {
        let commands: Vec<(&str, String)> = super::ex::help_rows(family).collect();
        let entries: Vec<Entry<'_>> = commands
            .iter()
            .map(|(usage, desc)| Entry {
                cell: page::Cell::Command,
                keys: usage,
                desc,
                planned: false,
            })
            .collect();
        page.part(family.title(), &entries);
    }
    page.part("Query language", &generated(&content::query_language()));
    for (title, part) in [
        ("Remote", content::REMOTE),
        ("Search workspace", content::SEARCH_WORKSPACE),
        ("Collections", content::COLLECTIONS),
        ("Indentation", content::INDENTATION),
        ("Review", content::REVIEW),
        ("Documentation", content::DOCUMENTATION),
    ] {
        page.part(title, &rows(part));
    }
    page.part("LSP servers", &generated(&content::lsp_servers()));
    page.finish()
}

impl Editor {
    /// The help page's model when `doc` is the help buffer: "is this
    /// help" is this accessor everywhere.
    pub fn help_page(&self, doc: strop_core::id::DocumentId) -> Option<&HelpPage> {
        self.document(doc).and_then(Document::help_page)
    }

    /// Open the generated help buffer (`:help` / `Space ?`).
    pub(crate) fn open_help(&mut self) {
        // already open → switch, don't stack copies
        let existing = self
            .docs
            .iter()
            .find(|(_, d)| d.help_page().is_some())
            .map(|(id, _)| id);
        if let Some(i) = existing {
            self.push_jump();
            self.switch_to(i);
            self.set_head(0);
            self.view_mut().view_top = 0;
            return;
        }
        let (text, page) = build();
        let mut buf = Buffer::from_text(&text);
        buf.name = Some("help".into());
        debug_assert_eq!(page.lines.len(), buf.last_content_line() + 1);
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from
        let _ = self.open_temporary(Document::help(buf, page));
    }

    pub(crate) fn open_help_topic(&mut self, topic: &str) {
        self.open_help();
        // Identity exhaustion leaves us on the origin buffer: never
        // search/scroll the wrong document (0056 AR13).
        if self.help_page(self.current()).is_none() {
            return;
        }
        let topic = topic.trim();
        if topic.is_empty() {
            return;
        }
        // `:help git` reaches a section by id or title, case-insensitively
        let title = crate::keymap::Section::find(topic)
            .map(|section| section.title.to_owned())
            .unwrap_or_else(|| topic.to_owned());
        let heading = format!("─ {title} ─");
        let mut line = self.buf().text().lines().position(|line| {
            line.chars()
                .take_while(|&c| c != '\n' && c != '\r')
                .collect::<String>()
                .eq_ignore_ascii_case(&heading)
        });
        // `:help write`, `:help :w`: the command's own row
        if line.is_none() {
            if let Some(named) = super::ex::split_command(topic.trim_start_matches(':')) {
                let usage = format!("   {} ", named.command.usage);
                line = self
                    .buf()
                    .text()
                    .lines()
                    .position(|line| line.to_string().starts_with(&usage));
            }
        }
        if let Some(line) = line {
            self.set_head(self.buf().line_start(line));
            self.view_mut().view_top = line;
        } else {
            self.message = format!("no help topic {topic:?}; :help shows the index");
        }
    }
}

#[cfg(test)]
mod tests;
