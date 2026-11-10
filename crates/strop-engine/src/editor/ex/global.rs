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
/// break is. Both follow the text as earlier commands edit it.
#[derive(Debug, Clone, Copy)]
struct Marked {
    start: usize,
    end: usize,
}

/// The lines `:g` marked, in buffer order. A line a command deletes (or
/// joins into the one above) drops out, as Vim's marks do.
#[derive(Debug, Default)]
pub(crate) struct MarkedLines {
    document: Option<DocumentId>,
    lines: VecDeque<Marked>,
}

impl MarkedLines {
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
        for &(start, old_end, new_end, breaks) in journal {
            let shift = |offset: usize| offset - old_end + new_end;
            self.lines.retain_mut(|line| {
                if line.start < start {
                    // The line's own text or break changed, at most.
                    if line.end >= old_end {
                        line.end = shift(line.end);
                    } else if line.end >= start {
                        line.end = start;
                    }
                    return true;
                }
                if line.start == start && start == old_end {
                    // Inserted at the line's first column: whole lines
                    // above it push it down, text joins it.
                    if breaks {
                        line.start = new_end;
                    }
                    line.end = shift(line.end);
                    return true;
                }
                if line.start >= old_end {
                    line.start = shift(line.start);
                    line.end = shift(line.end);
                    return true;
                }
                // The edit removed the line's first byte: gone when it
                // took the whole line with its break, or began above it.
                if start < line.start || old_end > line.end {
                    return false;
                }
                line.end = shift(line.end);
                true
            });
        }
        // A line whose start no longer begins a line was joined away.
        self.lines.retain_mut(|line| {
            let start = line.start.min(buffer.len_bytes());
            if start > 0 && buffer.byte_at(start - 1) != Some(b'\n') {
                return false;
            }
            line.end = buffer.line_end(buffer.line_of(start));
            true
        });
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
    if editor.pending.ex.compound == 0 {
        editor.buf_mut().hold_undo_group();
        editor.resolution.enabled = false;
    }
    editor.pending.ex.compound += 1;
    (document, deferring)
}

fn compound_end(editor: &mut Editor, (document, deferring): (DocumentId, bool)) {
    editor.pending.ex.compound -= 1;
    if editor.pending.ex.compound == 0 {
        editor.resolution.enabled = deferring;
        if editor.docs.get(document).is_some() {
            editor.doc_mut(document).buf.release_undo_group();
        }
    }
}

/// `:[range]g/pattern/command`, `:g!`, `:v`: mark the lines first,
/// then run the command with the cursor on each that still exists.
pub(crate) fn ex_global(editor: &mut Editor, call: &ExCall<'_>) {
    if editor.pending.ex.marked.document.is_some() {
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
    let lines: VecDeque<Marked> = (first..=last)
        .filter(|line| matching[line - first] != invert)
        .map(|line| Marked {
            start: buf.line_start(line),
            end: buf.line_end(line),
        })
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
    let compound = compound_begin(editor);
    editor.pending.ex.marked = MarkedLines {
        document: Some(compound.0),
        lines,
    };
    while let Some(line) = editor.pending.ex.marked.lines.pop_front() {
        if editor.current() != compound.0 || editor.should_quit {
            break;
        }
        editor.set_head(line.start.min(editor.buf().len_bytes()));
        run_ex(editor, &command);
    }
    editor.pending.ex.marked = MarkedLines::default();
    compound_end(editor, compound);
    if editor.current() == compound.0 {
        editor.clamp_cursor();
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
    if editor.pending.is_active() {
        editor.cancel_pending();
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
