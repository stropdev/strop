//! The one operator engine (0095 §2.2): an operator applied to resolved
//! ranges. Normal mode passes the plan's targets, Visual passes its
//! selection — neither has a second implementation.

use strop_core::id::DisplayColumn;
use strop_core::Range;
use strop_grammar::Op;

use crate::editor::{Editor, Register};

/// One range an operator acts on.
#[derive(Debug, Clone, Copy)]
pub(crate) struct Operand {
    /// The cursor the range was resolved from (the primary's identity).
    pub(crate) cursor: usize,
    pub(crate) range: Range,
    /// The operated text's first position before a linewise range
    /// widened it: where an operator that keeps the text leaves the cursor.
    pub(crate) origin: usize,
    /// `Resolved::end_adjusted`.
    pub(crate) end_adjusted: bool,
}

/// An operator ready to run.
pub(crate) struct Operation<'a> {
    pub(crate) op: Op,
    pub(crate) register: Option<char>,
    /// Sorted by start, non-overlapping.
    pub(crate) operands: &'a [Operand],
    /// Vim's `use_reg_one`: a delete by `%`, a search or a paragraph
    /// motion fills `"1` even within one line.
    pub(crate) reg_one: bool,
    /// What opened an Insert session (`c`), for its dot-repeat record.
    pub(crate) keys: &'a str,
    /// Where a linewise delete or shift leaves the cursor in its line:
    /// this column (Neovim's `nostartofline`), or the first non-blank.
    pub(crate) column: Option<DisplayColumn>,
}

fn rot13(c: char) -> char {
    match c {
        'a'..='z' => char::from((c as u8 - b'a' + 13) % 26 + b'a'),
        'A'..='Z' => char::from((c as u8 - b'A' + 13) % 26 + b'A'),
        other => other,
    }
}

/// A case mapping that is one scalar; otherwise the original (a letter
/// whose other case is longer keeps its place and its length).
fn single(mut cased: impl Iterator<Item = char>, original: char) -> char {
    match (cased.next(), cased.next()) {
        (Some(c), None) => c,
        _ => original,
    }
}

fn recase(op: Op, text: &str) -> String {
    text.chars()
        .map(|c| match op {
            Op::Upper => single(c.to_uppercase(), c),
            Op::Lower => single(c.to_lowercase(), c),
            Op::ToggleCase if c.is_lowercase() => single(c.to_uppercase(), c),
            Op::ToggleCase => single(c.to_lowercase(), c),
            _ => rot13(c),
        })
        .collect()
}

pub(crate) fn apply_operator(editor: &mut Editor, operation: Operation<'_>) {
    let Operation {
        op,
        register,
        operands,
        reg_one,
        keys,
        column,
    } = operation;
    let Some(first) = operands.first() else {
        return;
    };
    let linewise = first.range.is_linewise();
    if operands.iter().all(|o| o.range.is_empty()) && !matches!(op, Op::Change) && !op.on_lines() {
        // vim: an empty region is not an error; the cursor still
        // goes to where the region is (`di(` on `()`)
        editor.set_head(first.origin);
        editor.clamp_cursor();
        return;
    }
    let shapes = || operands.iter().map(|o| (o.range, o.range.is_linewise()));
    let primary_origin = operands
        .iter()
        .find(|o| o.cursor == editor.head())
        .unwrap_or(first)
        .origin;
    match op {
        Op::Yank => {
            // one register, parts joined with newlines (helix rule)
            let text = operands
                .iter()
                .map(|o| editor.buf().slice_string(o.range))
                .collect::<Vec<_>>()
                .join("\n");
            let mut register_text = if linewise {
                Register::linewise(text)
            } else {
                Register::characterwise(text)
            };
            register_text.file_provenance = editor.capture_filename_register(shapes(), false, true);
            editor.yank_register(register, register_text);
            if operands.len() == 1 {
                // vim: the cursor goes to the start of the yanked text
                editor.set_head(primary_origin);
                editor.clamp_cursor();
            }
            editor.flash(first.range);
        }
        Op::Indent | Op::Dedent => {
            editor.tx_begin();
            for operand in operands.iter().rev() {
                editor.apply_indent(operand.range, op == Op::Indent);
            }
            editor.tx_commit();
            land_in_line(editor, column);
            editor.normalize_cursors();
            editor.flash(Range::charwise(editor.head(), editor.head()));
        }
        Op::Filter => {
            // vim: `!{motion}` opens the `:` line with the lines as its
            // range; the command typed there filters them
            let lines = editor
                .buf()
                .line_of(first.range.end.get().saturating_sub(1))
                - editor.buf().line_of(first.range.start);
            editor.set_head(first.range.start.get());
            editor.clamp_cursor();
            editor.open_ex_line(&match lines {
                0 => ".!".to_owned(),
                more => format!(".,.+{more}!"),
            });
        }
        Op::Reindent | Op::Format | Op::FormatKeep | Op::Fold => {
            editor.message = format!(
                "{} ({}) is not supported — `strop compat` lists what is missing",
                op.key(),
                op.name()
            );
        }
        Op::Upper | Op::Lower | Op::ToggleCase | Op::Rot13 => {
            editor.tx_begin();
            for operand in operands.iter().rev() {
                let text = editor.buf().slice_string(operand.range);
                let cased = recase(op, &text);
                if cased != text {
                    let start = operand.range.start;
                    editor.buf_mut().delete(operand.range);
                    editor.buf_mut().insert(start, &cased);
                }
            }
            editor.tx_commit();
            editor.set_head(primary_origin);
            let mut origins: Vec<usize> = operands
                .iter()
                .map(|o| o.origin)
                .filter(|origin| *origin != primary_origin)
                .collect();
            origins.sort_unstable();
            editor.sels_mut().set_extras(origins);
            editor.clamp_cursor();
            editor.flash(first.range);
        }
        Op::Delete | Op::Change => {
            if op == Op::Change && linewise {
                // vim cc/S: clear content, keep the line — never
                // merge with the next one
                change_lines(editor, register, keys, operands);
                return;
            }
            // yank text reads top-down before any delete lands
            let texts: Vec<String> = operands
                .iter()
                .map(|o| editor.buf().slice_string(o.range))
                .collect();
            let empty = operands.iter().all(|o| o.range.is_empty());
            let provenance = editor.capture_filename_register(shapes(), true, true);
            editor.tx_begin();
            for operand in operands.iter().rev() {
                editor.filename_delete_hint(operand.range, operand.range.is_linewise());
                editor.buf_mut().delete(operand.range);
                editor.clear_filename_hint();
            }
            if !empty {
                let text = texts.join("\n");
                let mut register_text = if linewise {
                    Register::linewise(text)
                } else {
                    Register::characterwise(text)
                };
                register_text.file_provenance = provenance;
                editor.delete_register(register, register_text, reg_one);
            }
            // landings: each range start minus what lower deletes
            // already removed (deletes applied bottom-up above)
            let mut shift = 0usize;
            let mut landings: Vec<(bool, usize)> = Vec::with_capacity(operands.len());
            for operand in operands {
                landings.push((
                    operand.cursor == editor.head(),
                    operand.range.start.get() - shift,
                ));
                shift += operand.range.end - operand.range.start;
            }
            if linewise {
                // deleting the final lines leaves no line at that
                // index: vim lands on the new last line, never on
                // the phantom row past the final break
                let last = editor.buf().line_start(editor.buf().last_content_line());
                for landing in &mut landings {
                    landing.1 = landing.1.min(last);
                }
            }
            editor.set_head(
                landings
                    .iter()
                    .find(|(p, _)| *p)
                    .map(|(_, s)| *s)
                    .unwrap_or(landings[0].1),
            );
            // extras are the SECONDARY landings — the primary must
            // never stack with itself (0015: dw leaves 1 cursor)
            let mut starts: Vec<usize> = landings
                .iter()
                .filter(|(p, _)| !*p)
                .map(|(_, s)| *s)
                .collect();
            starts.sort_unstable();
            editor.sels_mut().set_extras(starts);
            if op == Op::Change {
                // no commit: the insert session closes the undo unit
                editor.enter_insert_from(keys);
                // clamp AFTER the mode switch — insert mode allows
                // the end-of-line cursor C just earned (0001 §5.5)
                editor.clamp_cursor();
            } else {
                if linewise {
                    land_in_line(editor, column);
                }
                editor.clamp_cursor();
                editor.tx_commit();
            }
            editor.flash(Range::charwise(editor.head(), editor.head()));
        }
    }
}

/// Put the cursor in its line at `column`, or on the first non-blank.
fn land_in_line(editor: &mut Editor, column: Option<DisplayColumn>) {
    let line = editor.buf().line_of(editor.head());
    match column {
        Some(column) => editor.land_at_cell(line, column),
        None => {
            let (start, end) = (editor.buf().line_start(line), editor.buf().line_end(line));
            let blank = |at: &usize| matches!(editor.buf().byte(*at), b' ' | b'\t');
            editor.set_head((start..end).find(|at| !blank(at)).unwrap_or(end));
            editor.clamp_cursor();
        }
    }
}

/// The column a linewise delete or shift keeps (Neovim's
/// `nostartofline`): the one the cursor wanted before the command.
/// `None` when a delete became linewise by `exclusive-linewise`.
pub(crate) fn kept_column(editor: &Editor, op: Op, operand: &Operand) -> Option<DisplayColumn> {
    (!operand.end_adjusted || op.on_lines()).then(|| {
        editor
            .view()
            .desired_column
            .unwrap_or_else(|| editor.cluster_cells(editor.head()).1 - 1)
    })
}

/// vim `cc` / `S` (and counted `2cc`): clear each line's content,
/// keep the line and its indent, open insert at the indent. The
/// register gets the full lines (with newlines), linewise.
pub(crate) fn change_lines(
    editor: &mut Editor,
    register_name: Option<char>,
    keys: &str,
    operands: &[Operand],
) {
    let provenance = editor.capture_filename_register(
        operands.iter().map(|o| (o.range, o.range.is_linewise())),
        false,
        false,
    );
    let newline = editor.newline_str();
    // texts + indents read top-down before any edit lands
    let texts: Vec<String> = operands
        .iter()
        .map(|o| {
            let first = editor.buf().line_of(o.range.start);
            let last = editor.buf().line_of(o.range.end.saturating_sub(1));
            let s = editor.buf().line_start(first);
            let e = editor
                .buf()
                .line_start(last + 1)
                .min(editor.buf().len_bytes());
            editor.buf().text().byte_slice(s..e).to_string()
        })
        .collect();
    let indents: Vec<String> = operands
        .iter()
        .map(|o| {
            let line = editor.buf().line_of(o.range.start);
            editor
                .buf()
                .line_text(line)
                .chars()
                .take_while(|c| *c == ' ' || *c == '\t')
                .collect()
        })
        .collect();
    editor.tx_begin();
    // apply bottom-up: each entry's range is still valid when it
    // applies; record (primary?, landing, net byte delta) per entry
    // and shift higher landings by the nets of the lower ones
    let mut entries: Vec<(bool, usize, isize)> = Vec::with_capacity(operands.len());
    for (operand, indent) in operands.iter().zip(indents.iter()).rev() {
        let r = operand.range;
        let first = editor.buf().line_of(r.start);
        let last = editor.buf().line_of(r.end.saturating_sub(1));
        let start = editor.buf().line_start(first);
        let before = editor.buf().len_bytes() as isize;
        if first == last {
            // one line: clear content, keep the newline
            let end = editor.buf().line_end(first);
            editor.buf_mut().delete(Range::charwise(start, end));
            editor.buf_mut().insert(start, indent);
        } else {
            // N lines collapse into one fresh line
            let end = editor
                .buf()
                .line_start(last + 1)
                .min(editor.buf().len_bytes());
            editor.filename_change_hint(Range::charwise(start, end));
            editor.buf_mut().delete(Range::charwise(start, end));
            editor.clear_filename_hint();
            editor
                .buf_mut()
                .insert(start, &format!("{indent}{newline}"));
        }
        let net = editor.buf().len_bytes() as isize - before;
        entries.push((operand.cursor == editor.head(), start + indent.len(), net));
    }
    let landings: Vec<(bool, usize)> = entries
        .iter()
        .enumerate()
        .map(|(i, (p, at, _))| {
            // entries after i applied at lower positions → they
            // shift this landing
            let shift: isize = entries[i + 1..].iter().map(|(_, _, n)| n).sum();
            (*p, (*at as isize + shift).max(0) as usize)
        })
        .collect();
    let mut register = Register::linewise(texts.join(""));
    register.file_provenance = provenance;
    editor.delete_register(register_name, register, false);
    editor.set_head(
        landings
            .iter()
            .find(|(p, _)| *p)
            .map(|(_, s)| *s)
            .unwrap_or(landings[0].1),
    );
    let mut starts: Vec<usize> = landings.iter().map(|(_, s)| *s).collect();
    starts.sort_unstable();
    editor.sels_mut().set_extras(starts);
    editor.enter_insert_from(keys);
    editor.clamp_cursor();
    editor.flash(Range::charwise(editor.head(), editor.head()));
}
