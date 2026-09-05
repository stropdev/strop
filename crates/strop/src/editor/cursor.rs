//! Selection-facing helpers (0014 wave 2): head/anchor accessors,
//! clamps, the multicursor cascade bookkeeping, scroll, flash.

use std::time::Instant;

use strop_core::Range;

use super::{Editor, Mode, FLASH_FOR};

impl Editor {
    /// The primary cursor's byte offset (was the `cursor` field).
    #[inline]
    pub fn head(&self) -> usize {
        self.sels().primary().head
    }

    #[inline]
    pub fn set_head(&mut self, pos: usize) {
        self.sels_mut().set_head(pos);
    }

    /// The visual anchor (== head when not in visual mode).
    #[inline]
    pub fn anchor(&self) -> usize {
        self.sels().primary().anchor
    }

    /// Extra selections beyond the primary (0013).
    pub fn extra_selections(&self) -> &[strop_core::selection::Selection] {
        self.sels().extra_heads()
    }

    pub(crate) fn flash(&mut self, range: Range) {
        self.flash = Some((range, Instant::now()));
    }

    pub fn flash_range(&self) -> Option<Range> {
        self.flash
            .and_then(|(r, at)| (at.elapsed() < FLASH_FOR).then_some(r))
    }

    pub fn clamp_cursor(&mut self) {
        let line = self.buf().line_of(self.head());
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let max = if self.mode == Mode::Insert {
            end
        } else {
            end.max(start + 1) - 1
        };
        let pos = self
            .buf()
            .clamp_boundary(self.head().clamp(start, max.max(start)));
        self.set_head(pos);
    }

    /// Clamp one position the way clamp_cursor clamps the primary.
    pub(crate) fn clamp_pos(&self, pos: usize) -> usize {
        let line = self.buf().line_of(pos);
        let start = self.buf().line_start(line);
        let end = self.buf().line_end(line);
        let max = if self.mode == Mode::Insert {
            end
        } else {
            end.max(start + 1) - 1
        };
        self.buf().clamp_boundary(pos.clamp(start, max.max(start)))
    }

    /// Every cursor position, primary first (0013 §3).
    pub(crate) fn all_cursors(&self) -> Vec<usize> {
        self.sels().heads()
    }

    /// Restore the invariant after any cascade: sorted and deduped. An
    /// extra MAY sit on the primary (Q plants there, then you move) —
    /// edit cascades dedupe positions before applying.
    pub(crate) fn normalize_cursors(&mut self) {
        self.sels_mut().normalize();
    }

    /// Remap cursors after a mirrored edit of `delta` bytes at each of
    /// `positions` (pre-edit, sorted, deduped): every cursor shifts by
    /// its own edit plus every edit below it (0013 §3).
    pub(crate) fn remap_after_mirrored_edit(&mut self, positions: &[usize], delta: isize) {
        let map = |old: usize| -> usize {
            let below = positions.partition_point(|&p| p < old);
            let own = usize::from(positions.contains(&old));
            (old as isize + delta * (below + own) as isize).max(0) as usize
        };
        self.set_head(map(self.head()));
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| map(s.head))
            .collect();
        self.sels_mut().set_extras(extras);
    }

    /// `Q`: drop the cursor under point when one exists, else plant one.
    pub(crate) fn toggle_cursor(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        self.sels_mut().toggle_extra();
        let n = self.sels().count();
        self.message = format!("{n} cursor{}", if n > 1 { "s" } else { "" });
    }

    /// `Space c` (helix's `C`): copy the primary cursor onto the same
    /// column of the next line — how vertical cursor stacks are built.
    pub(crate) fn add_cursor_next_line(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        // stack from the bottom-most cursor (helix C semantics: repeated
        // presses walk down the buffer)
        let base = self
            .extra_selections()
            .last()
            .map(|s| s.head)
            .unwrap_or_else(|| self.head());
        let line = self.buf().line_of(base);
        // the phantom line past a trailing newline is not a cursor home
        if line + 1 >= self.buf().len_lines()
            || self.buf().line_start(line + 1) >= self.buf().len_bytes()
        {
            self.message = "no line below".into();
            return;
        }
        let col = self.buf().col_of(base);
        let start = self.buf().line_start(line + 1);
        let end = self.buf().line_end(line + 1);
        let pos = (start + col).min(end.saturating_sub(1).max(start));
        self.sels_mut().plant_extra(pos);
        let n = self.sels().count();
        self.message = format!("{n} cursors");
    }

    /// Normal-mode Esc: collapse to the primary cursor (0013 §3).
    pub(crate) fn collapse_cursors(&mut self) {
        if self.sels().count() > 1 {
            self.sels_mut().collapse_extras();
            self.message = "1 cursor".into();
        }
    }

    /// Keep the cursor on screen; `rows` = text area height. The
    /// render loop calls this every frame, so it doubles as the
    /// viewport-height feed for the H/M/L/zz/ctrl-d family.
    pub fn scroll_to_cursor(&mut self, rows: usize) {
        self.view_rows = rows;
        let line = self.buf().line_of(self.head());
        if line < self.view_top() {
            self.view_mut().view_top = line;
        } else if line >= self.view_top() + rows {
            self.view_mut().view_top = line + 1 - rows;
        }
    }
}
