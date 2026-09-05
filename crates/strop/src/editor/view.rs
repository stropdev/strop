//! Viewport motions and the jump-back cluster (the vim canon wave):
//! `Ctrl-d/u/f/b` scroll, `zz/zt/zb` + `H/M/L` viewport placement,
//! `ZZ` write-quit, `Ctrl-^` alternate buffer, `gv`/`gi` re-entry,
//! `g;`/`g,` the change list. Viewport geometry comes from
//! `scroll_to_cursor`, which the render loop feeds every frame — the
//! pane's `view_rows` is the editor's single source of truth for
//! "how tall is the screen".

use super::{Editor, Mode};

impl Editor {
    /// Half-page scroll (vim ctrl-d/ctrl-u): the cursor moves with the
    /// view so its screen row holds.
    pub(crate) fn scroll_half_page(&mut self, down: bool) {
        self.scroll_by(self.view_rows().max(2) / 2, down);
    }

    /// Full-page scroll (vim ctrl-f/ctrl-b).
    pub(crate) fn scroll_full_page(&mut self, down: bool) {
        self.scroll_by(self.view_rows().saturating_sub(1).max(1), down);
    }

    pub(crate) fn scroll_by(&mut self, rows: usize, down: bool) {
        let line = self.buf().line_of(self.head());
        let last = self.buf().last_content_line();
        let line = if down {
            (line + rows).min(last)
        } else {
            line.saturating_sub(rows)
        };
        let col = self.buf().col_of(self.head());
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        self.set_head((start + col).min(end));
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
    }

    /// `zz`: center the cursor line; `zt`: to the top; `zb`: bottom.
    pub(crate) fn view_place(&mut self, place: char) {
        let line = self.buf().line_of(self.head());
        let rows = self.view_rows();
        let last = self.buf().last_content_line();
        let top = match place {
            't' => line,
            'b' => (line + 1).saturating_sub(rows),
            _ => (line + 1).saturating_sub(rows.max(2) / 2 + 1),
        };
        // never scroll past the content end (vim keeps the tail full)
        let max_top = (last + 1).saturating_sub(rows);
        self.view_mut().view_top = top.min(max_top);
    }

    /// `H`/`M`/`L`: jump to the top/middle/bottom VISIBLE line (vim:
    /// first non-blank of that line; a count on H/L offsets).
    pub(crate) fn jump_visible(&mut self, which: char, count: usize) {
        let rows = self.view_rows();
        let last = self.buf().last_content_line();
        let line = match which {
            'H' => self.view_top() + count.saturating_sub(1),
            'L' => {
                (self.view_top() + rows.saturating_sub(1)).saturating_sub(count.saturating_sub(1))
            }
            _ => self.view_top() + rows.max(2) / 2,
        }
        .min(last);
        let s = self.buf().line_start(line);
        let e = self.buf().line_end(line);
        // first non-blank, like vim
        let mut p = s;
        while p < e
            && self
                .buf()
                .byte_at(p)
                .is_some_and(|b| b == b' ' || b == b'\t')
        {
            p += 1;
        }
        self.set_head(p.min(e));
        self.clamp_cursor();
    }

    /// Counted scroll dispatch (vim: <count>ctrl-d scrolls count lines).
    pub(crate) fn scroll_counted(&mut self, which: char, n: usize) {
        let counted = n > 1;
        match (which, counted) {
            ('d', true) => self.scroll_by(n, true),
            ('u', true) => self.scroll_by(n, false),
            ('d', false) => self.scroll_half_page(true),
            ('u', false) => self.scroll_half_page(false),
            ('f', _) => self.scroll_full_page(true),
            _ => self.scroll_full_page(false),
        }
    }

    /// `ZZ`: write + close the window (vim's :x — only writes when
    /// dirty; a failed save keeps everything open).
    pub(crate) fn write_quit(&mut self) {
        if self.buf().dirty {
            if let Err(e) = self.buf_mut().save(false) {
                self.message = format!("write failed: {e}");
                return;
            }
            crate::session::save(self);
        }
        self.close_pane_or_buffer(false);
    }

    /// `Ctrl-^`: the alternate buffer (most-recently-used other doc).
    pub(crate) fn alternate_buffer(&mut self) {
        let cur = self.current();
        let Some(&alt) = self.mru.iter().find(|&&id| id != cur) else {
            self.message = "no alternate buffer".into();
            return;
        };
        self.switch_to(alt);
        self.clamp_cursor();
    }

    /// `gv`: reselect the last visual range exactly (vim).
    pub(crate) fn reselect_visual(&mut self) {
        let Some((anchor, head)) = self.last_visual else {
            self.message = "no previous visual selection".into();
            return;
        };
        let max = self.buf().len_bytes();
        let (a, h) = (anchor.min(max), head.min(max));
        let a = self.buf().clamp_boundary(a);
        let h = self.buf().clamp_boundary(h);
        self.sels_mut().stretch_primary(a, h);
        self.mode = Mode::Visual;
    }

    /// `gi`: insert where the last insert session ended (vim).
    pub(crate) fn insert_at_last(&mut self) {
        let Some(pos) = self.last_insert_pos else {
            self.message = "no previous insert".into();
            return;
        };
        let pos = self.buf().clamp_boundary(pos.min(self.buf().len_bytes()));
        self.set_head(pos);
        self.mode = Mode::Insert;
    }

    /// `g;` / `g,`: walk the change list — derived from the undo
    /// history's ancestor chain, no parallel recording (0015 doctrine:
    /// one source of truth).
    pub(crate) fn change_jump(&mut self, back: bool) {
        let positions = self.change_positions();
        if positions.is_empty() {
            self.message = "change list is empty".into();
            return;
        }
        // g; starts at the newest change and walks older; g, back
        // newer. A new commit invalidates the walk (the idx is only
        // meaningful against the depth it was taken at).
        let depth = self.buf().history.depth();
        let idx = match self.change_idx.filter(|(_, d)| *d == depth) {
            None => {
                if back {
                    0
                } else {
                    self.message = "at the newest change".into();
                    return;
                }
            }
            Some((i, _)) => {
                if back {
                    (i + 1).min(positions.len() - 1)
                } else {
                    i.saturating_sub(1)
                }
            }
        };
        self.change_idx = Some((idx, depth));
        let pos = positions[idx];
        let pos = self.buf().clamp_boundary(pos.min(self.buf().len_bytes()));
        self.set_head(pos);
        self.clamp_cursor();
        self.scroll_to_cursor(self.view_rows());
    }

    /// Ancestor-chain change positions, newest first.
    fn change_positions(&self) -> Vec<usize> {
        self.buf().history.change_positions()
    }
}
