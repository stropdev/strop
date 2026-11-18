//! The help page's model (0112 §5): the text and one [`LineKind`] per
//! line, written together by one [`Builder`], so the renderer reads a
//! row's shape instead of guessing it from the text. The layout is
//! fixed (§3.2): a 24-cell key column and a 76-cell measure, so nothing
//! is cut at 80 columns and the text never depends on the pane.

use std::ops::Range;

/// The key column's width in cells.
pub const KEY_COLUMN: usize = 24;
/// No line is wider than this.
pub const MEASURE: usize = 76;
/// A row's indent before the key cell.
const INDENT: usize = 2;
/// Cells between the key column and the description.
const GAP: usize = 3;
/// Where a description starts.
pub const DESC_COLUMN: usize = INDENT + KEY_COLUMN + GAP;
/// The description's width.
const DESC_WIDTH: usize = MEASURE - DESC_COLUMN;
/// The suffix a planned row's description ends with.
const SOON: &str = "  (soon)";

/// What a row's key cell holds, so the renderer knows how to read it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Cell {
    /// Keys as typed (`space f`, `ctrl-w h / j`).
    Keys,
    /// A command's usage (`:bd[elete][!] [number]`).
    Command,
    /// A query filter (`language:rust`).
    Filter,
    /// Words, not typed (`scope`, a language name).
    Label,
}

/// What one line of the help is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LineKind {
    /// The first line: every section's name.
    Tabs,
    /// `strop {version}` and how to use the page; the byte ranges of
    /// its key runs.
    Hints {
        keys: Vec<Range<usize>>,
    },
    /// `━ Title ━`: a section of the page.
    TabHeading,
    /// `─ Title ─`: a part of a section.
    Heading,
    /// A row: what its key cell is, the cell's bytes (unpadded).
    Entry {
        cell: Cell,
        keys: Range<usize>,
        planned: bool,
    },
    /// A description's wrapped rest.
    Continuation {
        planned: bool,
    },
    Blank,
}

/// One section of the page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub title: &'static str,
    /// The name's bytes on the first line.
    pub name: Range<usize>,
    /// The line of its `━ Title ━` heading.
    pub heading: usize,
}

/// The help page beside its buffer: one kind per buffer line.
#[derive(Debug, Clone, Default)]
pub struct HelpPage {
    pub tabs: Vec<Tab>,
    pub lines: Vec<LineKind>,
}

impl HelpPage {
    /// The section that holds `line` (§4.1): on the first line the name
    /// at or left of the byte, above the first heading the first.
    pub fn tab_at(&self, line: usize, byte_in_line: usize) -> usize {
        let held = |tab: &Tab| {
            if line == 0 {
                tab.name.start <= byte_in_line
            } else {
                tab.heading <= line
            }
        };
        self.tabs.iter().rposition(held).unwrap_or(0)
    }

    /// The section titled `name`, case-insensitively.
    pub fn tab_named(&self, name: &str) -> Option<usize> {
        self.tabs
            .iter()
            .position(|tab| tab.title.eq_ignore_ascii_case(name.trim()))
    }
}

/// Cells a help string takes: every glyph it holds is one cell wide.
fn cells(text: &str) -> usize {
    text.chars().count()
}

/// `text` in lines of at most `width` cells, broken at spaces; a word
/// longer than `width` stays whole on its own line.
fn wrap(text: &str, width: usize) -> Vec<&str> {
    let mut lines = Vec::new();
    let mut rest = text;
    while cells(rest) > width {
        let mut cut = None;
        for (count, (at, c)) in rest.char_indices().enumerate() {
            if count > width {
                break;
            }
            if c == ' ' {
                cut = Some(at);
            }
        }
        let Some(at) = cut.filter(|&at| at > 0).or_else(|| rest.find(' ')) else {
            break;
        };
        lines.push(&rest[..at]);
        rest = rest[at..].trim_start_matches(' ');
    }
    lines.push(rest);
    lines
}

/// One row of a part.
pub(super) struct Entry<'a> {
    pub cell: Cell,
    pub keys: &'a str,
    pub desc: &'a str,
    pub planned: bool,
}

/// Writes the help text and its model together.
pub(super) struct Builder {
    text: String,
    page: HelpPage,
    /// The current section has a part already.
    part_open: bool,
}

impl Builder {
    /// The first lines: the section names, the hints, a blank.
    pub fn new(titles: &[&'static str]) -> Self {
        let mut builder = Self {
            text: String::new(),
            page: HelpPage::default(),
            part_open: false,
        };
        let mut row = String::from(" ");
        for (at, title) in titles.iter().enumerate() {
            if at > 0 {
                row.push_str("  ");
            }
            let start = row.len();
            row.push_str(title);
            builder.page.tabs.push(Tab {
                title,
                name: start..row.len(),
                heading: 0,
            });
        }
        builder.line(&row, LineKind::Tabs);
        let hints = format!(
            " strop {} · ]] [[ section · enter open · / search · q close",
            env!("CARGO_PKG_VERSION")
        );
        // each phrase and how many of its bytes are keys
        let keys = [("]] [[ ", 5), ("enter ", 5), ("/ ", 1), ("q ", 1)]
            .iter()
            .filter_map(|&(phrase, len)| hints.find(phrase).map(|at| at..at + len))
            .collect();
        builder.line(&hints, LineKind::Hints { keys });
        builder.line("", LineKind::Blank);
        builder
    }

    fn line(&mut self, text: &str, kind: LineKind) {
        debug_assert!(cells(text) <= MEASURE, "{text:?} is wider than the measure");
        self.text.push_str(text);
        self.text.push('\n');
        self.page.lines.push(kind);
    }

    /// The next section's heading. Sections come in the order of the
    /// titles [`Builder::new`] was given.
    pub fn tab(&mut self, title: &str) {
        let at = self
            .page
            .tabs
            .iter()
            .position(|tab| tab.title == title && tab.heading == 0);
        debug_assert!(at.is_some(), "{title} is not a section of the page");
        if self.page.lines.len() > 3 {
            self.line("", LineKind::Blank);
        }
        if let Some(at) = at {
            self.page.tabs[at].heading = self.page.lines.len();
        }
        self.line(&format!("━ {title} ━"), LineKind::TabHeading);
        self.line("", LineKind::Blank);
        self.part_open = false;
    }

    /// A part's `─ Title ─` heading, a blank before all but the first.
    pub fn heading(&mut self, title: &str) {
        if self.part_open {
            self.line("", LineKind::Blank);
        }
        self.part_open = true;
        self.line(&format!("─ {title} ─"), LineKind::Heading);
    }

    /// A row: the key cell padded to the column, the description
    /// wrapped under the measure; a cell wider than the column takes
    /// its own line.
    pub fn entry(&mut self, entry: &Entry<'_>) {
        self.part_open = true;
        let mut desc = wrap(entry.desc, DESC_WIDTH);
        let soon_fits = desc
            .last()
            .is_some_and(|last| cells(last) + cells(SOON) <= DESC_WIDTH);
        if entry.planned && !soon_fits {
            desc.push(SOON.trim_start());
        }
        let last = desc.len() - 1;
        let soon = |at: usize| {
            if entry.planned && soon_fits && at == last {
                SOON
            } else {
                ""
            }
        };
        let keys = INDENT..INDENT + entry.keys.len();
        let kind = LineKind::Entry {
            cell: entry.cell,
            keys,
            planned: entry.planned,
        };
        let indent = " ".repeat(INDENT);
        let mut rest = &desc[..];
        if cells(entry.keys) <= KEY_COLUMN {
            let line = format!(
                "{indent}{:<KEY_COLUMN$}{}{}{}",
                entry.keys,
                " ".repeat(GAP),
                desc[0],
                soon(0)
            );
            self.line(line.trim_end(), kind);
            rest = &desc[1..];
        } else {
            self.line(&format!("{indent}{}", entry.keys), kind);
        }
        let first = desc.len() - rest.len();
        for (at, text) in rest.iter().enumerate() {
            let line = format!("{}{text}{}", " ".repeat(DESC_COLUMN), soon(first + at));
            self.line(
                &line,
                LineKind::Continuation {
                    planned: entry.planned,
                },
            );
        }
    }

    /// The finished text and its model.
    pub fn finish(self) -> (String, HelpPage) {
        debug_assert!(self.page.tabs.iter().all(|tab| tab.heading > 0));
        (self.text, self.page)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wrap_breaks_at_spaces_and_keeps_long_words_whole() {
        assert_eq!(wrap("one two three", 7), ["one two", "three"]);
        assert_eq!(wrap("one two three", 13), ["one two three"]);
        assert_eq!(wrap("abcdefghij kl", 4), ["abcdefghij", "kl"]);
        assert_eq!(wrap("", 4), [""]);
    }
}
