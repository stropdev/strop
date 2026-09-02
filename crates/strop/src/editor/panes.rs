//! Splits (0001 pillar 4: splits are core vim grammar). v1: a flat row
//! (`:vs`, side by side) or column (`:sp`, stacked) — mixed nesting is
//! the tree-layout follow-up. Buffers are shared between panes; cursor
//! and view offset are per-pane.

use super::Editor;

/// One pane's view state.
#[derive(Debug, Clone)]
pub struct Pane {
    pub buffer: usize,
    pub cursor: usize,
    pub view_top: usize,
}

/// v1 is a flat layout: Row = vertical splits side by side,
/// Column = horizontal splits stacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LayoutDir {
    Row,
    Column,
}

impl Editor {
    /// Split the active pane. `vertical` = `:vs` (new pane to the right).
    /// Without a path the pane shows the same buffer (the split point).
    pub(crate) fn split(&mut self, vertical: bool, path: Option<&str>) {
        self.sync_to_pane(); // capture the active pane before opening another
        if let Some(p) = path {
            if let Err(e) = self.open_buffer(p) {
                self.message = format!("open {p}: {e}");
                return;
            }
        }
        let (cursor, view_top) = (self.cursor, self.view_top);
        self.panes.push(Pane {
            buffer: self.current,
            cursor,
            view_top,
        });
        self.layout = if vertical {
            LayoutDir::Row
        } else {
            LayoutDir::Column
        };
        self.active_pane = self.panes.len() - 1;
    }

    /// `:q` closes the pane; the last pane's close is buffer close.
    pub(crate) fn close_pane_or_buffer(&mut self, force: bool) {
        if self.panes.len() > 1 {
            self.panes.remove(self.active_pane);
            self.active_pane = self.active_pane.min(self.panes.len() - 1);
            self.sync_from_pane();
        } else {
            self.close_buffer(force);
        }
    }

    /// `C-w` navigation: h/l/j/k direction, w cycle.
    pub(crate) fn pane_move(&mut self, key: char) {
        let n = self.panes.len();
        if n < 2 {
            self.message = "no other pane".into();
            return;
        }
        let next = match (self.layout, key) {
            (LayoutDir::Row, 'h') => self.active_pane.checked_sub(1).unwrap_or(n - 1),
            (LayoutDir::Row, 'l') => (self.active_pane + 1) % n,
            (LayoutDir::Column, 'k') => self.active_pane.checked_sub(1).unwrap_or(n - 1),
            (LayoutDir::Column, 'j') => (self.active_pane + 1) % n,
            (_, 'w') => (self.active_pane + 1) % n,
            _ => return,
        };
        self.sync_to_pane();
        self.active_pane = next;
        self.sync_from_pane();
    }

    /// Save the active pane's state before switching away.
    fn sync_to_pane(&mut self) {
        let pane = &mut self.panes[self.active_pane];
        pane.buffer = self.current;
        pane.cursor = self.cursor;
        pane.view_top = self.view_top;
    }

    fn sync_from_pane(&mut self) {
        let pane = self.panes[self.active_pane].clone();
        if pane.buffer < self.buffers.len() {
            self.current = pane.buffer;
        }
        self.cursor = pane.cursor.min(self.buf().len_bytes());
        self.view_top = pane.view_top;
        self.clamp_cursor();
        self.discover_git();
        let _ = self.highlighters.get(self.current); // highlighters stay per-buffer
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn vsplit_shares_buffer_and_navigates() {
        std::fs::write("/tmp/strop-split-a.rs", "fn a() {}\nfn b() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open("/tmp/strop-split-a.rs").unwrap());
        e.feed_text("j"); // line 2
        e.feed_text(":vs<cr>");
        assert_eq!(e.panes.len(), 2);
        assert_eq!(e.active_pane, 1);
        // the new pane shows the same buffer from its own view
        e.feed_text("gg");
        // C-w back to the first pane — it kept its cursor
        e.feed(crate::editor::Key::CtrlW);
        e.feed(crate::editor::Key::Char('h'));
        assert_eq!(e.active_pane, 0);
        assert_eq!(e.buf().line_of(e.cursor), 1, "pane 1 kept its own cursor");
        // :q closes the pane, buffer stays
        e.feed_text(":q<cr>");
        assert_eq!(e.panes.len(), 1);
        assert_eq!(e.buffers.len(), 1);
    }

    #[test]
    fn split_with_path_opens_other_file() {
        std::fs::write("/tmp/strop-split-a.rs", "fn a() {}\n").unwrap();
        std::fs::write("/tmp/strop-split-b.rs", "fn b() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open("/tmp/strop-split-a.rs").unwrap());
        e.feed_text(":vs /tmp/strop-split-b.rs<cr>");
        assert_eq!(e.panes.len(), 2);
        assert_eq!(e.buf().path.as_deref(), Some("/tmp/strop-split-b.rs"));
        e.feed(crate::editor::Key::CtrlW);
        e.feed(crate::editor::Key::Char('h'));
        assert_eq!(e.buf().path.as_deref(), Some("/tmp/strop-split-a.rs"));
    }
}
