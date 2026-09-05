//! Splits (0001 pillar 4: splits are core vim grammar). v1: a flat row
//! (`:vs`, side by side) or column (`:sp`, stacked) — mixed nesting is
//! the tree-layout follow-up. Documents are shared between panes; the
//! selections and scroll are per-pane (0014: the pane OWNS them — no
//! sync_to/from_pane copy-back, the active pane's state is the editor's).

use strop_core::selection::SelectionSet;

use super::Editor;

/// One pane: the document it shows plus its own view state.
#[derive(Debug, Clone)]
pub struct Pane {
    pub doc: strop_core::id::DocumentId,
    pub sels: SelectionSet,
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
        let view = self.view().clone();
        // with a path: the NEW pane shows it — the old pane keeps its doc
        let doc = if let Some(p) = path {
            match self.open_document(p) {
                Ok(id) => id,
                Err(e) => {
                    self.message = format!("open {p}: {e}");
                    return;
                }
            }
        } else {
            view.doc
        };
        self.panes.push(Pane {
            doc,
            sels: view.sels,
            view_top: view.view_top,
        });
        self.layout = if vertical {
            LayoutDir::Row
        } else {
            LayoutDir::Column
        };
        self.active_pane = self.panes.len() - 1;
        self.discover_git();
        self.lsp_maybe_attach();
    }

    /// `:q` closes the pane; the last pane's close is document close.
    pub(crate) fn close_pane_or_buffer(&mut self, force: bool) {
        if self.panes.len() > 1 {
            self.panes.remove(self.active_pane);
            self.active_pane = self.active_pane.min(self.panes.len() - 1);
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
        self.active_pane = next; // state is already per-pane: no sync
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
        assert_eq!(e.panes.len(), 2);
        assert_eq!(e.buf().path.as_deref(), b.to_str());
        e.feed(crate::editor::Key::CtrlW);
        e.feed(crate::editor::Key::Char('h'));
        assert_eq!(e.buf().path.as_deref(), a.to_str());
    }
}
