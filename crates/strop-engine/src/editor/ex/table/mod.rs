//! The command registry (plan 0110 §4 step 1): one row per command, the
//! only source for dispatch, completion, usage, descriptions and the
//! `?` rows. Aliases fold into their command's row (`ls`, `buffers`,
//! `files` are one command with three names). Files by family, in Vim's
//! precedence order where abbreviations overlap (`:s` is substitute, not
//! `:set`): `text`, `buffers`, `files`, `own`.

use super::ExCall;
use crate::editor::Editor;

mod buffers;
mod files;
mod own;
#[cfg(test)]
mod tests;
mod text;

/// What a range before the command means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Addr {
    /// The command takes none; typing one is an error.
    None,
    /// Buffer lines, checked against the buffer before the command runs.
    Lines,
    /// A number that is not a line (`:3b`).
    Count,
}

/// What follows a command's name: the completion kind of its argument.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Arg {
    None,
    File,
    Directory,
    Buffer,
    Theme,
    HelpTopic,
    /// Another command line (`:cdo`, and `:g/pat/` after its pattern).
    Command,
    /// A fixed vocabulary, one line each.
    Words(&'static [(&'static str, &'static str)]),
    Register,
    Mark,
    Pattern,
    Shell,
    Text,
}

/// Which `?` group a command is listed under.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Family {
    Text,
    Buffers,
    Files,
    Own,
}

impl Family {
    pub(crate) const ALL: [Family; 4] = [Self::Text, Self::Buffers, Self::Files, Self::Own];

    /// The `?` heading: `─ Command line: text ─`.
    pub(crate) const fn title(self) -> &'static str {
        match self {
            Self::Text => "Command line: text",
            Self::Buffers => "Command line: buffers",
            Self::Files => "Command line: files and windows",
            Self::Own => "Command line: strop",
        }
    }
}

/// One spelling of a command and the shortest abbreviation Vim accepts
/// for it, in bytes (`:s`, `:norm`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct Name {
    pub name: &'static str,
    pub min: usize,
}

/// `name` reached from its first `min` bytes.
pub(crate) const fn name(name: &'static str, min: usize) -> Name {
    Name { name, min }
}

pub(crate) type Run = fn(&mut Editor, &ExCall<'_>) -> super::ExResult;

#[derive(Clone, Copy)]
pub(crate) struct ExCommand {
    /// The primary name first, then its aliases.
    pub names: &'static [Name],
    pub range: Addr,
    /// Whether `!` may follow the name.
    pub bang: bool,
    /// The command changes the buffer's text: a readonly buffer refuses.
    pub edits: bool,
    pub arg: Arg,
    /// How it is typed: `:w[!] [file]`.
    pub usage: &'static str,
    /// What it does, one line.
    pub doc: &'static str,
    pub family: Family,
    /// The Normal key that does the same; a test runs both and compares.
    pub key: Option<&'static str>,
    pub run: Run,
}

impl std::fmt::Debug for ExCommand {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, ":{}", self.name())
    }
}

/// A command is its primary name.
impl PartialEq for ExCommand {
    fn eq(&self, other: &Self) -> bool {
        self.name() == other.name()
    }
}

impl Eq for ExCommand {}

/// A row; the family files set the family and the builders the rest.
pub(crate) const fn command(
    names: &'static [Name],
    usage: &'static str,
    doc: &'static str,
    run: Run,
) -> ExCommand {
    ExCommand {
        names,
        range: Addr::None,
        bang: false,
        edits: false,
        arg: Arg::None,
        usage,
        doc,
        family: Family::Text,
        key: None,
        run,
    }
}

impl ExCommand {
    /// The primary name (`write` for `:w`).
    pub(crate) const fn name(&self) -> &'static str {
        self.names[0].name
    }
    pub(crate) const fn lines(mut self) -> Self {
        self.range = Addr::Lines;
        self
    }
    pub(crate) const fn count(mut self) -> Self {
        self.range = Addr::Count;
        self
    }
    pub(crate) const fn bang(mut self) -> Self {
        self.bang = true;
        self
    }
    pub(crate) const fn edits(mut self) -> Self {
        self.edits = true;
        self
    }
    pub(crate) const fn arg(mut self, arg: Arg) -> Self {
        self.arg = arg;
        self
    }
    pub(crate) const fn key(mut self, key: &'static str) -> Self {
        self.key = Some(key);
        self
    }
    pub(crate) const fn family(mut self, family: Family) -> Self {
        self.family = family;
        self
    }

    /// The name this typed word abbreviates, if it abbreviates one.
    pub(crate) fn abbreviated(&self, word: &str) -> Option<&'static str> {
        self.names
            .iter()
            .find(|name| word.len() >= name.min && name.name.starts_with(word))
            .map(|name| name.name)
    }
}

/// The command a typed word names, and the name it abbreviates: the
/// first row with a name it abbreviates.
pub(crate) fn lookup(word: &str) -> Option<(&'static ExCommand, &'static str)> {
    if word.is_empty() {
        return None;
    }
    COMMANDS
        .iter()
        .find_map(|command| Some((command, command.abbreviated(word)?)))
}

/// The command with exactly this name.
pub(crate) fn exact_name(word: &str) -> Option<(&'static ExCommand, &'static str)> {
    COMMANDS.iter().find_map(|command| {
        let name = command.names.iter().find(|name| name.name == word)?;
        Some((command, name.name))
    })
}

/// The `?` rows of one family: usage, and the description with the
/// command's other names (`list the buffers (also :buffers :files)`).
pub(crate) fn help_rows(family: Family) -> impl Iterator<Item = (&'static str, String)> {
    COMMANDS
        .iter()
        .filter(move |command| command.family == family)
        .map(|command| {
            let mut desc = command.doc.to_owned();
            if command.names.len() > 1 {
                desc.push_str(" (also");
                for alias in &command.names[1..] {
                    desc.push_str(" :");
                    desc.push_str(alias.name);
                }
                desc.push(')');
            }
            (command.usage, desc)
        })
}

pub(super) fn optional(arg: &str) -> Option<&str> {
    (!arg.is_empty()).then_some(arg)
}

const TOTAL: usize = text::ROWS.len() + buffers::ROWS.len() + files::ROWS.len() + own::ROWS.len();

/// Every command, family by family; the order is the precedence.
pub(crate) static COMMANDS: [ExCommand; TOTAL] =
    concat([text::ROWS, buffers::ROWS, files::ROWS, own::ROWS]);

const fn concat(parts: [&[ExCommand]; 4]) -> [ExCommand; TOTAL] {
    let mut out = [PLACEHOLDER; TOTAL];
    let mut at = 0;
    let mut part = 0;
    while part < parts.len() {
        let rows = parts[part];
        let mut row = 0;
        while row < rows.len() {
            out[at] = rows[row];
            at += 1;
            row += 1;
        }
        part += 1;
    }
    out
}

const PLACEHOLDER: ExCommand = command(&[], "", "", |_, _| Ok(()));
