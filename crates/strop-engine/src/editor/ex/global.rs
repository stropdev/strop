//! The compound commands: `:g` / `:v` run a command on every (non-)matching
//! line, `:normal` types keys on every line of a range. Each is one undo
//! step, and everything inside one resolves at once.

use std::collections::VecDeque;

use strop_core::frontend_input::Key;
use strop_core::id::DocumentId;
use strop_core::Buffer;

use super::range::{compile_pattern, split_pattern, unescape_delimiter};
use super::ExCall;
use crate::editor::ex::*;
use crate::editor::{Editor, Mode};

/// One line `:g` has yet to visit: where it starts and where its line
/// break is. Both follow the text as earlier commands edit it. `order`
/// is its rank among the marked lines, which edits never change.
#[derive(Debug, Clone, Copy)]
struct Marked {
    start: usize,
    end: usize,
    order: usize,
}

/// The lines `:g` marked, in buffer order. A line a command deletes (or
/// joins into the one above) drops out, as Vim's marks do.
///
/// Each stored offset is the real one less `shift` (wrapping): an edit
/// moves every line after it by the same amount, so it changes `shift`
/// and touches only the lines on the shorter side of it, never all of
/// them (plan 0108 H4: `:g` on 100,000 lines was quadratic).
#[derive(Debug, Default)]
pub(crate) struct MarkedLines {
    document: Option<DocumentId>,
    lines: VecDeque<Marked>,
    shift: usize,
}

impl MarkedLines {
    fn new(document: DocumentId, lines: impl Iterator<Item = (usize, usize)>) -> Self {
        Self {
            document: Some(document),
            lines: lines
                .enumerate()
                .map(|(order, (start, end))| Marked { start, end, order })
                .collect(),
            shift: 0,
        }
    }

    /// A `:g` is running.
    pub(crate) fn busy(&self) -> bool {
        self.document.is_some()
    }

    fn real(&self, line: Marked) -> Marked {
        Marked {
            start: line.start.wrapping_add(self.shift),
            end: line.end.wrapping_add(self.shift),
            order: line.order,
        }
    }

    fn stored(&self, line: Marked) -> Marked {
        Marked {
            start: line.start.wrapping_sub(self.shift),
            end: line.end.wrapping_sub(self.shift),
            order: line.order,
        }
    }

    /// The next line to visit, in real offsets.
    fn pop_front(&mut self) -> Option<Marked> {
        let line = self.lines.pop_front()?;
        Some(self.real(line))
    }

    /// Carry the marks through one batch of edits to `document`.
    /// `journal` is `(start, old_end, new_end, inserted text ends in a
    /// line break)` per edit, in application order; `buffer` is the text
    /// after all of them.
    pub(crate) fn map_edits(
        &mut self,
        document: DocumentId,
        journal: &[(usize, usize, usize, bool)],
        buffer: &Buffer,
    ) {
        if self.document != Some(document) || self.lines.is_empty() {
            return;
        }
        let mut touched = Vec::new();
        for &edit in journal {
            self.map_edit(edit, &mut touched);
        }
        // A touched line whose start no longer begins a line was joined
        // away; the others measure their line's end again. Untouched
        // lines kept their text, so their ends only moved.
        touched.sort_unstable();
        touched.dedup();
        for order in touched {
            let Ok(index) = self.lines.binary_search_by_key(&order, |line| line.order) else {
                continue;
            };
            let mut line = self.real(self.lines[index]);
            let start = line.start.min(buffer.len_bytes());
            if start > 0 && buffer.byte_at(start - 1) != Some(b'\n') {
                self.lines.remove(index);
                continue;
            }
            line.end = buffer.line_end(buffer.line_of(start));
            self.lines[index] = self.stored(line);
        }
    }

    /// One edit: the line holding its start and the lines starting
    /// inside it or at its end follow Vim's mark rules one by one
    /// (`touched` names them); every later line moves by its length.
    fn map_edit(
        &mut self,
        (start, old_end, new_end, breaks): (usize, usize, usize, bool),
        touched: &mut Vec<usize>,
    ) {
        let offset = self.shift;
        let real_start = |line: &Marked| line.start.wrapping_add(offset);
        let mut first = self.lines.partition_point(|line| real_start(line) < start);
        if first > 0 && self.real(self.lines[first - 1]).end >= start {
            first -= 1;
        }
        let last = self
            .lines
            .partition_point(|line| real_start(line) <= old_end);
        let shift = |offset: usize| offset - old_end + new_end;
        let window: Vec<Option<Marked>> = (first..last)
            .map(|index| {
                let mut line = self.real(self.lines[index]);
                if line.start < start {
                    // The line's own text or break changed, at most.
                    if line.end >= old_end {
                        line.end = shift(line.end);
                    } else if line.end >= start {
                        line.end = start;
                    }
                } else if line.start == start && start == old_end {
                    // Inserted at the line's first column: whole lines
                    // above it push it down, text joins it.
                    if breaks {
                        line.start = new_end;
                    }
                    line.end = shift(line.end);
                } else if line.start >= old_end {
                    line.start = shift(line.start);
                    line.end = shift(line.end);
                } else if start < line.start || old_end > line.end {
                    // The edit removed the line's first byte: gone when
                    // it took the whole line with its break, or began
                    // above it.
                    return None;
                } else {
                    line.end = shift(line.end);
                }
                Some(line)
            })
            .collect();
        // Lines after the window move by the edit's length: move them,
        // or move everything through `shift` and put the lines up to the
        // window's end back — whichever touches fewer.
        let delta = new_end.wrapping_sub(old_end);
        if self.lines.len() - last <= last {
            for line in self.lines.range_mut(last..) {
                line.start = line.start.wrapping_add(delta);
                line.end = line.end.wrapping_add(delta);
            }
        } else {
            self.shift = self.shift.wrapping_add(delta);
            for line in self.lines.range_mut(..last) {
                line.start = line.start.wrapping_sub(delta);
                line.end = line.end.wrapping_sub(delta);
            }
        }
        for (index, line) in (first..last).zip(window).rev() {
            match line {
                Some(line) => {
                    touched.push(line.order);
                    self.lines[index] = self.stored(line);
                }
                None => {
                    self.lines.remove(index);
                }
            }
        }
    }
}

/// One character of `:normal`'s argument as the key it types.
fn typed_key(c: char) -> Key {
    match c {
        '\x1b' => Key::Esc,
        '\r' | '\n' => Key::Enter,
        '\t' => Key::Tab,
        '\x7f' | '\x08' => Key::Backspace,
        other => Key::Char(other),
    }
}

/// Enter a compound command: one undo group stays open on the
/// current buffer and nothing inside defers to a worker. Returns
/// what `compound_end` needs.
fn compound_begin(editor: &mut Editor) -> (DocumentId, bool) {
    let document = editor.current();
    let deferring = editor.resolution.enabled;
    if editor.cmdline.ex.compound == 0 {
        editor.buf_mut().hold_undo_group();
        editor.resolution.enabled = false;
    }
    editor.cmdline.ex.compound += 1;
    (document, deferring)
}

fn compound_end(editor: &mut Editor, (document, deferring): (DocumentId, bool)) {
    editor.cmdline.ex.compound -= 1;
    if editor.cmdline.ex.compound == 0 {
        editor.resolution.enabled = deferring;
        if editor.docs.get(document).is_some() {
            editor.doc_mut(document).buf.release_undo_group();
        }
    }
}

/// `:[range]g/pattern/command`, `:g!`, `:v`: mark the lines first,
/// then run the command with the cursor on each that still exists.
pub(crate) fn ex_global(editor: &mut Editor, call: &ExCall<'_>) {
    if editor.cmdline.ex.marked.document.is_some() {
        editor.message = "cannot do :global recursive".into();
        return;
    }
    let invert = call.bang || call.name == "vglobal";
    let Some(delimiter) = call
        .arg
        .chars()
        .next()
        .filter(|c| !c.is_alphanumeric() && !matches!(c, '\\' | '"' | '|' | ' '))
    else {
        editor.message = "regular expression missing from :global".into();
        return;
    };
    let (raw, command, _) = split_pattern(&call.arg[delimiter.len_utf8()..], delimiter);
    let pattern = if raw.is_empty() {
        match editor.last_search.as_ref() {
            Some(search) => search.query.source().to_owned(),
            None => {
                editor.message = "no previous regular expression".into();
                return;
            }
        }
    } else {
        unescape_delimiter(raw, delimiter)
    };
    let command = command.trim_start();
    if command.is_empty() {
        editor.message = ":g needs a command to run on the lines".into();
        return;
    }
    let query = match compile_pattern(&pattern, None) {
        Ok(query) => query,
        Err(error) => {
            editor.message = error;
            return;
        }
    };
    let (first, last) = call
        .range
        .map_or((0, editor.buf().last_content_line()), |range| range.lines());
    let buf = editor.buf();
    let mut matching = vec![false; last - first + 1];
    let visited = strop_grammar::search_visit(buf, &query, |hit| {
        let line = buf.line_of(hit.start.get());
        if line > last {
            return std::ops::ControlFlow::Break(());
        }
        if line >= first {
            matching[line - first] = true;
        }
        std::ops::ControlFlow::Continue(())
    });
    if let Err(error) = visited {
        editor.message = error.to_string();
        return;
    }
    let lines: Vec<(usize, usize)> = (first..=last)
        .filter(|line| matching[line - first] != invert)
        .map(|line| (buf.line_start(line), buf.line_end(line)))
        .collect();
    editor.last_search = Some(crate::editor::LastSearch {
        query,
        backward: false,
    });
    if lines.is_empty() {
        editor.message = if invert {
            format!("pattern found in every line: {pattern}")
        } else {
            format!("pattern not found: {pattern}")
        };
        return;
    }
    let command = command.to_owned();
    editor.push_jump();
    let compound = compound_begin(editor);
    editor.cmdline.ex.marked = MarkedLines::new(compound.0, lines.into_iter());
    while let Some(line) = editor.cmdline.ex.marked.pop_front() {
        if editor.current() != compound.0 || editor.should_quit {
            break;
        }
        if interrupted(editor) {
            break;
        }
        editor.set_head(line.start.min(editor.buf().len_bytes()));
        run_ex(editor, &command);
    }
    editor.cmdline.ex.marked = MarkedLines::default();
    compound_end(editor, compound);
    if editor.current() == compound.0 {
        editor.clamp_cursor();
    }
}

/// One command of a batch (`:cdo`, `:cfdo`): it finishes before the
/// batch moves on, so nothing in it defers to a worker, and whatever it
/// changes in the current buffer is one undo step (0094 §3.1).
pub(crate) fn run_ex_in_batch(editor: &mut Editor, command: &str) {
    let compound = compound_begin(editor);
    run_ex(editor, command);
    compound_end(editor, compound);
}

/// Whether `Ctrl-C` stops the running compound command before its next
/// line (vim's `got_int`); it says so.
fn interrupted(editor: &mut Editor) -> bool {
    match editor.cmdline.ex.interrupt.raised(&editor.tape) {
        Ok(false) => false,
        Ok(true) => {
            editor.message = "Interrupted".into();
            true
        }
        Err(error) => {
            editor.message = error.to_string();
            true
        }
    }
}

/// `:[range]norm[al][!] {keys}`
pub(crate) fn ex_normal_command(editor: &mut Editor, call: &ExCall<'_>) {
    if call.arg.is_empty() {
        editor.message = "argument required".into();
        return;
    }
    ex_normal(
        editor,
        call.range.map(super::range::LineRange::lines),
        call.arg,
    );
}

/// Type `keys` as Normal-mode input: once at the cursor, or with a
/// range (zero-based, inclusive lines) on each line with the cursor
/// at its start. Whatever the keys leave unfinished (a pending
/// operator, Insert or Visual mode, an open prompt) is abandoned, as
/// Vim's `:normal` ends with an Esc. One undo step.
pub(crate) fn ex_normal(editor: &mut Editor, range: Option<(usize, usize)>, keys: &str) {
    let keys: Vec<Key> = keys.chars().map(typed_key).collect();
    let compound = compound_begin(editor);
    match range {
        None => type_normal(editor, &keys),
        Some((first, last)) => {
            // Vim's rule: the line numbers are fixed when the command
            // starts; keys that add or remove lines do not move them,
            // and a number past the end is the last line.
            for line in first..=last {
                if editor.current() != compound.0 || editor.should_quit {
                    break;
                }
                if interrupted(editor) {
                    break;
                }
                let line = line.min(editor.buf().last_content_line());
                editor.set_head(editor.buf().line_start(line));
                type_normal(editor, &keys);
            }
        }
    }
    compound_end(editor, compound);
}

/// One run of typed keys, grounded afterwards.
fn type_normal(editor: &mut Editor, keys: &[Key]) {
    let depth = editor.macro_depth;
    if depth >= 64 {
        editor.message = ":normal recursion too deep".into();
        return;
    }
    editor.macro_depth = depth + 1;
    for key in keys {
        if editor.should_quit {
            break;
        }
        editor.feed(*key);
    }
    editor.macro_depth = depth;
    if editor.cmdline.is_open() {
        editor.cancel_cmdline();
    }
    if editor.mode != Mode::Normal || !editor.walker.is_ground() {
        let message = std::mem::take(&mut editor.message);
        editor.feed(Key::Esc);
        if editor.mode != Mode::Normal {
            editor.feed(Key::Esc);
        }
        editor.message = message;
    }
}
