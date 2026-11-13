//! Moving focus between panes (plans/0104 §3, §5.5): by number, in order,
//! and by direction — `ctrl-w h j k l` go to the window that touches the
//! current one on that side, at the cursor's row or column, and stop at the
//! edge (vim's `win_horz_neighbor` / `win_vert_neighbor`).

use crate::editor::prepare::CellRect;
use crate::editor::Editor;

use super::layout;

impl Editor {
    /// `Space w` navigation: h/l/j/k direction, w cycle, 1-9 direct focus.
    pub(crate) fn pane_move(&mut self, key: char) {
        let n = self.panes.len();
        if n < 2 {
            self.message = "no other pane".into();
            return;
        }
        let next = match key {
            '1'..='9' => {
                let index = key as usize - '1' as usize;
                if index >= n {
                    self.message = format!("no pane {key}");
                    return;
                }
                index
            }
            'h' | 'j' | 'k' | 'l' => match self.pane_towards(key) {
                Some(index) => index,
                None => return,
            },
            'w' => (self.active_pane + 1) % n,
            _ => return,
        };
        self.focus_pane(next);
    }

    /// Focus a pane by index (0056 AR02): the admitted form of a
    /// frontend pointer/focus action. State is already per-pane: no
    /// sync. Unknown indexes are ignored.
    pub fn focus_pane(&mut self, index: usize) {
        if index >= self.panes.len() {
            return;
        }
        if index != self.active_pane {
            self.cancel_pending();
            self.panes[self.active_pane].focused = self.focus_epoch;
        }
        self.active_pane = index;
        self.focus_epoch += 1;
        self.cancel_open(strop_core::worker::CancelReason::Superseded);
        self.discover_git();
        self.clamp_cursor();
    }

    /// The pane on side `key` (`h j k l`) of the active one, at the
    /// cursor's row or column; `None` at the edge.
    pub(crate) fn pane_towards(&self, key: char) -> Option<usize> {
        // the prepared geometry, widened to the tree's minimum so every
        // window has cells even before the first prepare
        let geometry = self.prepared.geometry;
        let (columns, rows) = layout::minimum(&self.layout);
        let area = CellRect::new(
            0,
            0,
            u16::try_from(columns.max(usize::from(geometry.columns))).unwrap_or(u16::MAX),
            u16::try_from(rows.max(usize::from(geometry.rows.saturating_sub(1))) + 1)
                .unwrap_or(u16::MAX),
        );
        let frames = self.layout.place(area).frames;
        let frame = *frames.get(self.active_pane)?;
        let (cx, cy) = self.cursor_cell(frame);
        let (x, y) = match key {
            'j' => (cx, usize::from(frame.y) + usize::from(frame.height)),
            'k' => (cx, usize::from(frame.y).checked_sub(1)?),
            'l' => (usize::from(frame.x) + usize::from(frame.width) + 1, cy),
            'h' => (usize::from(frame.x).checked_sub(2)?, cy),
            _ => return None,
        };
        // a divider column belongs to the frame on its left
        frames.iter().position(|other| {
            let left = usize::from(other.x);
            let top = usize::from(other.y);
            (left..=left + usize::from(other.width)).contains(&x)
                && (top..top + usize::from(other.height)).contains(&y)
        })
    }

    /// The cursor's cell inside the active pane's frame (title row first).
    fn cursor_cell(&self, frame: CellRect) -> (usize, usize) {
        let pane = self.view();
        let x = usize::from(frame.x);
        let y = usize::from(frame.y);
        let width = usize::from(frame.width).max(1);
        let height = usize::from(frame.height).max(2);
        let head = pane.sels.primary().head;
        let column = self
            .docs
            .get(pane.doc)
            .and_then(|document| {
                document
                    .buf
                    .try_cell_col_with_tab(head, self.indentation_at(pane.doc, head).width)
            })
            .map_or(0, |column| {
                (self.left_inset(pane.doc) + column.get()).saturating_sub(pane.hscroll.get())
            });
        let row = self
            .docs
            .get(pane.doc)
            .map_or(0, |document| document.buf.line_of(head))
            .saturating_sub(pane.view_top);
        (x + column.min(width - 1), y + 1 + row.min(height - 2))
    }
}
