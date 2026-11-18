//! The help page's model (0112 §5): the text and one [`LineKind`] per
//! line, written together by one [`Builder`], so the renderer reads a
//! row's shape instead of guessing it from the text.

use std::ops::Range;

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
    /// The first line: `strop {version}` and how to use the page; the
    /// byte ranges of its key runs.
    Hints {
        keys: Vec<Range<usize>>,
    },
    /// `─ Title ─`.
    Heading,
    /// A row: what its key cell is, the cell's bytes (unpadded).
    Entry {
        cell: Cell,
        keys: Range<usize>,
        planned: bool,
    },
    Blank,
}

/// The help page beside its buffer: one kind per buffer line.
#[derive(Debug, Clone, Default)]
pub struct HelpPage {
    pub lines: Vec<LineKind>,
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
}

impl Builder {
    pub fn new() -> Self {
        let mut builder = Self {
            text: String::new(),
            page: HelpPage::default(),
        };
        let hints = format!(
            "strop {} · help · / search · q close",
            env!("CARGO_PKG_VERSION")
        );
        let search = hints.find("/ search").map_or(0..0, |at| at..at + 1);
        let close = hints.find("q close").map_or(0..0, |at| at..at + 1);
        builder.line(
            &hints,
            LineKind::Hints {
                keys: vec![search, close],
            },
        );
        builder
    }

    fn line(&mut self, text: &str, kind: LineKind) {
        self.text.push_str(text);
        self.text.push('\n');
        self.page.lines.push(kind);
    }

    /// A part: its rule, then its rows with the keys padded to the
    /// widest cell.
    pub fn part(&mut self, title: &str, rows: &[Entry<'_>]) {
        self.line("", LineKind::Blank);
        self.line(&format!("─ {title} ─"), LineKind::Heading);
        let width = rows
            .iter()
            .map(|row| row.keys.chars().count())
            .max()
            .unwrap_or(0);
        for row in rows {
            let soon = if row.planned { "  (soon)" } else { "" };
            let line = format!("   {:<width$}   {}{soon}", row.keys, row.desc);
            self.line(
                &line,
                LineKind::Entry {
                    cell: row.cell,
                    keys: 3..3 + row.keys.len(),
                    planned: row.planned,
                },
            );
        }
    }

    /// The finished text and its model.
    pub fn finish(self) -> (String, HelpPage) {
        (self.text, self.page)
    }
}
