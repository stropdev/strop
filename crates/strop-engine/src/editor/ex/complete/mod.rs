//! Completion on the command line (plan 0110 §3.2). The line is split
//! once at the cursor (`split`): a command name lists commands, ranked
//! so the top row is what `Enter` would run (`rank`); an argument lists
//! what its command declares — files and directories (`paths`, a worker
//! job), buffers, words, themes, help topics (`arguments`).
//!
//! The menu opens by itself while a command name is typed and on `Tab`
//! or `Ctrl-D` anywhere; nothing is selected until `Tab`. `Tab` writes
//! the selected row into the line and, past the last row, puts back what
//! was typed. Everything here runs when the line changes; the renderer
//! only reads [`Completion`].

mod arguments;
pub(crate) mod paths;
mod rank;
pub(crate) mod split;
#[cfg(test)]
mod tests;

use super::table::Arg;
use crate::editor::Editor;
use split::Target;

/// Row text: borrowed from the registry, or owned (paths, buffers).
#[derive(Debug, Clone)]
pub(crate) enum Text {
    Static(&'static str),
    Owned(Box<str>),
}

impl Text {
    pub(crate) fn as_str(&self) -> &str {
        match self {
            Self::Static(text) => text,
            Self::Owned(text) => text,
        }
    }
}

impl Default for Text {
    fn default() -> Self {
        Self::Static("")
    }
}

/// One menu row.
#[derive(Debug, Clone)]
pub struct Row {
    /// What `Tab` writes into the line.
    pub(crate) text: Text,
    /// What the menu shows, when not `text` (a path's last segment).
    pub(crate) label: Option<Text>,
    /// The shortest abbreviation (commands) or the buffer number.
    pub(crate) abbrev: Text,
    pub(crate) doc: Text,
    /// The Normal key that does the same.
    pub(crate) key: Option<&'static str>,
    /// Matched characters of the label, as a bitmask of the first 64.
    pub(crate) matched: u64,
    /// A buffer with unsaved changes.
    pub(crate) mark: bool,
}

impl Row {
    pub fn text(&self) -> &str {
        self.text.as_str()
    }
    pub fn label(&self) -> &str {
        self.label.as_ref().unwrap_or(&self.text).as_str()
    }
    pub fn abbrev(&self) -> &str {
        self.abbrev.as_str()
    }
    pub fn doc(&self) -> &str {
        self.doc.as_str()
    }
    pub fn key(&self) -> Option<&'static str> {
        self.key
    }
    /// Whether the label's `index`-th character matched what was typed.
    pub fn matched(&self, index: usize) -> bool {
        index < 64 && self.matched & (1 << index) != 0
    }
    pub fn modified(&self) -> bool {
        self.mark
    }
}

/// What the menu lists: decides its title and its columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MenuKind {
    #[default]
    Commands,
    Paths,
    Buffers,
    Words,
    Themes,
    Topics,
}

/// The menu's state: refilled in place as the line changes.
#[derive(Debug, Default)]
pub struct Completion {
    open: bool,
    kind: MenuKind,
    title: Text,
    note: Option<Box<str>>,
    rows: Vec<Row>,
    /// The body bytes the rows replace.
    span: (usize, usize),
    /// What was typed there, put back after the last row.
    typed: String,
    selected: Option<usize>,
    /// How the command on the line is typed (`:w[!] [file]`).
    usage: Option<&'static str>,
    pub(crate) listing: Option<paths::Listing>,
    /// `Tab` asked for a path listing: select its first row when it lands.
    pub(crate) select_on_arrival: bool,
    scratch: rank::Scratch,
}

impl Completion {
    /// Whether the menu shows (open, with rows).
    pub fn is_open(&self) -> bool {
        self.open && !self.rows.is_empty()
    }
    pub fn kind(&self) -> MenuKind {
        self.kind
    }
    pub fn title(&self) -> &str {
        self.title.as_str()
    }
    pub fn note(&self) -> Option<&str> {
        self.note.as_deref()
    }
    pub fn rows(&self) -> &[Row] {
        &self.rows
    }
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }
    /// How the command on the line is typed, when one is named.
    pub fn usage(&self) -> Option<&'static str> {
        self.usage
    }
    /// The body offset the menu's rows start at (it hangs under it).
    pub fn anchor(&self) -> usize {
        self.span.0
    }

    pub(crate) fn close(&mut self) {
        self.open = false;
        self.selected = None;
        self.rows.clear();
    }

    /// The line closed: forget everything.
    pub(crate) fn reset(&mut self) {
        self.close();
        self.usage = None;
        self.listing = None;
        self.select_on_arrival = false;
        self.note = None;
    }

    /// Show `kind` rows (already in `self.rows`) for `[start, end)`.
    fn show(&mut self, kind: MenuKind, title: Text, body: &str, start: usize, end: usize) {
        self.open = true;
        self.kind = kind;
        self.title = title;
        self.span = (start, end);
        if self.typed.capacity() == 0 {
            self.typed.reserve(64);
        }
        self.typed.clear();
        self.typed.push_str(&body[start..end]);
        self.selected = None;
    }
}

/// `Tab`-family steps through the menu.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Step {
    Next,
    Previous,
    /// `Ctrl-D`: open without selecting.
    List,
    /// `Ctrl-Y`: keep the selected row, close.
    Accept,
    /// `Ctrl-E`: put back what was typed, close.
    Dismiss,
}

/// After every edit of a `:` line: the menu follows what is typed.
pub(crate) fn typed(editor: &mut Editor) {
    refresh(editor, false);
}

/// A row is selected: `Up`/`Down` move in the menu, not in history.
pub(crate) fn selected(editor: &Editor) -> bool {
    editor.cmdline.completion.is_open() && editor.cmdline.completion.selected.is_some()
}

/// One `Tab`-family key. False when it does nothing here (`Ctrl-E`
/// with nothing selected is End; `Ctrl-Y` with nothing selected).
pub(crate) fn step(editor: &mut Editor, step: Step) -> bool {
    if editor.cmdline.sigil() != Some(':') {
        return false;
    }
    let completion = &mut editor.cmdline.completion;
    match step {
        Step::Accept | Step::Dismiss if completion.selected.is_none() => return false,
        Step::Accept => {
            completion.close();
            return true;
        }
        Step::Dismiss => {
            apply(editor, None);
            editor.cmdline.completion.close();
            return true;
        }
        Step::List => {
            editor.cmdline.completion.select_on_arrival = false;
            refresh(editor, true);
            return true;
        }
        Step::Next | Step::Previous => {}
    }
    if !completion.is_open() {
        completion.select_on_arrival = true;
        refresh(editor, true);
        if !editor.cmdline.completion.is_open() {
            return true; // nothing to list, or a listing is on its way
        }
    }
    let completion = &editor.cmdline.completion;
    let last = completion.rows.len() - 1;
    let next = match (completion.selected, step == Step::Next) {
        (None, true) => Some(0),
        (None, false) => Some(last),
        (Some(at), true) => (at < last).then_some(at + 1),
        (Some(at), false) => at.checked_sub(1),
    };
    apply(editor, next);
    // one match completes the word; the next Tab starts afresh (a
    // directory descends)
    if editor.cmdline.completion.rows.len() == 1 && next.is_some() {
        editor.cmdline.completion.close();
    }
    true
}

/// Write row `selected` (or what was typed) into the line.
fn apply(editor: &mut Editor, selected: Option<usize>) {
    let (line, completion) = editor.cmdline.parts_mut();
    let Some(line) = line else {
        return;
    };
    let (start, end) = completion.span;
    let text = match selected {
        Some(at) => completion.rows[at].text.as_str(),
        None => completion.typed.as_str(),
    };
    line.replace_body(start, end, text);
    completion.span.1 = start + text.len();
    completion.selected = selected;
}

/// Recompute the menu from the line. `explicit` is `Tab`/`Ctrl-D`:
/// it opens a menu that typing alone does not (arguments, an empty
/// name) and fetches a path listing.
fn refresh(editor: &mut Editor, explicit: bool) {
    let Some(line) = editor.cmdline.line().filter(|line| line.sigil() == ':') else {
        return;
    };
    let target = split::target(line.body(), line.cursor() - 1);
    let usage = split::command(line.body()).map(|command| command.usage);
    editor.cmdline.completion.usage = usage;
    let was_open = editor.cmdline.completion.open;
    let kind = editor.cmdline.completion.kind;
    match target {
        Target::None => editor.cmdline.completion.close(),
        Target::Name { start, end } => {
            let (line, completion) = editor.cmdline.parts_mut();
            let Some(line) = line else { return };
            let body = line.body();
            let word = &body[start..end];
            if word.is_empty() && !explicit && !was_open {
                completion.close();
                return;
            }
            rank::commands(word, &mut completion.rows, &mut completion.scratch);
            completion.show(
                MenuKind::Commands,
                Text::Static("commands"),
                body,
                start,
                end,
            );
        }
        Target::Arg {
            command,
            arg,
            start,
            end,
        } => {
            let menu = match arg {
                Arg::File | Arg::Directory => MenuKind::Paths,
                Arg::Buffer => MenuKind::Buffers,
                Arg::Theme => MenuKind::Themes,
                Arg::HelpTopic => MenuKind::Topics,
                _ => MenuKind::Words,
            };
            if menu != MenuKind::Paths && !explicit && !(was_open && kind == menu) {
                editor.cmdline.completion.close();
                return;
            }
            if menu == MenuKind::Paths {
                editor.path_complete(command, arg == Arg::Directory, start, end, explicit);
            } else {
                arguments::rows(editor, arg, start, end);
            }
        }
    }
}
