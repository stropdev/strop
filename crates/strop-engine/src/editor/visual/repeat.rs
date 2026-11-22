//! `.` after a Visual verb: the same verb over the same extent from the
//! cursor, Vim's `redo_VIsual`. The extent is what Vim keeps: the number
//! of lines and, for a characterwise selection, its width on one line or
//! the cell its last line ends on — or that line's end, after `$`.

use strop_core::id::DisplayColumn;
use strop_grammar::Op;

use super::verbs::{run_verb, selected_lines};
use crate::editor::{Editor, Mode};
use crate::keymap::VisualVerb;

/// How much text a Visual repeat covers, measured from the cursor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum VisualExtent {
    /// `V`: this many whole lines.
    Lines(usize),
    /// `v` within one line: this many cells.
    Cells(usize),
    /// `v` over several lines, the last one ending on this cell.
    ToCell { lines: usize, cell: DisplayColumn },
    /// `v` after `$`: to the end of the last of these lines.
    ToLineEnd { lines: usize },
}

impl VisualExtent {
    fn lines(self) -> usize {
        match self {
            VisualExtent::Lines(lines)
            | VisualExtent::ToCell { lines, .. }
            | VisualExtent::ToLineEnd { lines } => lines,
            VisualExtent::Cells(_) => 1,
        }
    }
}

/// A Visual verb as `.` repeats it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct VisualRepeat {
    pub(crate) extent: VisualExtent,
    pub(crate) verb: VisualVerb,
    pub(crate) count: Option<usize>,
    pub(crate) register: Option<char>,
    pub(crate) arg: char,
}

impl VisualRepeat {
    /// What `.` repeats after `verb` runs on the current selection.
    /// `None` for a verb that changes nothing, for `I`/`A` (their Insert
    /// session records itself) and for a block (#276).
    pub(crate) fn capture(
        editor: &Editor,
        verb: VisualVerb,
        count: Option<usize>,
        register: Option<char>,
        arg: char,
    ) -> Option<Self> {
        if editor.mode == Mode::VisualBlock {
            return None;
        }
        let (verb, count, register) = match verb {
            VisualVerb::Operator(Op::Yank | Op::Filter)
            | VisualVerb::Lines(Op::Yank | Op::Filter)
            | VisualVerb::Insert { .. }
            | VisualVerb::SwapEnds
            | VisualVerb::Search { .. } => return None,
            // Vim's Visual put is a delete, then a put: only the delete
            // repeats — into the unnamed register, or nowhere after `P`
            VisualVerb::Put { keep } => {
                (VisualVerb::Operator(Op::Delete), None, keep.then_some('_'))
            }
            verb => (verb, count, register),
        };
        let linewise = editor.mode == Mode::VisualLine || matches!(verb, VisualVerb::Lines(_));
        Some(Self {
            extent: extent(editor, linewise),
            verb,
            count,
            register,
            arg,
        })
    }
}

/// The selection's extent, as Vim measures it for a repeat.
fn extent(editor: &Editor, linewise: bool) -> VisualExtent {
    let (first, last) = selected_lines(editor);
    let lines = last - first + 1;
    if linewise {
        return VisualExtent::Lines(lines);
    }
    if editor.view().desired_column == Some(DisplayColumn::new(usize::MAX)) {
        return VisualExtent::ToLineEnd { lines };
    }
    let selection = editor.sels().primary();
    let (start, end) = (
        selection.anchor.min(selection.head),
        selection.anchor.max(selection.head),
    );
    let (_, end_cells) = editor.cluster_cells(end);
    if lines == 1 {
        let (start_cell, _) = editor.cluster_cells(start);
        VisualExtent::Cells(end_cells.get().saturating_sub(start_cell.get()).max(1))
    } else {
        VisualExtent::ToCell {
            lines,
            cell: DisplayColumn::new(end_cells.get().saturating_sub(1)),
        }
    }
}

/// `.` after a Visual verb: select the extent from the cursor, then run
/// the verb on it. The repeat stays what it was — a replay measures
/// nothing (Vim's `redo_VIsual_busy`).
pub(crate) fn replay(editor: &mut Editor, repeat: &VisualRepeat) {
    let start = editor.head();
    let line = editor.buf().line_of(start);
    let last = (line + repeat.extent.lines() - 1).min(editor.buf().last_content_line());
    let cursor = editor.shown_cell();
    let line_end = DisplayColumn::new(usize::MAX);
    // the cell the selection ends on, which the cursor then wants (Vim
    // sets `curswant` to it, so a shift lands there)
    let wanted = match repeat.extent {
        VisualExtent::Lines(_) => None,
        VisualExtent::Cells(width) => Some(cursor + (width - 1)),
        VisualExtent::ToCell { cell, .. } => Some(cell),
        // Vim adds its MAXCOL to the cursor's column here, which wraps
        // for any column past the second: the selection runs back to the
        // line's start
        VisualExtent::ToLineEnd { lines: 1 } if cursor.get() > 1 => Some(DisplayColumn::new(0)),
        VisualExtent::ToLineEnd { .. } => Some(line_end),
    };
    let tab = editor.cur_indent().width;
    let (mode, end) = match wanted {
        None => (
            Mode::VisualLine,
            editor.buf().byte_at_cell(last, cursor, tab, false),
        ),
        Some(cell) if cell == line_end => (Mode::Visual, editor.buf().line_end(last)),
        Some(cell) => (
            Mode::Visual,
            editor.buf().byte_at_cell(last, cell, tab, true),
        ),
    };
    editor.sels_mut().collapse_extras();
    editor.sels_mut().stretch_primary(start, end);
    if wanted.is_some() {
        editor.view_mut().desired_column = wanted;
    }
    editor.mode = mode;
    run_verb(editor, *repeat);
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    // Neovim has no Visual `S<c>` to diff against
    #[test]
    fn dot_after_a_visual_surround_wraps_the_same_extent_from_the_cursor() {
        let mut editor = Editor::new(Buffer::from_text("alpha\nbravo\ncharlie\n"));
        editor.feed_text("vlS(j0.");
        assert_eq!(
            editor.buf().text().to_string(),
            "(al)pha\n(br)avo\ncharlie\n"
        );
        editor.feed_text("u");
        assert_eq!(editor.buf().text().to_string(), "(al)pha\nbravo\ncharlie\n");
    }
}
