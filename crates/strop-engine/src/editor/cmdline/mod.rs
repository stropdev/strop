//! The command line (plan 0110): one open line for `:`, `/`, `?` and
//! `|`, edited with Vim's command-line keys. It owns its text plus
//! everything about the moment it opened: the pane (selections and
//! viewport), the document and its revision. `Esc` and `Ctrl-C` cancel
//! at once — the line has no Normal mode; editing a line with the whole
//! grammar is the command-line window.
//!
//! Siblings by concern: `keys` (the key table and the reducer), `accept`
//! (what the editor does with a line: run it, cancel it, incsearch).

use strop_core::{id::BufferRevision, Range};
use strop_picker::LineEdit;

use super::{input::ParserState, panes::Pane};

mod accept;
pub(crate) mod history;
pub(crate) mod keys;
#[cfg(test)]
mod tests;

/// Where a line opened: the whole originating pane (document, complete
/// selections, viewport) and the buffer revision. Cancelling restores
/// this verbatim; running validates it first.
#[derive(Debug, Clone)]
pub(crate) struct Origin {
    pub pane_index: usize,
    pub pane: Pane,
    pub revision: BufferRevision,
}

/// What the line is for. The sigil is derivable; the context is not.
#[derive(Debug, Clone)]
pub(crate) enum Context {
    Ex(Origin),
    Search {
        origin: Origin,
        /// The typed entry state (counts/register/operator) that
        /// survived crossing into the line (`2d/foo` keeps 2 and d).
        state: ParserState,
        backward: bool,
    },
    Pipe {
        origin: Origin,
        range: Range,
        visual: bool,
    },
}

/// One open line: the editable text (sigil first) and its context.
#[derive(Debug, Clone)]
pub(crate) struct Line {
    edit: LineEdit,
    context: Context,
    /// `Ctrl-R` was typed: the next key names a register.
    register: bool,
    /// A walk through history in progress; any edit ends it.
    recall: Option<history::Recall>,
}

impl Line {
    pub(crate) fn new(context: Context) -> Self {
        let sigil = sigil_of(&context);
        let mut buffer = [0u8; 4];
        Self {
            edit: LineEdit::new(sigil.encode_utf8(&mut buffer).to_owned()),
            context,
            register: false,
            recall: None,
        }
    }

    /// A line that opens with text already typed (`:'<,'>` from Visual).
    pub(crate) fn with_body(context: Context, body: &str) -> Self {
        let mut line = Self::new(context);
        line.edit.insert_str(body);
        line
    }

    pub(crate) fn sigil(&self) -> char {
        sigil_of(&self.context)
    }

    /// The text after the sigil: the command or pattern.
    pub(crate) fn body(&self) -> &str {
        &self.edit.text()[1..]
    }

    /// The whole line, sigil included.
    pub(crate) fn text(&self) -> &str {
        self.edit.text()
    }

    /// Caret byte offset into `text()` (sigil-inclusive).
    pub(crate) fn cursor(&self) -> usize {
        self.edit.cursor()
    }

    pub(crate) fn context(&self) -> &Context {
        &self.context
    }

    pub(crate) fn origin(&self) -> &Origin {
        match &self.context {
            Context::Ex(origin) | Context::Search { origin, .. } | Context::Pipe { origin, .. } => {
                origin
            }
        }
    }

    /// The saved search entry, when this is a `/` or `?` line.
    pub(crate) fn search(&self) -> Option<(&Origin, &ParserState)> {
        match &self.context {
            Context::Search { origin, state, .. } => Some((origin, state)),
            _ => None,
        }
    }

    pub(crate) fn backward(&self) -> Option<bool> {
        match &self.context {
            Context::Search { backward, .. } => Some(*backward),
            _ => None,
        }
    }

    /// Replace the body, caret at its end.
    pub(crate) fn set_body(&mut self, body: &str) {
        let at = self.edit.text().len();
        self.edit.delete_range(1, at);
        self.edit.set_cursor(1);
        self.edit.insert_str(body);
    }
}

fn sigil_of(context: &Context) -> char {
    match context {
        Context::Ex(_) => ':',
        Context::Search {
            backward: false, ..
        } => '/',
        Context::Search { backward: true, .. } => '?',
        Context::Pipe { .. } => '|',
    }
}

/// The editor's one command line: open or closed, nothing in between.
#[derive(Debug, Default)]
pub struct Cmdline {
    active: Option<Line>,
    /// What the `:` line remembers between commands.
    pub(crate) ex: super::ex::ExState,
    pub(crate) history: history::Histories,
}

impl Cmdline {
    pub(crate) fn line(&self) -> Option<&Line> {
        self.active.as_ref()
    }

    pub fn is_open(&self) -> bool {
        self.active.is_some()
    }

    /// The whole line, sigil included; empty when no line is open.
    pub fn text(&self) -> &str {
        self.line().map_or("", Line::text)
    }

    /// Caret byte offset into `text()`; 0 when no line is open.
    pub fn cursor(&self) -> usize {
        self.line().map_or(0, Line::cursor)
    }

    /// The open line's sigil (`: / ? |`); None when closed.
    pub fn sigil(&self) -> Option<char> {
        self.line().map(Line::sigil)
    }

    /// Open a line. Opening while another is open is a caller bug: the
    /// previous origin would be silently discarded.
    pub(crate) fn open(&mut self, line: Line) {
        debug_assert!(
            self.active.is_none(),
            "cancel the previous line before opening another"
        );
        self.active = Some(line);
    }

    /// Close the line, handing it back (once).
    pub(crate) fn close(&mut self) -> Option<Line> {
        self.active.take()
    }

    pub(crate) fn line_mut(&mut self) -> Option<&mut Line> {
        self.active.as_mut()
    }

    /// One step through the open line's history; false when there is no
    /// line that way (or no history for this kind of line).
    pub(crate) fn recall(&mut self, older: bool, filtered: bool) -> bool {
        let Some(line) = self.active.as_mut() else {
            return false;
        };
        let Some(history) = self.history.of(line.sigil()) else {
            return false;
        };
        let mut recall = line
            .recall
            .take()
            .unwrap_or_else(|| history::Recall::new(line.body(), line.cursor() - 1));
        let moved = match recall.step(history, older, filtered) {
            Some(history::Step::Line(text)) => {
                line.set_body(text);
                true
            }
            Some(history::Step::Typed) => {
                line.set_body(recall.typed());
                true
            }
            None => false,
        };
        line.recall = Some(recall);
        moved
    }

    /// Remember a line run, in its own list.
    pub(crate) fn remember(&mut self, sigil: char, body: &str) {
        if let Some(history) = self.history.of_mut(sigil) {
            history.push(body);
        }
    }

    /// Vim's `":`: the newest `:` line that is not itself a repeat
    /// (`:@:`, `:@x`).
    pub(crate) fn last_command_line(&self) -> Option<&str> {
        self.history
            .ex
            .iter()
            .rev()
            .find(|line| !super::ex::split_command(line).is_some_and(|named| named.name == "@"))
    }
}
