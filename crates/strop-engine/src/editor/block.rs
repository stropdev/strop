//! Visual rectangles share streamed cell geometry with rendering. Partial wide
//! clusters become spaces; complete tabs retain their bytes in the register.
use super::transact::ChangeSet;
use super::{BlockRect, Editor, Mode, Register};
use strop_core::id::DisplayColumn;
use strop_core::layout::RopeGraphemes;
use strop_core::{Range, Replacement};

pub(crate) struct BlockInsertState {
    rows: Vec<usize>,
    column: DisplayColumn,
    pad: bool,
    /// Each row gets the text after its own end (`$A`), not at `column`.
    at_line_ends: bool,
    /// Where `I` / `A` leave the cursor when the Insert ends: the
    /// block's top-left cell.
    home: Option<(usize, DisplayColumn)>,
}
/// Geometry only: copy complete clusters from the rope into the final
/// register, and represent clipped cluster fragments by their cell counts.
struct RowPart {
    range: Range,
    whole: Range,
    leading: usize,
    trailing: usize,
    retained: usize,
    cells: usize,
    short: bool,
}

impl Editor {
    pub fn enter_block_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        if self.mode == Mode::Normal {
            self.view_mut().desired_column = None;
        }
        self.enter_visual_mode(Mode::VisualBlock);
    }

    /// The selection was extended with `$`: it reaches each line's end.
    fn wants_line_end(&self) -> bool {
        // An explicit N| wants at most usize::MAX - 1 (zero-based).
        self.view().desired_column == Some(DisplayColumn::new(usize::MAX))
    }

    /// The selected part of each row of the block, top to bottom; rows
    /// the block does not reach are left out.
    pub(crate) fn block_row_ranges(&self) -> Vec<Range> {
        let Some(rect) = self.block_rect() else {
            return Vec::new();
        };
        (rect.first_line..=rect.last_line)
            .map(|line| self.row_part(line, rect).range)
            .filter(|range| !range.is_empty())
            .collect()
    }

    /// `$A`: append at each row's own end, not at one fixed column.
    fn block_append_at_ends(&mut self, rect: BlockRect) {
        let (first, last) = (rect.first_line, rect.last_line);
        self.mode = Mode::Insert;
        self.set_head(self.buf().line_end(first));
        self.block_insert_state = Some(BlockInsertState {
            rows: (first + 1..=last).collect(),
            column: DisplayColumn::new(0),
            pad: false,
            at_line_ends: true,
            home: Some((first, rect.left_cell)),
        });
        self.enter_insert_from("<c-v>I");
    }
    pub fn block_rect_pub(&self) -> Option<BlockRect> {
        self.block_rect()
    }

    /// The cell a vertical move or a linewise operator keeps (Vim's
    /// curswant): the one remembered, else the one the cursor shows on —
    /// a TAB's last cell, any other character's first (a wide character
    /// shows the cursor on its left half).
    pub(crate) fn wanted_cell(&self) -> DisplayColumn {
        self.view()
            .desired_column
            .unwrap_or_else(|| self.shown_cell())
    }

    /// The cell the cursor shows on: a TAB's last cell, any other
    /// character's first (Vim's `w_virtcol`).
    pub(crate) fn shown_cell(&self) -> DisplayColumn {
        let (start, end) = self.cluster_cells(self.head());
        if self.buf().byte_at(self.head()) == Some(b'\t') {
            end - 1
        } else {
            start
        }
    }

    pub(crate) fn cluster_cells(&self, byte: usize) -> (DisplayColumn, DisplayColumn) {
        let line = self.buf().line_of(byte);
        let start = self.buf().line_start(line);
        let slice = self
            .buf()
            .text()
            .byte_slice(start..self.buf().line_end(line));
        let relative = byte.saturating_sub(start);
        let mut end = DisplayColumn::new(0);
        for (span, cluster) in RopeGraphemes::new(slice, self.cur_indent().width) {
            end = span.cell + span.width;
            if relative < span.byte + cluster.len() {
                return (span.cell, span.cell + span.width.max(1));
            }
        }
        (end, end + 1)
    }

    pub(crate) fn block_rect(&self) -> Option<BlockRect> {
        if self.mode != Mode::VisualBlock {
            return None;
        }
        let (anchor, head) = (self.anchor(), self.head());
        let (a, a_end) = self.cluster_cells(anchor);
        let (h, h_end) = self.cluster_cells(head);
        Some(BlockRect {
            first_line: self.buf().line_of(anchor).min(self.buf().line_of(head)),
            last_line: self.buf().line_of(anchor).max(self.buf().line_of(head)),
            left_cell: a.min(h),
            right_cell: (!self.wants_line_end()).then(|| a_end.max(h_end) - 1),
        })
    }

    /// Block vertical movement preserves the requested display column through
    /// short rows and clusters; byte columns cannot express that intent.
    pub(crate) fn block_vertical(&mut self, command: &strop_grammar::Command) -> bool {
        if self.mode != Mode::VisualBlock {
            return false;
        }
        let down = match command.target {
            strop_grammar::Target::Motion(strop_grammar::Motion::Down) => true,
            strop_grammar::Target::Motion(strop_grammar::Motion::Up) => false,
            _ => return false,
        };
        let desired = self.wanted_cell();
        let line = self.buf().line_of(self.head());
        let count = command.count.unwrap_or(1).max(1);
        let target = if down {
            line.saturating_add(count)
                .min(self.buf().last_content_line())
        } else {
            line.saturating_sub(count)
        };
        self.land_at_cell(target, desired);
        self.view_mut().desired_column = Some(desired);
        true
    }

    fn row_part(&self, line: usize, rect: BlockRect) -> RowPart {
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let empty = Range::charwise(end, end);
        let mut part = RowPart {
            range: empty,
            whole: empty,
            leading: 0,
            trailing: 0,
            retained: 0,
            cells: 0,
            short: false,
        };
        let left = rect.left_cell;
        let limit = rect.right_cell.map(|right| right + 1);
        let mut row_end = DisplayColumn::new(0);
        let text = self.buf().text().byte_slice(start..end);
        for (span, cluster) in RopeGraphemes::new(text, self.cur_indent().width) {
            if limit.is_some_and(|right| span.cell >= right) {
                break;
            }
            row_end = span.cell + span.width;
            if row_end <= left || span.width == 0 {
                continue;
            }
            let byte = (start + span.byte).into();
            let next = byte + cluster.len();
            part.range.start = part.range.start.min(byte);
            part.range.end = next;
            let cells = limit.map_or(row_end, |right| row_end.min(right)) - span.cell.max(left);
            part.cells += cells;
            if cells == span.width {
                part.whole.start = part.whole.start.min(byte);
                part.whole.end = next;
            } else {
                part.retained += span.width - cells;
                if part.whole.is_empty() {
                    part.leading += cells;
                } else {
                    part.trailing += cells;
                }
            }
        }
        part.short = row_end < left;
        part
    }

    fn block_register(&self, rect: BlockRect, mut visit: impl FnMut(usize, &RowPart)) -> Register {
        let parts: Vec<_> = (rect.first_line..=rect.last_line)
            .map(|line| self.row_part(line, rect))
            .collect();
        let width = rect.right_cell.map_or_else(
            || {
                parts
                    .iter()
                    .map(|part| part.cells)
                    .max()
                    .unwrap_or(0)
                    .max(1)
            },
            |right| right - rect.left_cell + 1,
        );
        // Vim includes the virtual EOL cell when padding rows that never
        // reached the block. A row ending exactly at its left edge is empty.
        let short_pad = width + usize::from(rect.right_cell.is_none());
        let bytes = parts
            .iter()
            .map(|p| {
                if p.short {
                    short_pad
                } else {
                    p.leading + p.whole.len() + p.trailing
                }
            })
            .sum::<usize>()
            + parts.len()
            - 1;
        let mut text = String::with_capacity(bytes);
        for (i, part) in parts.iter().enumerate() {
            if i > 0 {
                text.push('\n');
            }
            if part.short {
                text.extend(std::iter::repeat_n(' ', short_pad));
            } else {
                text.extend(std::iter::repeat_n(' ', part.leading));
                if !part.whole.is_empty() {
                    let slice = self
                        .buf()
                        .text()
                        .byte_slice(part.whole.start.get()..part.whole.end.get());
                    for chunk in slice.chunks() {
                        text.push_str(chunk);
                    }
                }
                text.extend(std::iter::repeat_n(' ', part.trailing));
            }
            visit(rect.first_line + i, part);
        }
        Register::blockwise(text, width.into())
    }

    pub(crate) fn land_at_cell(&mut self, line: usize, column: DisplayColumn) {
        let byte = self.buf().byte_at_cell(
            line,
            column,
            self.cur_indent().width,
            self.mode != Mode::Normal,
        );
        self.set_head(byte);
    }

    pub(crate) fn block_yank(&mut self, register: Option<char>) {
        let Some(rect) = self.block_rect() else {
            return;
        };
        let value = self.block_register(rect, |_, _| {});
        self.yank_register(register, value);
        self.mode = Mode::Normal;
        self.apply_pending_diagnostics();
        self.land_at_cell(rect.first_line, rect.left_cell);
    }
    pub(crate) fn block_delete(&mut self, register: Option<char>) {
        self.block_edit(false, register);
    }
    pub(crate) fn block_change(&mut self, register: Option<char>) {
        self.block_edit(true, register);
    }
    fn block_edit(&mut self, insert: bool, register: Option<char>) {
        let Some(rect) = self.block_rect() else {
            return;
        };
        let mut rows = Vec::new();
        let mut edits = Vec::new();
        let value = self.block_register(rect, |line, part| {
            if insert && line != rect.first_line && !part.short {
                rows.push(line);
            }
            if !part.range.is_empty() {
                edits.push(Replacement::new(part.range, " ".repeat(part.retained)));
            }
        });
        if let Err(error) = self.apply(
            self.current(),
            self.buf().revision(),
            ChangeSet {
                edits,
                undo_open: insert,
            },
        ) {
            self.message = error.to_string();
            return;
        }
        self.delete_register(register, value, false);
        self.mode = if insert { Mode::Insert } else { Mode::Normal };
        self.land_at_cell(rect.first_line, rect.left_cell);
        if insert {
            self.block_insert_state = Some(BlockInsertState {
                rows,
                column: rect.left_cell,
                pad: false,
                at_line_ends: false,
                home: None,
            });
            self.enter_insert_from("<c-v>c");
        }
    }

    /// Inserting within a tab/wide cluster preserves both outside portions as
    /// cells. A pads short lines; I and c skip rows that never reached the column.
    fn insertion(
        &self,
        line: usize,
        column: DisplayColumn,
        text: &str,
        pad: bool,
    ) -> Option<Replacement> {
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let mut width = DisplayColumn::new(0);
        for (span, cluster) in RopeGraphemes::new(
            self.buf().text().byte_slice(start..end),
            self.cur_indent().width,
        ) {
            if span.cell == column {
                return Some(Replacement::new(
                    Range::charwise(start + span.byte, start + span.byte),
                    text,
                ));
            }
            width = span.cell + span.width;
            if span.cell < column && column < width {
                return Some(Replacement::new(
                    Range::charwise(start + span.byte, start + span.byte + cluster.len()),
                    format!(
                        "{}{}{}",
                        " ".repeat(column - span.cell),
                        text,
                        " ".repeat(width - column)
                    ),
                ));
            }
        }
        if column > width && !pad {
            return None;
        }
        Some(Replacement::new(
            Range::charwise(end, end),
            format!(
                "{}{text}",
                " ".repeat(column.get().saturating_sub(width.get()))
            ),
        ))
    }

    pub(crate) fn block_insert(&mut self, right: bool) {
        let Some(rect) = self.block_rect() else {
            return;
        };
        let column = if right {
            let Some(last) = rect.right_cell else {
                return self.block_append_at_ends(rect);
            };
            last + 1
        } else {
            rect.left_cell
        };
        let rows = (rect.first_line + 1..=rect.last_line)
            .filter(|&line| self.insertion(line, column, "", right).is_some())
            .collect();
        if let Some(edit) = self.insertion(rect.first_line, column, "", true) {
            if let Err(error) = self.apply(
                self.current(),
                self.buf().revision(),
                ChangeSet {
                    edits: vec![edit],
                    undo_open: true,
                },
            ) {
                self.message = error.to_string();
                return;
            }
        }
        self.mode = Mode::Insert;
        self.land_at_cell(rect.first_line, column);
        self.block_insert_state = Some(BlockInsertState {
            rows,
            column,
            pad: right,
            at_line_ends: false,
            home: Some((rect.first_line, rect.left_cell)),
        });
        self.enter_insert_from("<c-v>I");
    }

    pub(crate) fn block_replicate(&mut self, typed: &str) {
        let Some(state) = self.block_insert_state.take() else {
            return;
        };
        if typed.is_empty() {
            return;
        }
        let edits = state
            .rows
            .into_iter()
            .filter_map(|line| {
                if line > self.buf().last_content_line() {
                    return None;
                }
                if state.at_line_ends {
                    let end = self.buf().line_end(line);
                    return Some(strop_core::Replacement::new(
                        Range::charwise(end, end),
                        typed.to_owned(),
                    ));
                }
                self.insertion(line, state.column, typed, state.pad)
            })
            .collect();
        if let Err(error) = self.apply(
            self.current(),
            self.buf().revision(),
            ChangeSet {
                edits,
                undo_open: true,
            },
        ) {
            self.message = error.to_string();
        }
        if let Some((line, column)) = state.home {
            self.land_at_cell(line, column);
        }
    }
}
