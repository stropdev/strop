//! The `:` line (plan 0095 §2.6): a range, a command name looked up in
//! one table, and one function per command family. Siblings by concern:
//! `range` (addresses), `table` (the vocabulary, which completion and
//! dispatch both read), `lines` (`:d :y :m :t :j :> :pu`), `sort`,
//! `global` (`:g :v :normal`), `substitute`, `buffers`, `listing`
//! (`:ls :reg :marks`), `files` (`:w :q :e` families), `directory`
//! (`:cd`), `tabs` (tab pages), `complete` (the menu: what completes
//! where).

mod buffers;
pub(crate) mod complete;
mod directory;
mod files;
mod global;
mod lines;
mod listing;
pub(crate) mod range;
mod sort;
pub(crate) mod substitute;
pub(crate) mod table;
mod tabs;
#[cfg(test)]
mod tests;

pub use substitute::{PreviewLine, SubstitutePreviewState, SubstitutionPreview};
pub(crate) use table::{help_rows, Family};

// The commands are functions over the editor, not methods of it (0087 A4:
// `Editor` does not grow); they see each other through this module.
pub(crate) use buffers::*;
pub(crate) use directory::*;
pub(crate) use files::*;
pub(crate) use global::*;
pub(crate) use lines::*;
pub(crate) use listing::*;
pub(crate) use range::*;
pub(crate) use sort::*;
pub(crate) use substitute::*;
pub(crate) use tabs::*;

use table::{Addr, ExCommand};

use crate::editor::Editor;

/// The command line's surface on the editor: what input, the renderer and
/// the other tracks call. Everything else in this module is a function.
impl Editor {
    /// Run one command line (the text after the `:`).
    pub(crate) fn run_ex(&mut self, cmdline: &str) {
        run_ex(self, cmdline);
    }

    /// Type `keys` as Normal-mode input: once at the cursor, or with a
    /// range (zero-based, inclusive lines) on each line with the cursor
    /// at its start. One undo step. `:cdo norm …` composes with this.
    pub fn ex_normal(&mut self, range: Option<(usize, usize)>, keys: &str) {
        ex_normal(self, range, keys);
    }

    /// Normal `&` (`:s`) and `g&` (`:%s//~/&`): the last substitute
    /// again, on the cursor line or the whole buffer, with or without
    /// its flags.
    pub fn repeat_substitute(&mut self, keep_flags: bool, whole_buffer: bool) {
        repeat_substitute(self, keep_flags, whole_buffer);
    }

    /// `@:`: the last command line again, `count` times.
    pub fn repeat_command_line(&mut self, count: usize) {
        if let Err(error) = repeat_command_line(self, count) {
            self.fail(error);
        }
    }

    /// The live `:s` preview: the plan Enter would apply.
    pub fn substitute_preview(&self) -> Option<SubstitutePreviewState<'_>> {
        substitute_preview(self)
    }

    /// The zero-based lines the open line's range names, or why it names
    /// none (`mark not set`); `None` while no range is typed.
    pub fn cmdline_range(&self) -> Option<Result<(usize, usize), &str>> {
        self.cmdline
            .parsed
            .range
            .as_ref()
            .map(|range| range.as_ref().map(|lines| *lines).map_err(String::as_str))
    }

    /// How the command on the open line is typed (`:w[!] [file]`).
    pub fn cmdline_usage(&self) -> Option<&'static str> {
        self.cmdline.completion.usage()
    }

    /// The command line's menu, when it shows.
    pub fn cmdline_menu(&self) -> Option<&complete::Completion> {
        let completion = &self.cmdline.completion;
        completion.is_open().then_some(completion)
    }

    /// The match a `:s///c` question is about (painted while it waits).
    pub fn confirm_range(&self) -> Option<strop_core::Range> {
        confirm_range(self)
    }
}

/// One parsed command line, as its handler sees it.
pub(crate) struct ExCall<'a> {
    /// The full name the typed word abbreviates (`xit` or `exit`).
    pub name: &'static str,
    pub bang: bool,
    /// The addresses typed, already checked against the buffer for a
    /// command that takes lines.
    pub range: Option<LineRange>,
    /// Everything after the name (and `!`), leading blanks removed.
    pub arg: &'a str,
}

impl ExCall<'_> {
    /// The lines the command applies to, zero-based and inclusive; the
    /// cursor line when none were typed.
    pub(crate) fn lines(&self, editor: &Editor) -> (usize, usize) {
        let line = editor.buf().line_of(editor.head());
        self.range.map_or((line, line), LineRange::lines)
    }

    /// `:d 3`, `:> 2`: a count after the command starts at the range's
    /// last line and runs `count` lines, stopping at the buffer's end.
    /// Returns the lines and what follows the count.
    pub(crate) fn counted<'a>(
        &self,
        editor: &Editor,
        arg: &'a str,
    ) -> Result<((usize, usize), &'a str), String> {
        let lines = self.lines(editor);
        let digits = arg.len() - arg.trim_start_matches(|c: char| c.is_ascii_digit()).len();
        if digits == 0 {
            return Ok((lines, arg));
        }
        let count: usize = arg[..digits].parse().unwrap_or(usize::MAX);
        if count == 0 {
            return Err("positive count required".into());
        }
        let last = lines
            .1
            .saturating_add(count - 1)
            .min(editor.buf().last_content_line());
        Ok(((lines.1, last), arg[digits..].trim_start()))
    }
}

/// What the command line remembers between commands.
#[derive(Debug, Default)]
pub(crate) struct ExState {
    /// The last `:s`, for `:&`, `&`, `g&` and `~`.
    pub last_substitute: Option<substitute::LastSubstitute>,
    /// An open `:s///c` question.
    pub confirm: Option<substitute::Confirm>,
    /// Depth of running compound commands (`:g`, `:normal`): inside one,
    /// everything resolves at once and the undo group stays open.
    pub compound: usize,
    /// The lines `:g` marked and has yet to visit.
    pub marked: global::MarkedLines,
    /// `Ctrl-C` the key reader saw while a compound command runs.
    pub interrupt: crate::editor::interrupt::Interrupt,
    /// Where `:cd -` goes back to.
    pub previous_directory: Option<std::path::PathBuf>,
    /// `:wqa` asked to quit once its writes have landed.
    pub quit_after_writes: bool,
}

/// A command name typed on the line: the command, the name it
/// abbreviates, whether `!` follows, and the argument.
pub(crate) struct Named<'a> {
    pub command: &'static ExCommand,
    pub name: &'static str,
    pub bang: bool,
    pub arg: &'a str,
}

/// The command a line names, whether `!` follows it, and its argument.
/// `None` when no command starts the text.
pub(crate) fn split_command(text: &str) -> Option<Named<'_>> {
    let text = text.trim_start();
    let (command, name, length) = command_word(text)?;
    let rest = &text[length..];
    let (bang, rest) = match rest.strip_prefix('!') {
        Some(rest) if command.bang => (true, rest),
        _ => (false, rest),
    };
    Some(Named {
        command,
        name,
        bang,
        arg: rest.trim_start(),
    })
}

/// The command at the start of `text` and how many bytes name it.
pub(crate) fn command_word(text: &str) -> Option<(&'static ExCommand, &'static str, usize)> {
    let first = text.chars().next()?;
    let word = |extra: fn(char) -> bool| {
        text.len()
            - text
                .trim_start_matches(|c: char| c.is_ascii_alphabetic() || extra(c))
                .len()
    };
    if !first.is_ascii_alphabetic() {
        let (command, name) = table::lookup(&text[..first.len_utf8()])?;
        return Some((command, name, first.len_utf8()));
    }
    let letters = word(|_| false);
    // strop's own commands carry hyphens (`select-next`); `:m-2` is
    // `:m` with an address, so the longer word must name one exactly.
    let hyphenated = word(|c| c == '-');
    if hyphenated > letters {
        if let Some((command, name)) = table::exact_name(&text[..hyphenated]) {
            return Some((command, name, hyphenated));
        }
    }
    match table::lookup(&text[..letters]) {
        Some((command, name)) => Some((command, name, letters)),
        // `:ka` is `:k a` (Vim's one glued argument).
        None if first == 'k' => {
            let (command, name) = table::lookup("k")?;
            Some((command, name, 1))
        }
        None => None,
    }
}

/// What a command returns: `Err` says why it failed, in error ink.
pub(crate) type ExResult = Result<(), String>;

/// Run one command line (the text after the `:`); a failure shows in
/// the echo row as one.
pub(crate) fn run_ex(editor: &mut Editor, cmdline: &str) {
    if let Err(message) = run_ex_line(editor, cmdline) {
        editor.fail(message);
    }
}

fn run_ex_line(editor: &mut Editor, cmdline: &str) -> ExResult {
    let (range, rest) = parse_ex_range(editor, editor.head(), cmdline)?;
    let rest = rest.trim_start();
    if rest.is_empty() {
        if let Some(range) = range {
            // `:N` goes to the line; past the end is the last line.
            let line = range.last.clamp(1, ex_line_count(editor)) - 1;
            editor.push_jump();
            ex_land_in_column(editor, line, ex_column(editor));
            editor.scroll_to_cursor(editor.view_rows());
        }
        return Ok(());
    }
    let Some(Named {
        command,
        name,
        bang,
        arg,
    }) = split_command(rest)
    else {
        return Err(format!("not an editor command: {rest}"));
    };
    match command.range {
        Addr::None if range.is_some() => return Err("no range allowed".into()),
        Addr::Lines => {
            if let Some(range) = range {
                check_ex_range(editor, range)?;
            }
        }
        Addr::None | Addr::Count => {}
    }
    if command.edits && editor.buf().readonly {
        editor.refuse_readonly();
        return Ok(());
    }
    (command.run)(
        editor,
        &ExCall {
            name,
            bang,
            range,
            arg,
        },
    )
}

/// `@:`: the last command line again, `count` times.
fn repeat_command_line(editor: &mut Editor, count: usize) -> ExResult {
    let line = editor
        .cmdline
        .last_command_line()
        .map(str::to_owned)
        .ok_or("no previous command line")?;
    for _ in 0..count.max(1) {
        run_ex(editor, &line);
    }
    Ok(())
}

/// `:@:` repeats the last command line; `:@x` runs register `x`, one
/// command per line.
pub(crate) fn ex_at(editor: &mut Editor, call: &ExCall<'_>) -> ExResult {
    let mut names = call.arg.chars();
    match (names.next(), names.next()) {
        (Some(':'), None) => repeat_command_line(editor, 1),
        (Some(name), None) => {
            let text = editor.register(Some(name)).text;
            if text.is_empty() {
                return Err(format!("nothing in register {name}"));
            }
            for line in text.lines() {
                run_ex(editor, line);
            }
            Ok(())
        }
        _ => Err("usage: :@: or :@{register}".into()),
    }
}

/// Normal `&` (`:s`) and `g&` (`:%s//~/&`): the last substitute
/// again, on the cursor line or the whole buffer, with or without
/// its flags.
fn repeat_substitute(editor: &mut Editor, keep_flags: bool, whole_buffer: bool) {
    run_ex(
        editor,
        match (keep_flags, whole_buffer) {
            (false, false) => "s",
            (true, false) => "&&",
            (false, true) => "%s",
            (true, true) => "%s//~/&",
        },
    );
}

/// The first non-blank of `line` (zero-based), where most line
/// commands leave the cursor.
pub(crate) fn ex_land(editor: &mut Editor, line: usize) {
    let line = line.min(editor.buf().last_content_line());
    let end = editor.buf().line_end(line);
    let mut at = editor.buf().line_start(line);
    while at < end && matches!(editor.buf().byte_at(at), Some(b' ' | b'\t')) {
        at += 1;
    }
    editor.set_head(at);
    editor.clamp_cursor();
}

/// The cursor's column, in characters.
pub(crate) fn ex_column(editor: &Editor) -> usize {
    let head = editor.head();
    editor
        .buf()
        .text()
        .byte_slice(editor.buf().line_start(editor.buf().line_of(head))..head)
        .chars()
        .count()
}

/// `line` at `column` (read before the command changed the text):
/// Neovim's 'nostartofline' for the commands that only change the
/// line (`:N`, `:m`, `:t`).
pub(crate) fn ex_land_in_column(editor: &mut Editor, line: usize, column: usize) {
    let line = line.min(editor.buf().last_content_line());
    let start = editor.buf().line_start(line);
    let text = editor.buf().line_text(line);
    let offset = text
        .char_indices()
        .nth(column)
        .map_or(text.len(), |(offset, _)| offset);
    editor.set_head(start + offset);
    editor.clamp_cursor();
}
