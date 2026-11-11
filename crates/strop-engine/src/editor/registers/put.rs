//! Put (`p`/`P`): a register lands per its shape — charwise and
//! linewise text at every cursor, a block as its rectangle.

use strop_core::id::DisplayColumn;
use strop_core::layout::LineLayout;

use super::{Register, RegisterShape};
use crate::editor::Editor;

impl Editor {
    /// Paste one register's content without storing it (bracketed
    /// paste and clipboard results never touch the register file —
    /// vim's rule).
    pub(super) fn paste_register(&mut self, register: &Register, count: usize, before: bool) {
        match register.shape {
            RegisterShape::Blockwise { width } => {
                self.paste_blockwise(register, width, count, before)
            }
            shape @ (RegisterShape::Characterwise | RegisterShape::Linewise) => self.paste_text(
                register.text.repeat(count),
                shape,
                before,
                if shape == RegisterShape::Linewise {
                    register.file_provenance.as_ref()
                } else {
                    None
                },
                count,
            ),
        }
    }

    /// Insertion point, landing byte and exact bytes for one cursor's
    /// charwise/linewise paste. A linewise paste below a final row
    /// without its separator grows one (vim's `p` at EOF); the cursor
    /// lands on the first non-blank of the first pasted row.
    fn paste_points(&self, cursor: usize, text: &str, shape: RegisterShape, before: bool) -> Spot {
        match shape {
            RegisterShape::Linewise => {
                let line = self.buf().line_of(cursor);
                let (at, prefix) = if before {
                    (self.buf().line_start(line), 0usize)
                } else if line + 1 < self.buf().len_lines() {
                    (self.buf().line_start(line + 1), 0usize)
                } else {
                    // below the last row: the buffer gains the break it
                    // lacks, then the pasted rows (vim appends the
                    // newline at EOF; an empty buffer keeps vim's
                    // leading empty row)
                    let ends_break = self.buf().len_bytes() > 0
                        && self.buf().byte(self.buf().len_bytes() - 1) == b'\n';
                    let prefix = if ends_break {
                        0
                    } else {
                        self.newline_str().len()
                    };
                    (self.buf().len_bytes(), prefix)
                };
                // vim: first non-blank of the first pasted row
                let row = &text[prefix.min(text.len())..];
                let blank = row
                    .chars()
                    .take_while(|c| *c == ' ' || *c == '\t')
                    .map(char::len_utf8)
                    .sum::<usize>()
                    .min(row.len());
                Spot {
                    at,
                    land: at + prefix + blank,
                    text: if prefix == 0 {
                        text.to_string()
                    } else {
                        format!("{}{}", self.newline_str(), text)
                    },
                }
            }
            RegisterShape::Characterwise => {
                let at = if before {
                    cursor
                } else {
                    // 0020 §10: after the cursor means after the CHAR — a
                    // byte step from a multibyte lead inserted before it
                    self.buf()
                        .ceil_boundary(cursor + 1)
                        .min(self.buf().len_bytes())
                };
                // vim: the cursor lands on the LAST pasted char, both p
                // and P — the whole char, never a mid-cluster byte
                let last = text.chars().next_back().map_or(0, char::len_utf8);
                Spot {
                    at,
                    land: at + text.len().saturating_sub(last),
                    text: text.to_string(),
                }
            }
            RegisterShape::Blockwise { .. } => unreachable!("blockwise pastes its own way"),
        }
    }

    fn paste_text(
        &mut self,
        text: String,
        shape: RegisterShape,
        before: bool,
        provenance: Option<&std::sync::Arc<crate::editor::filesystem::draft::FileRegister>>,
        count: usize,
    ) {
        // nvim rule: every command is one undo unit — a lone paste must
        // commit its own revision (it used to ride the *next* command's)
        self.tx_begin();
        let cursors = self.all_cursors();
        if cursors.len() == 1 {
            let spot = self.paste_points(self.head(), &text, shape, before);
            self.filename_paste_hint(
                spot.at,
                &spot.text,
                usize::from(spot.text.len() > text.len()),
                provenance,
                count,
            );
            self.buf_mut().insert(spot.at, &spot.text);
            self.clear_filename_hint();
            self.set_head(spot.land);
            self.clamp_cursor();
            self.tx_commit();
            return;
        }
        // multicursor paste (0013 §3): the same register at every
        // cursor, bottom-up so insertion points stay valid mid-batch
        // (a spot below a final row without its break carries the
        // break it grew — bytes can differ per cursor)
        let primary = self.head();
        let mut jobs: Vec<(usize, usize, bool, String)> = cursors
            .into_iter()
            .map(|c| {
                let spot = self.paste_points(c, &text, shape, before);
                (spot.at, spot.land, c == primary, spot.text)
            })
            .collect();
        jobs.sort_by_key(|j| j.0);
        jobs.dedup_by_key(|j| j.0); // stacked cursors paste once
                                    // each landing shifts by what lower insertions already added
        let mut shift = 0usize;
        for j in &mut jobs {
            j.1 += shift;
            shift += j.3.len();
        }
        for (at, _, _, insert) in jobs.iter().rev() {
            self.filename_paste_hint(
                *at,
                insert,
                usize::from(insert.len() > text.len()),
                provenance,
                count,
            );
            self.buf_mut().insert(*at, insert);
            self.clear_filename_hint();
        }
        self.sels_mut()
            .set_extras(jobs.iter().filter(|j| !j.2).map(|j| j.1));
        self.set_head(jobs.iter().find(|j| j.2).map(|j| j.1).unwrap_or(primary));
        self.normalize_cursors();
        self.clamp_cursor();
        self.tx_commit();
    }

    /// Blockwise put (vim's counted block paste): every register row
    /// lands at one DISPLAY column on consecutive rows — the same
    /// streamed layout with the configured tab stops measures the
    /// column on every row, so tabs and wide clusters cannot skew it.
    /// Short rows grow spaces to the column; rows past the end create
    /// terminated rows; `count` repeats the rectangle horizontally;
    /// the whole put is one undo unit and the cursor lands on the
    /// first pasted cluster.
    fn paste_blockwise(
        &mut self,
        register: &Register,
        width: DisplayColumn,
        count: usize,
        before: bool,
    ) {
        let tab = self.cur_indent().width.max(1);
        // rectangles are primary-only (vim has no multicursor block):
        // extras collapse, the head anchors the column
        self.sels_mut().collapse_extras();
        let cursor = self.head();
        let line = self.buf().line_of(cursor);
        let head_row = self.buf().line_text(line);
        let layout = LineLayout::build(&head_row, tab);
        let col = self.buf().col_of(cursor);
        let column = if before || head_row.is_empty() {
            // `P` lands at the cursor's cell; `p` after an empty row's
            // cursor lands at cell 0 — there is no cluster to be after
            layout.cell_at_byte(col)
        } else {
            // after the cursor means after its whole cluster (0020 §10)
            let cluster = layout.spans().iter().rev().find(|s| s.byte <= col);
            layout.cell_at_byte(col) + cluster.map_or(0, |s| s.width)
        };
        let rows: Vec<&str> = register.text.split('\n').collect();
        let sep = self.newline_str();
        let len = self.buf().len_bytes();
        let ends_break = len > 0 && self.buf().byte(len - 1) == b'\n';
        // rows at or past the buffer's phantom final row merge into ONE
        // tail edit at the end: a terminated buffer's phantom row keeps
        // the break that already precedes it, later rows grow their own
        let tail_from = if ends_break {
            self.buf().len_lines().saturating_sub(1)
        } else {
            self.buf().len_lines()
        };
        let mut edits: Vec<(usize, String)> = Vec::with_capacity(rows.len() + 1);
        let mut tail = String::new();
        let mut row0_land = cursor;
        for (i, row) in rows.iter().enumerate() {
            let target = line + i;
            if target < tail_from {
                let start = self.buf().line_start(target);
                let end = self.buf().line_end(target);
                let text = self.buf().line_text(target);
                let row_layout = LineLayout::build(&text, tab);
                // mid-row inserts right-pad the row to the rectangle so
                // following text cannot creep into the block (vim keeps
                // the block rectangular); at the row's end nothing
                // follows and no pad grows
                let mid_row = row_layout.width.get() > column.get();
                let content = block_row(row, width, count, mid_row, tab);
                let (at, content) = if row_layout.width.get() < column.get() {
                    // short row: spaces grow the row out to the column
                    let pad = " ".repeat(column.get() - row_layout.width.get());
                    (end, format!("{pad}{content}"))
                } else {
                    // a cell inside a wide cluster or a tab lands the row
                    // at that cluster's start — clusters never split
                    (start + row_layout.byte_at_cell(column), content)
                };
                if i == 0 {
                    row0_land = at;
                }
                edits.push((at, content));
            } else {
                if !(tail.is_empty() && ends_break && target == tail_from) {
                    // the phantom row keeps its existing break; every
                    // created row grows one
                    tail.push_str(sep);
                }
                if column.get() > 0 {
                    tail.push_str(&" ".repeat(column.get()));
                }
                tail.push_str(&block_row(row, width, count, false, tab));
                if i == 0 {
                    row0_land = len
                        + if ends_break && target == tail_from {
                            0
                        } else {
                            sep.len()
                        };
                }
            }
        }
        if !tail.is_empty() {
            // created rows are terminated rows (vim's line model — the
            // buffer gains the final break it lacks)
            tail.push_str(sep);
            edits.push((len, tail));
        }
        self.tx_begin();
        for (at, text) in edits.iter().rev() {
            self.buf_mut().insert(*at, text);
        }
        self.set_head(row0_land);
        self.clamp_cursor();
        self.tx_commit();
    }
}

/// One cursor's paste: insertion byte, landing byte, and the exact
/// bytes that land (a paste below a final row without its break grows
/// one first — vim's `p` at EOF).
struct Spot {
    at: usize,
    land: usize,
    text: String,
}

/// One pasted block row's bytes. Mid-row puts right-pad the row to the
/// rectangle before repeating it `count` times (vim's counted block
/// put repeats the PADDED row horizontally); puts at a row's end or on
/// created rows repeat the bare row.
fn block_row(row: &str, width: DisplayColumn, count: usize, mid_row: bool, tab: usize) -> String {
    let mut unit = row.to_string();
    if mid_row {
        let cells = LineLayout::build(row, tab).width.get();
        let pad = width.get().saturating_sub(cells);
        unit.push_str(&" ".repeat(pad));
    }
    unit.repeat(count)
}
