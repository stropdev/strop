//! Splits (0001 pillar 4: splits are core vim grammar). v1: a flat row
//! (`:vs`, side by side) or column (`:sp`, stacked) — mixed nesting is
//! the tree-layout follow-up. Documents are shared between panes; the
//! selections and scroll are per-pane (0014: the pane OWNS them — no
//! sync_to/from_pane copy-back, the active pane's state is the editor's).

use strop_core::id::DisplayColumn;
use strop_core::selection::SelectionSet;

use super::Editor;

/// One pane: the document it shows plus its own view state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pane {
    pub doc: strop_core::id::DocumentId,
    pub sels: SelectionSet,
    pub view_top: usize,
    /// Horizontal display-cell origin (0031 R6): glyphs, overlays and
    /// every caret project through this; fixed left margins never do.
    pub hscroll: DisplayColumn,
    /// Desired cell retained while vertical motions cross short/wide rows.
    pub desired_column: Option<DisplayColumn>,
}

impl Pane {
    /// Minimal horizontal scrolling: preserve the origin unless the
    /// caret leaves it. `width` is CONTENT width — every fixed left
    /// margin (sidebar, blame, number gutter) is excluded.
    pub(crate) fn reveal_column(&mut self, column: DisplayColumn, width: usize) {
        if width == 0 {
            return;
        }
        if column < self.hscroll {
            self.hscroll = column;
        } else if column.get() - self.hscroll.get() >= width {
            self.hscroll = DisplayColumn::new(column.get() - (width - 1));
        }
    }
}

/// v1 is a flat layout: Row = vertical splits side by side,
/// Column = horizontal splits stacked.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum LayoutDir {
    Row,
    Column,
}

impl Editor {
    /// The active pane — the editor's selections/scroll ARE its state.
    #[inline]
    pub fn view(&self) -> &Pane {
        &self.panes[self.active_pane]
    }

    #[inline]
    pub fn view_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active_pane]
    }

    /// Split the active pane. `vertical` = `:vs` (new pane to the right).
    /// Without a path the pane shows the same document (the split point).
    pub(crate) fn split(&mut self, vertical: bool, path: Option<&str>) {
        if let Some(path) = path {
            self.request_open(path.into(), super::io::OpenIntent::Split { vertical });
        } else {
            self.split_document(vertical, self.current());
        }
    }
    pub(crate) fn split_document(&mut self, vertical: bool, doc: strop_core::id::DocumentId) {
        // a text prompt belongs to the pane/document it was opened on:
        // splitting away cancels it (R7) before any view state moves
        self.cancel_pending();
        let view = self.view().clone();
        // a same-document split keeps the whole view (hscroll included);
        // a different document starts from a zero origin
        self.panes.push(if doc == view.doc {
            view
        } else {
            Pane {
                doc,
                sels: SelectionSet::default(),
                view_top: 0,
                hscroll: DisplayColumn::new(0),
                desired_column: None,
            }
        });
        self.layout = if vertical {
            LayoutDir::Row
        } else {
            LayoutDir::Column
        };
        self.active_pane = self.panes.len() - 1;
        self.focus_epoch += 1;
        self.discover_git();
        self.lsp_maybe_attach();
    }

    /// `:q` closes the pane; the last pane's close is document close.
    pub(crate) fn close_pane_or_buffer(&mut self, force: bool) {
        if self.panes.len() > 1 {
            self.cancel_pending();
            self.panes.remove(self.active_pane);
            self.active_pane = self.active_pane.min(self.panes.len() - 1);
            self.focus_epoch += 1;
            // the surviving pane's document may differ from the closed
            // pane's — git discovery follows the view, no copy-back
            self.discover_git();
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
        if next != self.active_pane {
            self.cancel_pending();
        }
        self.active_pane = next; // state is already per-pane: no sync
        self.focus_epoch += 1;
        self.discover_git();
        self.clamp_cursor();
    }
}

impl Editor {
    /// Table shims (0008 stage 2): ctrl-w children dispatch by key.
    pub(crate) fn pane_move_pub(&mut self, key: char) {
        self.pane_move(key);
    }
    pub(crate) fn split_pub(&mut self, key: char) {
        self.split(key == 'v', None);
    }
    pub(crate) fn pane_close_pub(&mut self) {
        self.close_pane_or_buffer(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    #[test]
    fn vsplit_shares_buffer_and_navigates() {
        // unique path: parallel tests sharing a fixture file race
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("vsplit-a.rs");
        std::fs::write(&a, "fn a() {}\nfn b() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
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
        assert_eq!(e.buf().line_of(e.head()), 1, "pane 1 kept its own cursor");
        // :q closes the pane, buffer stays
        e.feed_text(":q<cr>");
        assert_eq!(e.panes.len(), 1);
        assert_eq!(e.docs.len(), 1);
    }

    #[test]
    fn split_with_path_opens_other_file() {
        let dir = tempfile::tempdir().unwrap();
        let a = dir.path().join("split-a.rs");
        let b = dir.path().join("split-b.rs");
        std::fs::write(&a, "fn a() {}\n").unwrap();
        std::fs::write(&b, "fn b() {}\n").unwrap();
        let mut e = Editor::new(Buffer::open(a.to_str().unwrap()).unwrap());
        e.feed_text(&format!(":vs {}<cr>", b.display()));
        e.wait_io().unwrap();
        assert_eq!(e.panes.len(), 2);
        assert_eq!(e.buf().path.as_deref(), Some(b.as_path()));
        e.feed(crate::editor::Key::CtrlW);
        e.feed(crate::editor::Key::Char('h'));
        assert_eq!(e.buf().path.as_deref(), Some(a.as_path()));
    }
}
