//! Tab pages (plans/0113): Vim's second layout over the same buffers. A
//! tab page is a window tree, its panes and the pane with the focus. The
//! current one is `Editor.panes`, `Editor.active_pane` and `Tabs.layout`,
//! so every reader of the visible panes is untouched; the others are
//! parked here.
//!
//! A parked pane is treated exactly as an inactive pane. Every site that
//! follows a *document* (an edit maps its selections, a close rebinds it,
//! terminal output moves its cursor) walks `self.panes` AND
//! `self.tabs.panes_mut()`: a site added later that walks only
//! `self.panes` by document leaves a parked pane on a dead document or a
//! stale offset. `Tabs::panes_mut` is the only way in.
//!
//! The functions below are the only code that changes which tab page is
//! current. They are functions over the editor, not methods of it (0087
//! A4: `Editor` does not grow), as the `:` commands are.

use std::mem;

use strop_core::id::DocumentId;

use super::prepare::{CellRect, ViewGeometry};
use super::{Editor, Layout, Pane};

/// A tab page that is not on screen: its window tree, its panes in
/// reading order, and the pane that has the focus when it comes back.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct TabPage {
    pub layout: Layout,
    pub panes: Vec<Pane>,
    pub active_pane: usize,
    /// The focus epoch when it was left; `g<Tab>` looks for one stamp.
    #[serde(skip)]
    pub left: u64,
}

impl TabPage {
    fn well_formed(&self) -> bool {
        self.layout.well_formed()
            && self.layout.leaves() == self.panes.len()
            && self.active_pane < self.panes.len()
    }
}

/// The window trees: the current tab page's, and the tab pages other than
/// the current one. In tab order the current one stands at `current`,
/// between `parked[current - 1]` and `parked[current]`.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Tabs {
    /// The current tab page's window tree: leaf `k` is `Editor.panes[k]`
    /// (plans/0104).
    pub layout: Layout,
    parked: Vec<TabPage>,
    current: usize,
    /// The stamp of the tab page left most recently; `None` once it closed.
    #[serde(skip)]
    previous: Option<u64>,
}

impl Tabs {
    /// Tab pages, the current one included.
    pub(crate) fn count(&self) -> usize {
        self.parked.len() + 1
    }

    /// Every parked pane, for the sites that follow a document.
    pub(crate) fn panes(&self) -> impl Iterator<Item = &Pane> {
        self.parked.iter().flat_map(|page| page.panes.iter())
    }

    /// Every parked pane, mutably: the only way to a parked pane.
    pub(crate) fn panes_mut(&mut self) -> impl Iterator<Item = &mut Pane> {
        self.parked
            .iter_mut()
            .flat_map(|page| page.panes.iter_mut())
    }

    /// The parked page at tab-order `index` (never the current one).
    fn parked_at(&self, index: usize) -> Option<&TabPage> {
        match index {
            index if index < self.current => self.parked.get(index),
            index if index > self.current => self.parked.get(index - 1),
            _ => None,
        }
    }

    /// Each tab page's window tree, in tab order.
    pub(crate) fn layouts(&self) -> impl Iterator<Item = &Layout> {
        let (before, after) = self.parked.split_at(self.current);
        before
            .iter()
            .map(|page| &page.layout)
            .chain(std::iter::once(&self.layout))
            .chain(after.iter().map(|page| &page.layout))
    }

    /// Invariants 1 and 2 of plans/0113 §4.
    pub(crate) fn well_formed(&self) -> bool {
        self.current <= self.parked.len() && self.parked.iter().all(TabPage::well_formed)
    }
}

/// What the tab line says about one tab page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TabSummary {
    /// What its current pane shows.
    pub document: DocumentId,
    /// How many panes it holds.
    pub panes: usize,
    /// A pane in it shows a buffer with unsaved changes.
    pub unsaved: bool,
}

/// Tab pages, the current one included.
pub fn count(editor: &Editor) -> usize {
    editor.tabs.count()
}

/// The current tab page, 0-based in tab order.
pub fn current(editor: &Editor) -> usize {
    editor.tabs.current
}

/// The tab line's facts about tab page `index` (0-based); `None` past the
/// last.
pub fn summary(editor: &Editor, index: usize) -> Option<TabSummary> {
    let (panes, active) = if index == editor.tabs.current {
        (editor.panes.as_slice(), editor.active_pane)
    } else {
        let page = editor.tabs.parked_at(index)?;
        (page.panes.as_slice(), page.active_pane)
    };
    let unsaved = panes.iter().any(|pane| {
        editor
            .docs
            .get(pane.doc)
            .is_some_and(|document| document.buf.is_dirty())
    });
    Some(TabSummary {
        document: panes.get(active)?.doc,
        panes: panes.len(),
        unsaved,
    })
}

/// Where the panes go: the view, less the tab line's row when there are
/// two or more tab pages. The callers of `Layout::place` start here; the
/// layout and the kernel never see the tab line.
pub fn pane_area(editor: &Editor, geometry: ViewGeometry) -> CellRect {
    let top = u16::from(editor.tabs.count() > 1).min(geometry.rows);
    CellRect::new(0, top, geometry.columns, geometry.rows - top)
}

/// Invariants 1–3 of plans/0113 §4.
pub(crate) fn well_formed(editor: &Editor) -> bool {
    editor.tabs.well_formed()
        && editor
            .tabs
            .panes()
            .all(|pane| editor.docs.get(pane.doc).is_some())
}

/// While the command-line window is open no tab page changes (Vim's E11).
pub(crate) fn refusal(editor: &Editor) -> Option<&'static str> {
    editor
        .cmdline
        .window
        .is_some()
        .then_some("close the command-line window first")
}

/// The current tab page leaves the screen.
fn park(editor: &mut Editor) -> TabPage {
    TabPage {
        layout: mem::take(&mut editor.tabs.layout),
        panes: mem::take(&mut editor.panes),
        active_pane: mem::take(&mut editor.active_pane),
        left: editor.focus_epoch,
    }
}

/// A parked page becomes the current one.
fn show(editor: &mut Editor, page: TabPage) {
    editor.tabs.layout = page.layout;
    editor.panes = page.panes;
    editor.active_pane = page.active_pane;
}

fn leave(editor: &mut Editor) {
    editor.cancel_cmdline();
    editor.cancel_open(strop_core::worker::CancelReason::Superseded);
}

fn arrive(editor: &mut Editor) {
    editor.focus_epoch += 1;
    editor.discover_git();
    editor.clamp_cursor();
    editor.lsp_attach_document(editor.current());
    debug_assert!(well_formed(editor));
}

/// Tab page `index` (0-based) becomes current.
pub(crate) fn go(editor: &mut Editor, index: usize) {
    if index == editor.tabs.current || index >= editor.tabs.count() {
        return;
    }
    leave(editor);
    let page = park(editor);
    editor.tabs.previous = Some(page.left);
    // insert-then-remove keeps tab order with no index arithmetic
    editor.tabs.parked.insert(editor.tabs.current, page);
    let page = editor.tabs.parked.remove(index);
    show(editor, page);
    editor.tabs.current = index;
    arrive(editor);
}

/// A new tab page right after the current one, `pane` its only pane.
pub(crate) fn open(editor: &mut Editor, pane: Pane) {
    leave(editor);
    let page = park(editor);
    editor.tabs.previous = Some(page.left);
    editor.tabs.parked.insert(editor.tabs.current, page);
    editor.tabs.current += 1;
    show(
        editor,
        TabPage {
            layout: Layout::Leaf,
            panes: vec![pane],
            active_pane: 0,
            left: 0,
        },
    );
    arrive(editor);
}

/// A new tab page showing `document`, as a split to it would.
pub(crate) fn open_document(editor: &mut Editor, document: DocumentId) {
    let view = editor.view();
    let pane = if document == view.doc {
        view.clone()
    } else {
        Pane::fresh(document, view.jumps.clone())
    };
    open(editor, pane);
}

/// Close the current tab page; the one that takes its number (or the new
/// last) becomes current. `false` with one tab page.
pub(crate) fn close_current(editor: &mut Editor) -> bool {
    let Some(last) = editor.tabs.parked.len().checked_sub(1) else {
        return false;
    };
    leave(editor);
    let index = editor.tabs.current.min(last);
    let page = editor.tabs.parked.remove(index);
    show(editor, page);
    editor.tabs.current = index;
    editor.tabs.previous = None;
    arrive(editor);
    true
}

/// Close tab page `index` (0-based); `false` when there is no other. Closing
/// another page is bookkeeping: the current page, its pending work and the
/// focus epoch they are keyed on stay as they are.
pub(crate) fn close(editor: &mut Editor, index: usize) -> bool {
    let current = editor.tabs.current;
    if index == current {
        return close_current(editor);
    }
    let at = if index < current { index } else { index - 1 };
    if at >= editor.tabs.parked.len() {
        return false;
    }
    let page = editor.tabs.parked.remove(at);
    if editor.tabs.previous == Some(page.left) {
        editor.tabs.previous = None;
    }
    if index < current {
        editor.tabs.current -= 1;
    }
    debug_assert!(well_formed(editor));
    true
}

/// `:tabonly`: every other tab page goes; the current one, and its pending
/// work, stays.
pub(crate) fn only(editor: &mut Editor) {
    if editor.tabs.parked.is_empty() {
        return;
    }
    editor.tabs.parked.clear();
    editor.tabs.current = 0;
    editor.tabs.previous = None;
    debug_assert!(well_formed(editor));
}

/// `:tabmove`: the current tab page moves to tab-order `index`; the parked
/// order is already the order of the others. The page on screen does not
/// change, so neither does its pending work.
pub(crate) fn move_to(editor: &mut Editor, index: usize) {
    debug_assert!(index < editor.tabs.count());
    if index == editor.tabs.current {
        return;
    }
    editor.tabs.current = index;
    debug_assert!(well_formed(editor));
}

/// `g<Tab>`: the tab page that was current before this one; nothing when
/// there is none or it has closed.
pub(crate) fn previous(editor: &mut Editor) {
    if let Some(refused) = refusal(editor) {
        editor.message = refused.into();
        return;
    }
    let Some(stamp) = editor.tabs.previous else {
        return;
    };
    let Some(at) = editor
        .tabs
        .parked
        .iter()
        .position(|page| page.left == stamp)
    else {
        return;
    };
    let index = if at < editor.tabs.current { at } else { at + 1 };
    go(editor, index);
}

/// `gt` / `gT`: `count` is a target, not a repeat. `gt` the next, `{N}gt`
/// tab page N, `gT` the previous, `{N}gT` N back; all wrap.
pub(crate) fn step(editor: &mut Editor, back: bool, count: Option<usize>) {
    if let Some(refused) = refusal(editor) {
        editor.message = refused.into();
        return;
    }
    let tabs = editor.tabs.count();
    let current = editor.tabs.current;
    let target = match (back, count) {
        (false, None | Some(0)) => (current + 1) % tabs,
        (false, Some(number)) if number <= tabs => number - 1,
        (false, Some(number)) => {
            editor.message = format!("no tab {number}");
            return;
        }
        (true, count) => {
            let steps = count.unwrap_or(1) % tabs;
            (current + tabs - steps) % tabs
        }
    };
    go(editor, target);
}

#[cfg(test)]
mod tests;
