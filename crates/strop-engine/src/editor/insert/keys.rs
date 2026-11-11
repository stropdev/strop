//! Insert-mode editing keys beyond typing: `Ctrl-W`, `Ctrl-U`, `Ctrl-T`,
//! `Ctrl-D`, `Ctrl-O`, and Replace mode (`R`).

use strop_core::frontend_input::Key;
use strop_core::id::ByteOffset;
use strop_core::Range;

use crate::editor::structural_indent::Edit;
use crate::editor::{Editor, Mode};

/// What the Insert keys beyond typing remember between keys.
#[derive(Debug, Default)]
pub(crate) struct InsertKeys {
    /// `Ctrl-R` is waiting for its register name.
    pub(crate) register: bool,
    /// Where this Insert session's typing began: `Ctrl-W` and `Ctrl-U`
    /// stop there once.
    pub(crate) origin: Option<usize>,
    /// `Ctrl-O` is waiting for its one Normal command.
    pub(crate) one_command: Option<OneCommand>,
    /// Replace mode (`R`): what each typed character replaced (`None`
    /// for one appended at the line's end), newest last.
    pub(crate) replaced: Option<Vec<Option<char>>>,
}

/// `Ctrl-O` waiting for its one Normal command.
#[derive(Debug, Clone, Copy)]
pub(crate) struct OneCommand {
    /// The cursor stood past the line's last character.
    at_line_end: bool,
    /// The `Ctrl-O` key itself has passed.
    armed: bool,
}

/// How far back a deleting key reaches.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Back {
    /// `Ctrl-W`: the word before the cursor.
    Word,
    /// `Ctrl-U`: everything typed on this line, else the line's text.
    Line,
}

/// Where `Ctrl-W` / `Ctrl-U` at `pos` deletes back to. Both stop once
/// at the place this Insert session began, as Vim's do.
fn back_target(editor: &Editor, pos: usize, back: Back) -> usize {
    let buf = editor.buf();
    let line_start = buf.line_start(buf.line_of(pos));
    let blank = |at: usize| matches!(buf.byte(at), b' ' | b'\t');
    let mut floor = line_start;
    if back == Back::Line {
        // autoindent keeps the indent when the cursor is past it
        let mut indent = line_start;
        while indent < pos && blank(indent) {
            indent += 1;
        }
        if indent < pos {
            floor = indent;
        }
    }
    if let Some(origin) = editor.insert_keys.origin.filter(|origin| *origin < pos) {
        floor = floor.max(origin);
    }
    if back == Back::Line {
        return floor;
    }
    let mut at = pos;
    while at > floor && blank(at - 1) {
        at -= 1;
    }
    let class = |at: usize| {
        let start = buf.clamp_boundary(at);
        let c = buf.text().char(buf.text().byte_to_char(start));
        (start, strop_grammar::is_keyword(c), c.is_whitespace())
    };
    if at > floor {
        let (_, word, _) = class(at - 1);
        while at > floor {
            let (start, is_word, is_space) = class(at - 1);
            if is_space || is_word != word {
                break;
            }
            at = start;
        }
    }
    at.max(floor)
}

/// `Ctrl-W` / `Ctrl-U` at every cursor. At a line's start they join
/// it to the line above, like Backspace.
pub(crate) fn insert_delete_back(editor: &mut Editor, back: Back) {
    let mut positions = editor.all_cursors();
    positions.sort_unstable();
    positions.dedup();
    let line_start = |pos: usize| editor.buf().line_start(editor.buf().line_of(pos));
    if positions.iter().all(|&pos| pos == line_start(pos)) {
        editor.feed_insert(Key::Backspace);
        return;
    }
    let edits: Vec<Edit> = positions
        .iter()
        .filter_map(|&pos| {
            let start = back_target(editor, pos, back);
            (start < pos).then(|| Edit {
                cursor: ByteOffset::new(pos),
                start: ByteOffset::new(start),
                end: ByteOffset::new(pos),
                text: String::new(),
                landing: 0,
            })
        })
        .collect();
    let removed = edits
        .iter()
        .find(|edit| edit.cursor.get() == editor.head())
        .map_or(0, |edit| {
            editor
                .buf()
                .text()
                .byte_slice(edit.start.get()..edit.end.get())
                .len_chars()
        });
    if edits.is_empty() {
        return;
    }
    editor.apply_indentation(&edits);
    if let Some(recording) = &mut editor.recording_insert {
        for _ in 0..removed {
            recording.pop();
        }
    }
    // the stop at the session's start is spent
    if editor
        .insert_keys
        .origin
        .is_some_and(|origin| origin >= editor.head())
    {
        editor.insert_keys.origin = None;
    }
}

/// `Ctrl-T` / `Ctrl-D`: indent or dedent the cursor's line by one
/// unit; the cursor keeps its place in the text.
pub(crate) fn insert_shift(editor: &mut Editor, right: bool) {
    let head = editor.head();
    let line = editor.buf().line_of(head);
    let start = editor.buf().line_start(line);
    let before = editor.buf().line_end(line) - start;
    editor.apply_indent(Range::charwise(start, start + 1), right);
    let after = editor.buf().line_end(line) - start;
    let moved = (head as isize + after as isize - before as isize).max(start as isize);
    editor.set_head(moved as usize);
    editor.clamp_cursor();
}

/// `Ctrl-O`: one Normal-mode command, then back to Insert.
pub(crate) fn insert_one_command(editor: &mut Editor) {
    let line_end = editor.buf().line_end(editor.buf().line_of(editor.head()));
    editor.insert_keys.one_command = Some(OneCommand {
        at_line_end: editor.head() >= line_end,
        armed: false,
    });
    editor.mode = Mode::Normal;
    editor.clamp_cursor();
}

/// After a key in Normal mode: if it completed the one command
/// `Ctrl-O` allowed, Insert resumes where that command left the cursor.
pub(crate) fn resume_insert(editor: &mut Editor) {
    let Some(one) = editor.insert_keys.one_command.as_mut() else {
        return;
    };
    if !one.armed {
        // this is the `Ctrl-O` key itself
        one.armed = true;
        return;
    }
    let at_end = one.at_line_end;
    if editor.mode == Mode::Insert {
        editor.insert_keys.one_command = None;
        return;
    }
    if editor.mode != Mode::Normal || !editor.walker.is_ground() || editor.pending.is_active() {
        return;
    }
    editor.insert_keys.one_command = None;
    editor.mode = Mode::Insert;
    let line_end = editor.buf().line_end(editor.buf().line_of(editor.head()));
    // the cursor stood past the last character: it goes back there
    // if the command left it on that character
    if at_end && editor.buf().ceil_boundary(editor.head() + 1) >= line_end {
        editor.set_head(line_end);
    }
}

/// `R`: Insert mode that types over the text.
pub(crate) fn enter_replace(editor: &mut Editor) {
    editor.sels_mut().collapse_extras();
    editor.enter_insert_from("i");
    editor.insert_keys.replaced = Some(Vec::new());
    crate::editor::normal::edits::record_keys(editor, "R".into());
}

/// One character typed in Replace mode: it takes the place of the
/// character under the cursor, or is appended at the line's end.
pub(crate) fn replace_typed(editor: &mut Editor, c: char) {
    let head = editor.head();
    let line_end = editor.buf().line_end(editor.buf().line_of(head));
    let mut over = None;
    if head < line_end {
        let end = editor.buf().ceil_boundary(head + 1);
        let old = editor.buf_mut().delete(Range::charwise(head, end));
        over = old.chars().next();
    }
    let mut bytes = [0u8; 4];
    editor.buf_mut().insert(head, c.encode_utf8(&mut bytes));
    editor.set_head(head + c.len_utf8());
    if let Some(replaced) = &mut editor.insert_keys.replaced {
        replaced.push(over);
    }
    if let Some(recording) = &mut editor.recording_insert {
        recording.push(c);
    }
}

/// Backspace in Replace mode: the character typed last gives its
/// place back to what it replaced. Past the typed text it only moves.
pub(crate) fn replace_backspace(editor: &mut Editor) {
    let head = editor.head();
    let line_start = editor.buf().line_start(editor.buf().line_of(head));
    if head == line_start {
        return;
    }
    let start = editor.buf().clamp_boundary(head - 1);
    if let Some(original) = editor.insert_keys.replaced.as_mut().and_then(Vec::pop) {
        editor.buf_mut().delete(Range::charwise(start, head));
        if let Some(original) = original {
            let mut bytes = [0u8; 4];
            editor
                .buf_mut()
                .insert(start, original.encode_utf8(&mut bytes));
        }
        if let Some(recording) = &mut editor.recording_insert {
            recording.pop();
        }
    }
    editor.set_head(start);
}
