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
}
struct RowPart {
    range: Option<Range>,
    selected: String,
    remaining: String,
}

impl Editor {
    pub fn enter_block_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let head = self.head();
        self.sels_mut().stretch_primary(head, head);
        self.view_mut().desired_column = None;
        self.mode = Mode::VisualBlock;
    }
    pub fn block_rect_pub(&self) -> Option<BlockRect> {
        self.block_rect()
    }

    fn cluster_cells(&self, byte: usize) -> (DisplayColumn, DisplayColumn) {
        let line = self.buf().line_of(byte);
        let start = self.buf().line_start(line);
        let slice = self
            .buf()
            .text()
            .byte_slice(start..self.buf().line_end(line));
        let relative = byte.saturating_sub(start);
        let mut end = DisplayColumn::new(0);
        for (span, cluster) in RopeGraphemes::new(slice, self.config.tab_size) {
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
            right_cell: a_end.max(h_end) - 1,
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
        let desired = self
            .view()
            .desired_column
            .unwrap_or_else(|| self.cluster_cells(self.head()).1 - 1);
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

    fn row_part(&self, line: usize, left: DisplayColumn, right: DisplayColumn) -> RowPart {
        let start = self.buf().line_start(line);
        let slice = self
            .buf()
            .text()
            .byte_slice(start..self.buf().line_end(line));
        let mut first = None;
        let mut last = start;
        let mut selected = String::new();
        let mut remaining = String::new();
        for (span, text) in RopeGraphemes::new(slice, self.config.tab_size) {
            let end = span.cell + span.width;
            if end <= left || span.cell >= right || span.width == 0 {
                continue;
            }
            first.get_or_insert(start + span.byte);
            last = start + span.byte + text.len();
            let lo = span.cell.max(left);
            let hi = end.min(right);
            if lo == span.cell && hi == end {
                selected.push_str(&text);
            } else {
                selected.push_str(&" ".repeat(hi - lo));
                remaining.push_str(&" ".repeat(lo - span.cell));
                remaining.push_str(&" ".repeat(end - hi));
            }
        }
        if first.is_none() {
            selected = " ".repeat(right - left);
        }
        RowPart {
            range: first.map(|first| Range::charwise(first, last)),
            selected,
            remaining,
        }
    }

    fn land_at_cell(&mut self, line: usize, column: DisplayColumn) {
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let byte = RopeGraphemes::new(
            self.buf().text().byte_slice(start..end),
            self.config.tab_size,
        )
        .find_map(|(span, _)| (span.cell + span.width > column).then_some(start + span.byte))
        .unwrap_or(end);
        self.set_head(byte);
        self.clamp_cursor();
    }

    pub(crate) fn block_yank(&mut self) {
        let Some(rect) = self.block_rect() else {
            return;
        };
        let rows: Vec<_> = (rect.first_line..=rect.last_line)
            .map(|line| {
                self.row_part(line, rect.left_cell, rect.right_cell + 1)
                    .selected
            })
            .collect();
        self.set_register(
            None,
            Register::blockwise(
                rows.join("\n"),
                DisplayColumn::new(rect.right_cell - rect.left_cell + 1),
            ),
        );
        self.mode = Mode::Normal;
        self.land_at_cell(rect.first_line, rect.left_cell);
    }
    pub(crate) fn block_delete(&mut self) {
        self.block_edit(false);
    }
    pub(crate) fn block_change(&mut self) {
        self.block_edit(true);
    }
    fn block_edit(&mut self, insert: bool) {
        let Some(rect) = self.block_rect() else {
            return;
        };
        let mut rows = Vec::new();
        let mut selected = Vec::new();
        let mut edits = Vec::new();
        for line in rect.first_line..=rect.last_line {
            let part = self.row_part(line, rect.left_cell, rect.right_cell + 1);
            selected.push(part.selected);
            if let Some(range) = part.range {
                if line != rect.first_line {
                    rows.push(line);
                }
                edits.push(Replacement::new(range, part.remaining));
            }
        }
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
        self.set_register(
            None,
            Register::blockwise(
                selected.join("\n"),
                DisplayColumn::new(rect.right_cell - rect.left_cell + 1),
            ),
        );
        self.mode = if insert { Mode::Insert } else { Mode::Normal };
        self.land_at_cell(rect.first_line, rect.left_cell);
        if insert {
            self.block_insert_state = Some(BlockInsertState {
                rows,
                column: rect.left_cell,
                pad: false,
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
            self.config.tab_size,
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
            rect.right_cell + 1
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
    }
}
