//! Command-line history (plan 0110 §3.2): two lists, `:` and `/ ?`, of
//! the lines run, newest last, 200 each, kept per project in the
//! session file. `Up`/`Down` walk the lines that start with what was
//! typed before the cursor; `Ctrl-P`/`Ctrl-N` walk them all.

use std::collections::VecDeque;

/// How many lines each list keeps.
pub(crate) const CAP: usize = 200;

/// One list, oldest first.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub(crate) struct History {
    lines: VecDeque<Box<str>>,
}

impl History {
    /// Remember a line run: an equal older line moves to the newest
    /// place; the oldest falls off past [`CAP`].
    pub(crate) fn push(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        if let Some(at) = self.lines.iter().position(|old| &**old == line) {
            self.lines.remove(at);
        }
        if self.lines.len() == CAP {
            self.lines.pop_front();
        }
        self.lines.push_back(line.into());
    }

    pub(crate) fn len(&self) -> usize {
        self.lines.len()
    }

    pub(crate) fn get(&self, index: usize) -> Option<&str> {
        self.lines.get(index).map(|line| &**line)
    }

    /// Oldest first.
    pub(crate) fn iter(&self) -> impl DoubleEndedIterator<Item = &str> {
        self.lines.iter().map(|line| &**line)
    }

    /// The list from its saved form, newest last, cut to [`CAP`].
    pub(crate) fn from_lines(lines: Vec<String>) -> Self {
        let mut history = Self::default();
        for line in lines {
            history.push(&line);
        }
        history
    }

    /// The nearest line from `from` (exclusive) in `older` or newer
    /// direction that starts with `prefix`.
    fn find(&self, from: usize, older: bool, prefix: &str) -> Option<usize> {
        if older {
            (0..from)
                .rev()
                .find(|&at| self.lines[at].starts_with(prefix))
        } else {
            (from + 1..self.lines.len()).find(|&at| self.lines[at].starts_with(prefix))
        }
    }
}

/// The `:` list and the `/ ?` list.
#[derive(Debug, Default)]
pub(crate) struct Histories {
    pub ex: History,
    pub search: History,
}

impl Histories {
    /// The list a line with this sigil reads and writes; a pipe has none.
    pub(crate) fn of(&self, sigil: char) -> Option<&History> {
        match sigil {
            ':' => Some(&self.ex),
            '/' | '?' => Some(&self.search),
            _ => None,
        }
    }

    pub(crate) fn of_mut(&mut self, sigil: char) -> Option<&mut History> {
        match sigil {
            ':' => Some(&mut self.ex),
            '/' | '?' => Some(&mut self.search),
            _ => None,
        }
    }
}

/// A walk through history from an open line: what was typed when it
/// began (restored past the newest line) and where it is now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Recall {
    /// The body as typed when the walk began.
    typed: String,
    /// The part of it before the cursor: the filter of `Up`/`Down`.
    prefix: usize,
    /// The history line shown; `None` is the typed text.
    at: Option<usize>,
}

impl Recall {
    pub(crate) fn new(typed: &str, cursor: usize) -> Self {
        Self {
            typed: typed.to_owned(),
            prefix: cursor.min(typed.len()),
            at: None,
        }
    }

    /// One step: the body to show, or `None` when there is no line that
    /// way (the line stays as it is).
    pub(crate) fn step<'h>(
        &mut self,
        history: &'h History,
        older: bool,
        filtered: bool,
    ) -> Option<Step<'h>> {
        let prefix = if filtered {
            &self.typed[..self.prefix]
        } else {
            ""
        };
        let from = self.at.unwrap_or(history.len());
        match history.find(from, older, prefix) {
            Some(at) => {
                self.at = Some(at);
                history.get(at).map(Step::Line)
            }
            // newer than the newest: back to what was typed
            None if !older && self.at.is_some() => {
                self.at = None;
                Some(Step::Typed)
            }
            None => None,
        }
    }

    pub(crate) fn typed(&self) -> &str {
        &self.typed
    }
}

/// Where a history step landed.
#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Step<'h> {
    Line(&'h str),
    Typed,
}
