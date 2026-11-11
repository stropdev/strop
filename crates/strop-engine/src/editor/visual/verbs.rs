//! Visual-mode verbs (0095 §2.3): what each key of the `visual` section
//! does with the selection. Operators run through the one operator
//! engine (`normal/operators.rs`); the rest are here.

use strop_core::Range;
use strop_grammar::{self as grammar, Op};

use crate::editor::normal::operators::{Operand, Operation};
use crate::editor::registers::RegisterShape;
use crate::editor::{Editor, LastSearch, Mode, Register};
use crate::keymap::VisualVerb;

pub(crate) fn visual_verb(
    editor: &mut Editor,
    verb: VisualVerb,
    count: Option<usize>,
    register: Option<char>,
    arg: char,
) {
    let reads_only = matches!(
        verb,
        VisualVerb::Operator(Op::Yank)
            | VisualVerb::Lines(Op::Yank)
            | VisualVerb::SwapEnds
            | VisualVerb::Search { .. }
    );
    if editor.buf().readonly && !reads_only {
        editor.refuse_readonly();
        editor.mode = Mode::Normal;
        return;
    }
    match verb {
        VisualVerb::Operator(op) => visual_operate(editor, op, register, false),
        VisualVerb::Lines(op) => visual_operate(editor, op, register, true),
        VisualVerb::Put { keep } => visual_put(editor, register, count.unwrap_or(1), keep),
        VisualVerb::Join { spaces } => {
            let (first, last) = selected_lines(editor);
            editor.mode = Mode::Normal;
            editor.set_head(editor.buf().line_start(first));
            crate::editor::normal::edits::join_lines(editor, Some(last - first + 1), spaces);
        }
        VisualVerb::Replace => visual_replace(editor, arg),
        VisualVerb::Surround => editor.visual_surround(arg),
        VisualVerb::Insert { append } => visual_insert(editor, append),
        VisualVerb::SwapEnds => {
            let selection = editor.sels().primary();
            editor
                .sels_mut()
                .stretch_primary(selection.head, selection.anchor);
        }
        VisualVerb::Search { backward } => visual_search(editor, backward),
        VisualVerb::Comment => editor.toggle_visual_comments(),
        VisualVerb::Add { negative } => {
            let amount = count.unwrap_or(1) as i64;
            visual_add(editor, if negative { -amount } else { amount });
        }
    }
}

/// The selection's first and last line.
pub(crate) fn selected_lines(editor: &Editor) -> (usize, usize) {
    let (a, b) = (
        editor.buf().line_of(editor.anchor()),
        editor.buf().line_of(editor.head()),
    );
    (a.min(b), a.max(b))
}

/// The ranges an operator covers: the selection, one range per row
/// of a block, or one per occurrence selection.
fn selected_ranges(editor: &Editor, whole_lines: bool) -> Vec<Range> {
    if whole_lines || editor.mode == Mode::VisualLine {
        return vec![editor.selected_line_range()];
    }
    if editor.mode == Mode::VisualBlock {
        return editor.block_row_ranges();
    }
    if editor.sels().count() == 1 {
        return editor.visual_range().into_iter().collect();
    }
    let mut spans: Vec<(usize, usize)> = std::iter::once(editor.sels().primary())
        .chain(editor.extra_selections().iter().copied())
        .map(|selection| editor.selection_span(selection))
        .collect();
    spans.sort_unstable();
    spans.dedup();
    spans
        .into_iter()
        .map(|(start, end)| Range::charwise(start, end).with_inclusive(true))
        .collect()
}

/// An operator over the selection, through the one operator engine.
fn visual_operate(editor: &mut Editor, op: Op, register: Option<char>, whole_lines: bool) {
    if !editor.register_writable(register) {
        return;
    }
    if op == Op::Filter {
        let (first, _) = selected_lines(editor);
        editor.mode = Mode::Normal;
        editor.sels_mut().collapse_extras();
        editor.set_head(editor.buf().line_start(first));
        editor.open_ex_line("'<,'>!");
        return;
    }
    let block = editor.mode == Mode::VisualBlock && !whole_lines;
    if block && matches!(op, Op::Delete | Op::Yank | Op::Change) {
        // a block has its own register shape and its own Insert
        match op {
            Op::Yank => editor.block_yank(register),
            Op::Change => editor.block_change(register),
            _ => editor.block_delete(register),
        }
        return;
    }
    let occurrences = editor.mode == Mode::Visual && editor.sels().count() > 1;
    if occurrences && matches!(op, Op::Delete | Op::Yank | Op::Change) {
        // occurrence selections (0049 §7.4) keep their own cursors
        editor.visual_operate_cascade(op, register);
        return;
    }
    let ranges = selected_ranges(editor, whole_lines);
    let Some(first) = ranges.first().copied() else {
        editor.mode = Mode::Normal;
        return;
    };
    let head = editor.head();
    let operands: Vec<Operand> = ranges
        .iter()
        .enumerate()
        .map(|(index, range)| Operand {
            cursor: if index == 0 { head } else { range.start.get() },
            range: *range,
            origin: range.start.get(),
            end_adjusted: false,
        })
        .collect();
    let column = crate::editor::normal::operators::kept_column(editor, op, &operands[0]);
    let linewise = first.is_linewise();
    let repeat = (op != Op::Yank && ranges.len() == 1).then(|| visual_repeat(editor, op, first));
    editor.mode = Mode::Normal;
    editor.sels_mut().collapse_extras();
    crate::editor::normal::operators::apply_operator(
        editor,
        Operation {
            op,
            register,
            operands: &operands,
            reg_one: false,
            keys: if linewise { "V..." } else { "v..." },
            column,
        },
    );
    // (after the operator: entering Insert resets the record)
    match repeat {
        Some(Ok(command)) => {
            editor.last_change = Some(command);
            editor.last_cmd_keys.clear();
        }
        Some(Err(keys)) => {
            editor.last_change = None;
            editor.last_cmd_keys = keys;
        }
        None => {}
    }
}

/// Dot-repeat after a Visual operator: the same operator over the
/// same extent from the cursor — N lines or N characters of one line
/// as a command, otherwise the keys for the selection's line and
/// column deltas.
fn visual_repeat(editor: &Editor, op: Op, range: Range) -> Result<grammar::Command, String> {
    let (first, last) = selected_lines(editor);
    let command = |count: usize, target: grammar::Target, keys: String| grammar::Command {
        op: Some(op),
        register: None,
        count: Some(count),
        target,
        force: None,
        keys,
    };
    if range.is_linewise() {
        let lines = last - first + 1;
        Ok(command(
            lines,
            grammar::Target::Linewise,
            format!("{lines}{0}{0}", op.key()),
        ))
    } else if first == last {
        let chars = editor.buf().slice_string(range).chars().count().max(1);
        Ok(command(
            chars,
            grammar::Target::Motion(grammar::Motion::Right),
            format!("{}{chars}l", op.key()),
        ))
    } else {
        Err(editor.visual_replay_keys(op))
    }
}

/// `p` / `P`: the register takes the selection's place. The replaced
/// text becomes the unnamed register unless `keep`.
fn visual_put(editor: &mut Editor, name: Option<char>, count: usize, keep: bool) {
    let register = editor.register(name);
    if matches!(register.shape, RegisterShape::Blockwise { .. }) || editor.mode == Mode::VisualBlock
    {
        editor.message = "put over a selection: block text is not supported".into();
        return;
    }
    let linewise = editor.mode == Mode::VisualLine;
    let Some(range) = editor.visual_range() else {
        return;
    };
    let replaced = editor.buf().slice_string(range);
    let newline = editor.newline_str();
    let body = register.text.repeat(count);
    let register_lines = register.shape == RegisterShape::Linewise;
    // (text to insert, offset of the cursor inside it)
    let (text, land) = match (linewise, register_lines) {
        (true, true) => (body, 0),
        (true, false) => (format!("{body}{newline}"), 0),
        (false, true) => (format!("{newline}{body}"), newline.len()),
        (false, false) => {
            let last = body.chars().next_back().map_or(0, char::len_utf8);
            let land = body.len().saturating_sub(last);
            (body, land)
        }
    };
    editor.tx_begin();
    editor.buf_mut().delete(range);
    editor.buf_mut().insert(range.start, &text);
    editor.tx_commit();
    if !keep {
        let replaced = if linewise {
            Register::linewise(replaced)
        } else {
            Register::characterwise(replaced)
        };
        editor.delete_register(None, replaced, false);
    }
    editor.mode = Mode::Normal;
    let at = range.start.get() + land;
    if linewise || register_lines {
        // on the first non-blank of the first line put
        let end = editor.buf().line_end(editor.buf().line_of(at));
        let text = (at..end)
            .find(|at| !matches!(editor.buf().byte(*at), b' ' | b'\t'))
            .unwrap_or(at);
        editor.set_head(text);
    } else {
        editor.set_head(at);
    }
    editor.clamp_cursor();
    editor.flash(Range::charwise(editor.head(), editor.head()));
    editor.last_insert = None;
}

/// `r<c>`: every selected character becomes `c`; line breaks stay.
fn visual_replace(editor: &mut Editor, c: char) {
    let ranges = selected_ranges(editor, false);
    let Some(first) = ranges.first().copied() else {
        return;
    };
    editor.tx_begin();
    for range in ranges.iter().rev() {
        let text = editor.buf().slice_string(*range);
        let replaced: String = text
            .chars()
            .map(|old| if matches!(old, '\n' | '\r') { old } else { c })
            .collect();
        let start = range.start;
        editor.buf_mut().delete(*range);
        editor.buf_mut().insert(start, &replaced);
    }
    editor.tx_commit();
    editor.mode = Mode::Normal;
    editor.sels_mut().collapse_extras();
    editor.set_head(first.start.get());
    editor.clamp_cursor();
    editor.flash(Range::charwise(editor.head(), editor.head()));
    editor.last_insert = None;
}

/// `I` / `A`. A block inserts on every row: at its left edge, or
/// after its right edge (after each line's end when it was extended
/// with `$`). Any other selection inserts once, where Vim does: `I` in
/// column 1 of the first line; `A` after the selection's last
/// character, or in column 1 of the last line of a linewise one.
fn visual_insert(editor: &mut Editor, append: bool) {
    let (first, last) = selected_lines(editor);
    if editor.mode == Mode::VisualBlock {
        if append && editor.wants_line_end() {
            editor.block_append_at_ends(first, last);
        } else {
            editor.block_insert(append);
        }
        return;
    }
    let linewise = editor.mode == Mode::VisualLine;
    let after = editor.visual_range().map(|range| range.end.get());
    editor.mode = Mode::Normal;
    editor.sels_mut().collapse_extras();
    let at = match (append, linewise, after) {
        (true, false, Some(after)) => {
            let line = editor.buf().line_of(editor.anchor().max(editor.head()));
            after.min(editor.buf().line_end(line))
        }
        (true, ..) => editor.buf().line_start(last),
        (false, ..) => editor.buf().line_start(first),
    };
    editor.set_head(at);
    editor.enter_insert_from("i");
}

/// `*` / `#`: search for the selected text, literally.
fn visual_search(editor: &mut Editor, backward: bool) {
    let Some(range) = editor.visual_range() else {
        return;
    };
    let text = editor.buf().slice_string(range);
    let text = text.trim_end_matches(['\n', '\r']);
    let mut pattern = String::with_capacity(text.len());
    for c in text.chars() {
        match c {
            '\n' => pattern.push_str("\\n"),
            '\\' | '/' | '.' | '*' | '[' | ']' | '~' | '^' | '$' => {
                pattern.push('\\');
                pattern.push(c);
            }
            other => pattern.push(other),
        }
    }
    let query = match grammar::CompiledQuery::compile(&pattern, false) {
        Ok(query) => query,
        Err(error) => {
            editor.message = error.to_string();
            return;
        }
    };
    // Neovim's default mapping is `y/\V<C-R>"<CR>`: the selection is
    // yanked on the way
    editor.yank_register(None, Register::characterwise(text.to_owned()));
    editor.mode = Mode::Normal;
    editor.set_head(range.start.get());
    editor.last_search = Some(LastSearch { query, backward });
    editor.repeat_search(false);
}

/// `Ctrl-A` / `Ctrl-X`: add to the first number on each selected line.
fn visual_add(editor: &mut Editor, delta: i64) {
    let Some(range) = editor
        .visual_range()
        .or_else(|| (editor.mode == Mode::VisualBlock).then(|| editor.selected_line_range()))
    else {
        return;
    };
    let (first, last) = selected_lines(editor);
    let from = if editor.mode == Mode::Visual {
        range.start.get()
    } else {
        editor.buf().line_start(first)
    };
    editor.mode = Mode::Normal;
    editor.tx_begin();
    for line in (first..=last).rev() {
        let start = if line == first {
            from
        } else {
            editor.buf().line_start(line)
        };
        if let Some((range, replacement)) =
            crate::editor::normal::edits::number_edit(editor, start, delta)
        {
            editor.buf_mut().delete(range);
            editor.buf_mut().insert(range.start, &replacement);
        }
    }
    editor.tx_commit();
    editor.set_head(from);
    editor.clamp_cursor();
}
