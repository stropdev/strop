//! Visual block mode (0017): `ctrl-v` selects a rectangle. The
//! selection stays one primary (anchor/head are the corners); the
//! rectangle derives from CELL columns — wide chars and tabs measure
//! the same on every row because LineLayout is the single seam.

use super::{Editor, Mode};

impl Editor {
    /// `ctrl-v`: enter block mode with an empty rectangle at the cursor.
    pub fn enter_block_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let h = self.head();
        self.sels_mut().stretch_primary(h, h);
        self.mode = Mode::VisualBlock;
    }

    pub fn block_rect_pub(&self) -> Option<(usize, usize, u16, u16)> {
        self.block_rect()
    }

    /// The rectangle: (first line, last line, left cell, right cell).
    /// Cell columns come from the SAME LineLayout on every row — a tab
    /// or wide char can't skew the columns apart.
    pub(crate) fn block_rect(&self) -> Option<(usize, usize, u16, u16)> {
        if self.mode != Mode::VisualBlock {
            return None;
        }
        let (a, h) = (self.anchor(), self.head());
        let (la, lh) = (self.buf().line_of(a), self.buf().line_of(h));
        let (ca, ch) = (self.buf().cell_col_of(a), self.buf().cell_col_of(h));
        Some((la.min(lh), la.max(lh), ca.min(ch), ca.max(ch)))
    }

    /// One rect row's byte range on `line` — None when the line is too
    /// short to reach the rectangle (vim skips short lines on y/d, pads
    /// on I/A... we skip: padding is surprising with real text).
    fn rect_line_bytes(&self, line: usize, cl: u16, cr: u16) -> Option<(usize, usize)> {
        let (s, e) = (self.buf().line_start(line), self.buf().line_end(line));
        let text = self
            .buf()
            .rope
            .byte_slice(s..e)
            .to_string()
            .trim_end_matches('\n')
            .to_string();
        let layout = strop_core::layout::LineLayout::build(&text, 8);
        if layout.width < cl {
            return None;
        }
        let start = s + layout.byte_at_cell(cl);
        let end = s + layout.byte_at_cell(cr + 1).min(layout.len_bytes);
        let end = if cr as usize >= layout.width as usize {
            e.min(s + layout.len_bytes)
        } else {
            end
        };
        Some((start, end.max(start)))
    }

    /// `x`/`d` on the rectangle: per-line delete, one undo unit.
    pub(crate) fn block_delete(&mut self) {
        let Some((la, lh, cl, cr)) = self.block_rect() else {
            return;
        };
        let mut killed = Vec::new();
        self.tx_begin();
        // descend: earlier byte ranges stay valid as later lines edit
        for line in (la..=lh).rev() {
            if let Some((s, e)) = self.rect_line_bytes(line, cl, cr) {
                if e > s {
                    killed.push(self.buf().rope.byte_slice(s..e).to_string());
                    self.buf_mut().delete(strop_core::Range::charwise(s, e));
                }
            }
        }
        self.tx_commit();
        killed.reverse();
        self.set_register(None, killed.join(""), false);
        let s = self
            .buf()
            .line_start(la.min(self.buf().last_content_line()));
        self.set_head(self.buf().clamp_boundary(s));
        self.after_visual_op();
    }

    /// `y` on the rectangle: join the cell-span text of every row.
    pub(crate) fn block_yank(&mut self) {
        let Some((la, lh, cl, cr)) = self.block_rect() else {
            return;
        };
        let mut parts = Vec::new();
        for line in la..=lh {
            if let Some((s, e)) = self.rect_line_bytes(line, cl, cr) {
                parts.push(self.buf().rope.byte_slice(s..e).to_string());
            }
        }
        self.set_register(None, parts.join("\n"), false);
        self.message = format!("{} lines yanked (block)", lh - la + 1);
        self.after_visual_op();
    }

    /// `c` on the rectangle: delete it, insert on every row (the typed
    /// text replicates at Esc — vim's block change).
    pub(crate) fn block_change(&mut self) {
        let Some((la, lh, cl, _)) = self.block_rect() else {
            return;
        };
        self.block_delete_pending = Some((la, lh, cl));
        self.tx_begin();
        for line in (la..=lh).rev() {
            if let Some((s, e)) =
                self.rect_line_bytes(line, cl, self.block_rect().map(|r| r.3).unwrap_or(cl))
            {
                if e > s {
                    self.buf_mut().delete(strop_core::Range::charwise(s, e));
                }
            }
        }
        self.tx_commit();
        let s = self.buf().line_start(la);
        let text = self
            .buf()
            .rope
            .byte_slice(s..self.buf().line_end(la))
            .to_string()
            .trim_end_matches('\n')
            .to_string();
        let layout = strop_core::layout::LineLayout::build(&text, 8);
        self.set_head(s + layout.byte_at_cell(cl).min(layout.len_bytes));
        self.enter_insert_from("<c-v>c");
    }

    /// `I`/`A` on the rectangle: insert at the left/right edge, text
    /// replicating per row at Esc.
    pub(crate) fn block_insert(&mut self, right_edge: bool) {
        let Some((la, lh, cl, cr)) = self.block_rect() else {
            return;
        };
        let cell = if right_edge { cr + 1 } else { cl };
        self.block_delete_pending = Some((la, lh, cell));
        let s = self.buf().line_start(la);
        let text = self
            .buf()
            .rope
            .byte_slice(s..self.buf().line_end(la))
            .to_string()
            .trim_end_matches('\n')
            .to_string();
        let layout = strop_core::layout::LineLayout::build(&text, 8);
        self.set_head(s + layout.byte_at_cell(cell).min(layout.len_bytes));
        self.enter_insert_from("<c-v>I");
    }

    /// Esc from a block insert/change: the typed text lands on every
    /// rect row at its cell (vim). Called from the insert Esc path.
    pub(crate) fn block_replicate(&mut self, typed: &str) {
        let Some((la, lh, cell)) = self.block_delete_pending.take() else {
            return;
        };
        if typed.is_empty() {
            return;
        }
        self.tx_begin();
        for line in (la + 1..=lh).rev() {
            if line > self.buf().last_content_line() {
                continue;
            }
            let s = self.buf().line_start(line);
            let e = self.buf().line_end(line);
            let text = self
                .buf()
                .rope
                .byte_slice(s..e)
                .to_string()
                .trim_end_matches('\n')
                .to_string();
            let layout = strop_core::layout::LineLayout::build(&text, 8);
            let at = s + layout.byte_at_cell(cell).min(layout.len_bytes);
            self.buf_mut().insert(at, typed);
        }
        self.tx_commit();
    }

    /// Shared exit after a block op: normal mode, collapse.
    fn after_visual_op(&mut self) {
        self.mode = Mode::Normal;
        self.clamp_cursor();
    }
}
