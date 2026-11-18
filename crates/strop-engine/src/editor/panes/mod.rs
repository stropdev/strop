//! Splits (0001 pillar 4: splits are core vim grammar). The panes form
//! vim's window tree (plans/0104, `layout.rs`): `:sp` and `:vs` divide the
//! current pane and nest as needed; closing one gives its cells to its
//! neighbours. `panes[k]` is window `k` in reading order. Documents are
//! shared between panes; the selections and scroll are per-pane (0014: the
//! pane OWNS them — no sync_to/from_pane copy-back, the active pane's state
//! is the editor's).

use strop_core::id::DisplayColumn;
use strop_core::selection::SelectionSet;

use super::Editor;

mod focus;
mod layout;
pub use layout::{Axis, Layout, Placement};

/// What `:q` closes (plans/0113 §4): the owner is computed first, then
/// acted on — not an ordered chain of tests.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Closing {
    CommandLineWindow,
    Buffer,
    Pane,
    TabPage,
}

/// One pane: the document it shows plus its own view state.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Pane {
    pub doc: strop_core::id::DocumentId,
    /// Input-mode views mirror the live terminal; Normal views pin its text.
    pub terminal_input: bool,
    pub sels: SelectionSet,
    pub view_top: usize,
    /// Horizontal display-cell origin (0031 R6): glyphs, overlays and
    /// every caret project through this; fixed left margins never do.
    pub hscroll: DisplayColumn,
    /// Desired cell retained while vertical motions cross short/wide rows.
    pub desired_column: Option<DisplayColumn>,
    /// Vim jump history belongs to this window, not the document or editor.
    #[serde(default)]
    pub jumps: super::jumps::JumpList,
    /// When this pane last had focus (the editor's focus epoch): `ctrl-w p`
    /// returns to the most recent other one. View-local, never persisted.
    #[serde(skip)]
    pub focused: u64,
}

impl Pane {
    /// A pane on `doc` from its start, keeping the jump list it came from.
    pub(crate) fn fresh(doc: strop_core::id::DocumentId, jumps: super::jumps::JumpList) -> Self {
        Pane {
            terminal_input: false,
            doc,
            sels: SelectionSet::default(),
            view_top: 0,
            hscroll: DisplayColumn::new(0),
            desired_column: None,
            jumps,
            focused: 0,
        }
    }

    /// Minimal horizontal scrolling: preserve the origin unless the
    /// caret leaves it. `width` is CONTENT width — every fixed left
    /// margin (sidebar, blame, number gutter) is excluded.
    pub fn reveal_column(&mut self, column: DisplayColumn, width: usize) {
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

impl Editor {
    /// The active pane — the editor's selections/scroll ARE its state.
    #[inline]
    pub fn view(&self) -> &Pane {
        &self.panes[self.active_pane]
    }

    /// Engine-internal pane mutation (0056 AR02): the frontend reads
    /// through [`Editor::view`]/[`Editor::panes`] and moves viewports
    /// through admitted actions and [`Editor::prepare_view`].
    #[inline]
    pub(crate) fn view_mut(&mut self) -> &mut Pane {
        &mut self.panes[self.active_pane]
    }

    /// Readonly pane list (0056 AR02): frontends lay out and stamp
    /// against this; splits/focus go through admitted actions.
    pub fn panes(&self) -> &[Pane] {
        &self.panes
    }

    #[inline]
    pub fn active_pane(&self) -> usize {
        self.active_pane
    }

    /// The window tree.
    #[inline]
    pub fn layout(&self) -> &Layout {
        &self.tabs.layout
    }

    pub(super) fn layout_matches_panes(&self) -> bool {
        self.tabs.layout.well_formed()
            && self.tabs.layout.leaves() == self.panes.len().max(1)
            && (self.panes.is_empty() || self.active_pane < self.panes.len())
    }

    /// The new pane opens after the active one along `axis` and takes focus.
    fn insert_pane(&mut self, axis: Axis, pane: Pane) {
        self.tabs.layout.split(self.active_pane, axis);
        self.panes.insert(self.active_pane + 1, pane);
        self.panes[self.active_pane].focused = self.focus_epoch;
        self.active_pane += 1;
        self.focus_epoch += 1;
        debug_assert!(self.layout_matches_panes());
    }

    /// Close the active pane (two or more); its neighbour takes its cells
    /// and the focus.
    fn remove_active_pane(&mut self) {
        let focus = self.tabs.layout.remove(self.active_pane);
        self.panes.remove(self.active_pane);
        self.active_pane = focus;
        self.focus_epoch += 1;
        debug_assert!(self.layout_matches_panes());
    }

    /// `:only`, `ctrl-w o`: the active pane becomes the only one.
    pub(crate) fn keep_only_active_pane(&mut self) {
        if self.panes.len() < 2 {
            return;
        }
        self.cancel_cmdline();
        let only = self.panes.swap_remove(self.active_pane);
        self.panes = vec![only];
        self.tabs.layout = Layout::Leaf;
        self.active_pane = 0;
        self.focus_epoch += 1;
        self.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
        self.discover_git();
        debug_assert!(self.layout_matches_panes());
    }
    /// Split the active pane. `vertical` = `:vs` (new pane to the right).
    /// Without a path the pane shows the same document (the split point).
    pub fn split(&mut self, vertical: bool, path: Option<&str>) {
        if let Some(path) = path {
            self.request_user_open(path, super::io::OpenIntent::Split { vertical });
        } else {
            self.split_document(vertical, self.current());
        }
    }
    pub fn split_document(&mut self, vertical: bool, doc: strop_core::id::DocumentId) {
        // a text prompt belongs to the pane/document it was opened on:
        // splitting away cancels it (R7) before any view state moves
        self.cancel_cmdline();
        self.cancel_open(strop_core::worker::CancelReason::Superseded);
        let view = self.view().clone();
        // a same-document split keeps the whole view (hscroll included);
        // a different document starts from a zero origin
        let pane = if doc == view.doc {
            view
        } else {
            Pane::fresh(doc, view.jumps)
        };
        self.insert_pane(if vertical { Axis::Row } else { Axis::Column }, pane);
        self.discover_git();
        self.lsp_attach_document(self.current());
    }

    /// What `:q` on the active pane closes.
    pub(crate) fn closing(&self) -> Closing {
        let current = self.current();
        if self
            .cmdline
            .window
            .is_some_and(|window| window.doc == current)
        {
            Closing::CommandLineWindow
        } else if self.is_commit_message(current) {
            // closing the commit message is what commits (or aborts)
            Closing::Buffer
        } else if self.panes.len() > 1 {
            Closing::Pane
        } else if self.tabs.count() > 1 {
            Closing::TabPage
        } else {
            Closing::Buffer
        }
    }

    /// `ctrl-w c` and `:close` refuse here (Vim's E444): the last window
    /// of the last tab page. A commit message in a split still closes as
    /// a buffer, as it did before tab pages.
    pub(crate) fn closes_last_window(&self) -> bool {
        self.panes.len() == 1 && self.closing() == Closing::Buffer
    }

    /// `:q`: the pane; a tab page's only pane closes the tab page; the
    /// last pane of the last tab page closes the buffer.
    pub(crate) fn close_pane_or_buffer(&mut self, force: bool) {
        match self.closing() {
            // `:q` in the command-line window closes it, buffer and all
            Closing::CommandLineWindow => super::cmdline::window::close(self),
            Closing::Pane => {
                self.cancel_cmdline();
                self.remove_active_pane();
                self.cancel_open(strop_core::worker::CancelReason::OwnerClosed);
                // the surviving pane's document may differ from the closed
                // pane's — git discovery follows the view, no copy-back
                self.discover_git();
            }
            // its buffers stay open, as with Neovim's 'hidden'
            Closing::TabPage => {
                super::tabs::close_current(self);
            }
            Closing::Buffer => {
                self.close_buffer(force);
            }
        }
    }

    /// `ctrl-w T`: the active pane leaves its tab page (closed there as
    /// `ctrl-w c` would) and becomes the only pane of a new one after it.
    fn pane_to_tab(&mut self) {
        if let Some(refused) = super::tabs::refusal(self) {
            self.message = refused.into();
            return;
        }
        if self.panes.len() == 1 {
            self.message = "already only one window".into();
            return;
        }
        let pane = self.view().clone();
        self.cancel_cmdline();
        self.remove_active_pane();
        super::tabs::open(self, pane);
    }

    /// `:qa` / `:qall` — quit the editor, closing every buffer through the
    /// real close path (leases, sessions, remote permits all settle). vim:
    /// refuses while any buffer is dirty; `:qa!` discards.
    pub(crate) fn quit_all(&mut self, force: bool) {
        if !force && self.filesystem.unconfirmed() > 0 {
            self.message =
                "some file changes are still unconfirmed; wait for the check, or :qa! to force"
                    .into();
            return;
        }
        if !force {
            let dirty = self.docs.iter().filter(|(_, d)| d.buf.is_dirty()).count();
            if dirty > 0 {
                self.message = format!("{dirty} unsaved buffer(s) — :qa! to discard");
                return;
            }
        }
        // Running terminals block `:qa` with one notice, like Neovim's
        // E947 (0090 §4); `:qa!` stops them and quits.
        if self.terminals.live() {
            if !force {
                let running = self.terminals.live_count();
                self.message = format!(
                    "{running} terminal{} running — :qa! stops them",
                    if running == 1 { "" } else { "s" }
                );
                return;
            }
            self.stop_all_terminals();
            self.should_quit = true;
            return;
        }
        let session_policy = self.session_policy;
        self.request_session_save();
        self.session_policy = crate::session::SessionPolicy::Disabled;
        while !self.docs.is_empty() {
            if !self.close_buffer(force) {
                self.session_policy = session_policy;
                return;
            }
        }
        self.session_policy = session_policy;
    }

    /// Table shims (0008 stage 2): Space w children dispatch by key.
    pub(crate) fn pane_move_pub(&mut self, key: char) {
        self.pane_move(key);
    }
    pub(crate) fn split_pub(&mut self, key: char) {
        self.split(key == 'v', None);
    }
    pub(crate) fn pane_close_pub(&mut self) {
        self.close_pane_or_buffer(false);
    }

    /// `ctrl-w {key}`: vim's window commands on the window tree. Directions
    /// stop at the edge, as in vim; panes are always equal, so `=` has
    /// nothing to do and says nothing.
    pub(crate) fn window_command(&mut self, key: char) {
        let panes = self.panes.len();
        match key {
            'h' | 'j' | 'k' | 'l' | 'w' => self.pane_move(key),
            'W' if panes > 1 => self.focus_pane((self.active_pane + panes - 1) % panes),
            'p' if panes > 1 => {
                let previous = (0..panes)
                    .filter(|index| *index != self.active_pane)
                    .max_by_key(|index| self.panes[*index].focused);
                if let Some(previous) = previous {
                    self.focus_pane(previous);
                }
            }
            'W' | 'p' => self.message = "no other pane".into(),
            's' => self.split(false, None),
            'v' => self.split(true, None),
            'n' => {
                let scratch = super::document::Document::scratch(strop_core::Buffer::from_text(""));
                match self.docs.try_insert(scratch) {
                    Ok(document) => self.split_document(false, document),
                    Err(error) => self.message = error.to_string(),
                }
            }
            // vim: `c` never closes the last window (E444), but a tab
            // page's only one closes the tab page; `q` is `:q`
            'c' if self.closes_last_window() => {
                self.message = "cannot close the last window".into()
            }
            'c' | 'q' => self.close_pane_or_buffer(false),
            'o' if panes == 1 => self.message = "already only one window".into(),
            'o' => self.keep_only_active_pane(),
            'T' => self.pane_to_tab(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests;
