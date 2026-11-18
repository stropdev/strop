//! `:help` (also `Space ?`): the keybinding table as a real readonly
//! buffer (0001 §4) — `/` searches it, motions walk it, `q` closes it.
//! One buffer in nine sections (0112): the first line names them, each
//! opens under a `━ Title ━` heading, and nothing is ever hidden, so `/`
//! still searches all of help. A buffer you can search beats a card you
//! can only scroll. `page` writes the text and its typed lines,
//! `content` holds the section table and what the binding table and the
//! command registry do not generate.

mod content;
mod page;

pub use page::{Cell, HelpPage, LineKind, Tab};

use strop_core::frontend_input::Key;
use strop_core::Buffer;

use super::document::Document;
use super::Editor;
use content::Source;
use page::{Builder, Entry};

/// A row that is not a planned binding.
fn plain<'a>(cell: Cell, keys: &'a str, desc: &'a str) -> Entry<'a> {
    Entry {
        cell,
        keys,
        desc,
        planned: false,
    }
}

/// One part's rows.
fn part_rows(page: &mut Builder, part: &Source) {
    match part {
        Source::Section(id) => {
            for b in crate::keymap::BINDINGS
                .iter()
                .filter(|b| b.sections.contains(id))
            {
                page.entry(&Entry {
                    cell: Cell::Keys,
                    keys: b.keys,
                    desc: b.desc,
                    planned: !b.live,
                });
            }
        }
        // One row per command, written from the registry (0110 §3.1).
        Source::Family(family) => {
            for (usage, desc) in super::ex::help_rows(*family) {
                page.entry(&plain(Cell::Command, usage, &desc));
            }
        }
        Source::Rows(_, rows) => {
            for row in *rows {
                page.entry(&plain(row.cell, row.keys, row.desc));
            }
        }
        Source::QueryLanguage | Source::LspServers => {
            let rows = match part {
                Source::QueryLanguage => content::query_language(),
                _ => content::lsp_servers(),
            };
            for row in &rows {
                page.entry(&plain(row.cell, &row.keys, &row.desc));
            }
        }
    }
}

/// The help text and its model, section by section (§3.1). A section
/// with one part prints no `─ Title ─` under its heading.
fn build() -> (String, HelpPage) {
    let titles: Vec<&'static str> = content::TABS.iter().map(|tab| tab.title).collect();
    let mut page = Builder::new(&titles);
    for tab in content::TABS {
        page.tab(tab.title);
        for part in tab.parts {
            if tab.parts.len() > 1 {
                page.heading(part.title());
            }
            part_rows(&mut page, part);
        }
    }
    page.finish()
}

/// The line `:help {topic}` lands on (§6): a section's title; a keymap
/// section by id or title; any part's title; a command by any of its
/// names (its row, found by its usage).
fn topic_line(page: &HelpPage, text: &ropey::Rope, topic: &str) -> Option<usize> {
    if let Some(tab) = page.tab_named(topic) {
        return Some(page.tabs[tab].heading);
    }
    let heading = |title: &str| {
        (0..page.lines.len()).find(|&line| {
            page.lines[line] == LineKind::Heading
                && text
                    .line(line)
                    .to_string()
                    .trim_end()
                    .strip_prefix("─ ")
                    .and_then(|rest| rest.strip_suffix(" ─"))
                    .is_some_and(|found| found.eq_ignore_ascii_case(title))
        })
    };
    if let Some(section) = crate::keymap::Section::find(topic) {
        let tab = content::TABS.iter().position(|tab| {
            tab.parts
                .iter()
                .any(|part| matches!(part, Source::Section(id) if *id == section.id))
        })?;
        return heading(section.title).or(Some(page.tabs[tab].heading));
    }
    if let Some(line) = heading(topic) {
        return Some(line);
    }
    let named = super::ex::split_command(topic.trim_start_matches(':'))?;
    (0..page.lines.len()).find(|&line| match &page.lines[line] {
        LineKind::Entry {
            cell: Cell::Command,
            keys,
            ..
        } => text.line(line).to_string().get(keys.clone()) == Some(named.command.usage),
        _ => false,
    })
}

/// The page's section titles, in order: `:help` completion lists them
/// first.
pub(crate) fn section_titles() -> impl Iterator<Item = &'static str> + Clone {
    content::TABS.iter().map(|tab| tab.title)
}

/// The help text with every wrapped description joined back onto its
/// row, for tests that read a phrase.
#[cfg(test)]
pub(crate) fn unwrapped(text: &str) -> String {
    text.replace(&format!("\n{}", " ".repeat(page::DESC_COLUMN)), " ")
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
            self.help_home();
            return;
        }
        let (text, page) = build();
        let mut buf = Buffer::from_text(&text);
        buf.name = Some("help".into());
        debug_assert_eq!(page.lines.len(), buf.last_content_line() + 1);
        // a temporary surface (0051 §7 R07): ctrl-o AND `:q` restore
        // the exact view the user came from
        if self.open_temporary(Document::help(buf, page)).is_some() {
            self.help_home();
        }
    }

    /// The cursor on the first section's name, the page from its top.
    fn help_home(&mut self) {
        let first = self
            .help_page(self.current())
            .and_then(|page| page.tabs.first())
            .map_or(0, |tab| tab.name.start);
        self.set_head(first);
        self.view_mut().view_top = 0;
    }

    pub(crate) fn open_help_topic(&mut self, topic: &str) {
        self.open_help();
        let topic = topic.trim();
        // Identity exhaustion leaves us on the origin buffer: never
        // search/scroll the wrong document (0056 AR13).
        let Some(page) = self.help_page(self.current()) else {
            return;
        };
        if topic.is_empty() {
            return;
        }
        match topic_line(page, self.buf().text(), topic) {
            Some(line) => self.help_land(line),
            None => self.message = format!("no help topic {topic:?}; :help shows the index"),
        }
    }

    /// Keys the help owns (§6): `]]` / `[[` move between sections
    /// (a count repeats), `enter` on the first line opens the section
    /// under the cursor. Every other key is the buffer's.
    pub(crate) fn help_key(&mut self, key: Key) -> bool {
        let Some(page) = self.help_page(self.current()) else {
            return false;
        };
        let line = self.buf().line_of(self.head());
        let column = self.head() - self.buf().line_start(line);
        let opened = page.tabs.get(page.tab_at(0, column)).map(|tab| tab.heading);
        let headings: Vec<usize> = page.tabs.iter().map(|tab| tab.heading).collect();
        let count = self.walker.state.count().unwrap_or(1);
        let target = match key {
            Key::Char(']') if self.walker.at_prefix(&["]"]) => {
                // the count-th heading below the line, or nothing
                let below = headings.iter().filter(|&&heading| heading > line);
                below.take(count).last().copied()
            }
            Key::Char('[') if self.walker.at_prefix(&["["]) => {
                // above the first heading is the top of the page
                let above = headings.iter().filter(|&&heading| heading < line).count();
                Some(above.checked_sub(count).map_or(0, |at| headings[at]))
            }
            Key::Enter if line == 0 && self.walker.is_ground() => opened,
            _ => return false,
        };
        self.walker.clear();
        if let Some(target) = target.filter(|&target| target != line) {
            self.push_jump();
            self.help_land(target);
        }
        true
    }

    /// The cursor to `line`'s start, `line` the top row.
    fn help_land(&mut self, line: usize) {
        self.set_head(self.buf().line_start(line));
        self.view_mut().view_top = line;
    }
}

#[cfg(test)]
mod tests;
