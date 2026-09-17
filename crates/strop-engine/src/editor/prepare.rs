//! AR01/AR03: admitted view preparation and the prepared-view read model.
//!
//! The supported flow (0056 §4):
//!
//! ```text
//! input / source change / service result / viewport change
//!       -> admitted engine update ([`Editor::prepare_view`])
//!       -> prepared view + owned work requests
//!       -> readonly presentation query (the frontend's paint)
//! ```
//!
//! Preparation is idempotent: a stamp over every view-relevant input
//! (geometry, panes, revisions, picker state, hunk identity, git
//! discovery, config geometry inputs) skips viewport adjustment and work
//! admission when nothing meaningful changed. The externally keyed
//! admissions (git hunks, picker preview reads, visible-window analysis)
//! are each internally idempotent, so an unchanged repaint admits zero
//! worker tickets — [`AdmissionProbe`] is the instrument.
//!
//! Geometry is an explicit input owned by the view epoch: terminal cells
//! here, never pixels. Paint reads [`Editor::prepared_view`] and live
//! readonly engine state; it never admits work or mutates viewports.

use std::hash::{Hash, Hasher};

use strop_core::id::{BufferRevision, DisplayColumn, DocumentId};

use super::panes::LayoutDir;
use super::{Editor, Mode, Surface};

pub mod geometry;
pub use geometry::{
    gutter_width, pane_cells, picker_card, picker_field_split, picker_inner, picker_input_height,
    picker_results, picker_rows_per_result, picker_split, preview_source_lines, preview_window,
    text_budget, CellRect, ViewGeometry, BLAME_GUTTER_WIDTH, STANDARD_GUTTER,
};

/// The identity of one accepted preparation: the generation bumps on
/// every meaningful state/geometry change, so a frontend holding an old
/// epoch knows its viewport facts are stale.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewEpoch {
    pub generation: u64,
    pub geometry: ViewGeometry,
}

/// Declared bounds of a prepared pane window (AR03): a consumer can tell
/// a complete window from a partial remote window, decorations still in
/// flight, a revision the document has moved past, or a vanished source.
/// There is no empty-success fallback — every state is explicit.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowBounds {
    /// Full source window available; decorations (if any) cached.
    Complete,
    /// Remote partial window: line numbers are window-relative (0036).
    Partial,
    /// Visible-window analysis admitted but not yet cached.
    Loading,
    /// The document's revision moved past this prepared window.
    Stale,
    /// The document vanished since preparation (defensive; panes are
    /// rebound before documents close, so this should never paint).
    Error,
}

/// One pane's prepared presentation window, keyed by document identity +
/// revision and the owning view generation.
#[derive(Debug, Clone, Copy)]
pub struct PreparedPane {
    pub doc: DocumentId,
    /// The document revision this window was prepared against.
    pub revision: BufferRevision,
    /// The pane's cell rectangle (identity title row already subtracted).
    pub rect: CellRect,
    /// The text budget: `rect` minus the reserved scrollbar track column
    /// (0064 §1) — the SAME budget preparation clamps against and paint
    /// draws into, so viewport geometry and cells can never diverge.
    pub budget: CellRect,
    pub cursor: usize,
    pub view_top: usize,
    pub hscroll: DisplayColumn,
    pub terminal_input: bool,
    /// Preview/search/selection/flash overlays belong to the driven pane.
    pub overlays: bool,
    /// Declared bounds, refreshed on every preparation pass.
    pub bounds: WindowBounds,
}

impl PreparedPane {
    /// True when the live document revision moved past this window —
    /// a stale-revision window must not drive coordinate conversions or
    /// viewport decisions.
    pub fn is_stale(&self, current: BufferRevision) -> bool {
        // The verified re-key rule (0057 VF18, strop_core::viewguard).
        strop_core::viewguard::revision_is_stale(self.revision.get(), current.get())
    }
}

/// The published result of one accepted preparation (AR03). Borrowed by
/// the TUI per frame; process clients (0056 AR09/AR10) serialize bounded
/// windows keyed by generation + pane revisions, never a whole-Editor
/// clone.
#[derive(Debug, Clone)]
pub struct PreparedView {
    pub generation: u64,
    pub geometry: ViewGeometry,
    pub active_pane: usize,
    pub panes: Vec<PreparedPane>,
}

impl PreparedView {
    pub(crate) fn empty() -> Self {
        Self {
            generation: 0,
            geometry: ViewGeometry {
                columns: 0,
                rows: 0,
            },
            active_pane: 0,
            panes: Vec::new(),
        }
    }

    /// The prepared window for one pane index.
    pub fn pane(&self, index: usize) -> Option<&PreparedPane> {
        self.panes.get(index)
    }
}

/// Worker-ticket admissions since startup (AR01 evidence): a repaint
/// without an engine/view change moves none of these counters.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub struct AdmissionProbe {
    /// Hunk diff jobs admitted by [`Editor::refresh_hunks`].
    pub hunks: usize,
    /// Viewport/preview analysis tickets (syntax, guides, search).
    pub analysis: usize,
    /// Picker preview bounded reads.
    pub previews: usize,
    /// Matching-delimiter scans.
    pub pairs: usize,
}
impl Editor {
    /// The current view epoch: the last accepted preparation's generation
    /// and geometry.
    pub fn view_epoch(&self) -> ViewEpoch {
        ViewEpoch {
            generation: self.view_generation,
            geometry: self.prepared.geometry,
        }
    }

    /// The published prepared view (AR03), borrowed per frame.
    pub fn prepared_view(&self) -> &PreparedView {
        &self.prepared
    }

    /// Worker-ticket admissions since startup (AR01 evidence).
    pub fn admission_probe(&self) -> AdmissionProbe {
        self.admissions
    }

    /// AR01: the admitted engine update that owns every preparation-side
    /// effect — hunk refresh, visible-analysis/preview admission and
    /// viewport/caret adjustment. The frontend calls this when input,
    /// source, service results or viewport geometry change, before the
    /// readonly paint. Idempotent for unchanged inputs: an equal stamp
    /// admits no tickets, launches no process and mutates no viewport.
    pub fn prepare_view(&mut self, geometry: ViewGeometry) -> ViewEpoch {
        let stamp = self.view_stamp(geometry);
        if Some(stamp) != self.frame_stamp {
            // Hunk refresh is externally keyed (HunkKey covers document,
            // revision, file, repo and git view) and its remaining inputs
            // — git discovery, source identity, remote completeness — are
            // stamped above, so a stamp change is exactly when it can
            // admit new work. Render-safe: pure checks and registration;
            // the diff itself runs on a worker.
            self.refresh_hunks();
            let rects = pane_cells(
                self.panes.len(),
                self.layout == LayoutDir::Row,
                CellRect::from(geometry),
            );
            let settled = self.adjust_active_viewport(&rects);
            // Visible-window analysis and pair admission for every pane,
            // after the viewport settled. Paint reads the same windows
            // back as pure cache hits.
            let panes: Vec<PreparedPane> = self
                .panes
                .iter()
                .enumerate()
                .map(|(index, pane)| PreparedPane {
                    doc: pane.doc,
                    revision: self
                        .docs
                        .get(pane.doc)
                        .map(|document| document.buf.revision())
                        .unwrap_or(BufferRevision::new(0)),
                    rect: rects[index],
                    budget: text_budget(rects[index]),
                    cursor: pane.sels.primary().head,
                    view_top: pane.view_top,
                    hscroll: pane.hscroll,
                    terminal_input: pane.terminal_input,
                    overlays: index == self.active_pane,
                    bounds: WindowBounds::Complete, // refreshed below
                })
                .collect();
            for pane in &panes {
                self.admit_visible_work(pane);
            }
            self.prepared = PreparedView {
                generation: self.view_generation,
                geometry,
                active_pane: self.active_pane,
                panes,
            };
            if settled {
                self.frame_stamp = Some(stamp);
                self.view_generation = self.view_generation.saturating_add(1);
                self.prepared.generation = self.view_generation;
            }
            // Not settled: the caret's line layout is still cold (the
            // bounded probe declines long cold lines). Preparation stays
            // pending — admission above is keyed, so the next frame
            // re-runs it once analysis has installed the line's layout,
            // converging exactly like the pre-AR01 every-frame clamp.
        }
        // Declared bounds reflect service results that land without a
        // stamp change (analysis completion, hunk publication) — refresh
        // them on every preparation pass.
        self.refresh_pane_bounds();
        // The picker's reveal clamp and preview admission are externally
        // keyed (selection/rows, preview load keys) and internally
        // idempotent: they run on every preparation, as before.
        self.reveal_picker(geometry);
        self.view_epoch()
    }

    /// Active-pane viewport and terminal geometry own their exact pane
    /// rect. Pure state adjustment plus the keyed terminal resize.
    /// Returns false when the caret's horizontal probe declined (cold
    /// layout cache on a long line): the viewport is not yet provably
    /// revealing the caret, so preparation must stay pending.
    fn adjust_active_viewport(&mut self, rects: &[CellRect]) -> bool {
        let mut settled = true;
        for (index, rect) in rects.iter().enumerate() {
            if index != self.active_pane {
                continue;
            }
            let pane_doc = self.panes[index].doc;
            let terminal_input = self.terminal_view_input(&self.panes[index]);
            let h = usize::from(rect.height);
            if h == 0 {
                self.view_rows = 0;
            } else if terminal_input {
                self.view_rows = h;
            } else {
                self.scroll_to_cursor(h);
            }
            if !terminal_input && h != 0 {
                // 0064 §1: clamp against the same reserved budget paint
                // draws into — the track column is never content.
                let width = usize::from(text_budget(*rect).width)
                    .saturating_sub(self.left_inset(self.current()));
                let head = self.head();
                let column_probe = self
                    .buf()
                    .try_cell_col_with_tab(head, self.indentation_at(pane_doc, head).width);
                if let Some(column) = column_probe {
                    self.view_mut().reveal_column(column, width);
                } else {
                    settled = false;
                }
            }
            if terminal_input && rect.width != 0 && rect.height != 0 {
                let budget = text_budget(*rect);
                self.prepare_terminal_geometry(pane_doc, budget.width, budget.height);
            }
        }
        settled
    }

    /// Frame-preparation admission for one pane's visible window:
    /// analysis for the pane document plus its collection's projected
    /// sources, and the active pane's pair-match job. Paint serves the
    /// identical windows from cache.
    fn admit_visible_work(&mut self, pane: &PreparedPane) {
        let rows = usize::from(pane.budget.height);
        if rows == 0 || self.docs.get(pane.doc).is_none() {
            return;
        }
        let buf = &self.doc(pane.doc).buf;
        let last_line = pane.view_top.saturating_add(rows).min(buf.len_lines());
        let first = buf.line_start(pane.view_top);
        let last = buf.line_end(last_line.saturating_sub(1));
        self.document_analysis(
            pane.doc,
            first,
            last,
            pane.hscroll.get(),
            usize::from(pane.budget.width),
        );
        let collection_rows: Vec<Option<super::CollectionRowInfo>> = (0..rows)
            .map(|row| self.collection_row_info(pane.doc, pane.view_top.saturating_add(row)))
            .collect();
        let mut windows: Vec<(DocumentId, usize, usize)> = Vec::new();
        for info in collection_rows.iter().flatten() {
            if let Some((source, start, end)) = info.source {
                if let Some((_, first, last)) = windows.iter_mut().find(|(doc, ..)| *doc == source)
                {
                    *first = (*first).min(start);
                    *last = (*last).max(end);
                } else {
                    windows.push((source, start, end));
                }
            }
        }
        for (source, first, last) in windows {
            self.document_analysis(
                source,
                first,
                last,
                pane.hscroll.get(),
                usize::from(pane.budget.width),
            );
        }
        if pane.overlays {
            let _ = self.pair_highlight(pane.doc, pane.cursor, matches!(self.mode, Mode::Insert));
        }
    }

    /// Preparation-time scroll clamping for the open picker: the same
    /// results area paint uses, so the viewport matches what is about to
    /// be drawn.
    fn reveal_picker(&mut self, geometry: ViewGeometry) {
        let (kind, replace_visible) = match self.picker.as_ref() {
            Some(glue) if !matches!(glue.picker.kind, strop_picker::Kind::RemoteAddress) => {
                (glue.picker.kind, glue.picker.replacement_visible)
            }
            _ => return,
        };
        let Some(results) = picker_results(CellRect::from(geometry), kind, replace_visible) else {
            return;
        };
        let visible_rows =
            (results.height as usize / picker_rows_per_result(kind, replace_visible)).max(1);
        if let Some(glue) = self.picker.as_mut() {
            glue.picker.reveal_selected(visible_rows);
        }
        self.admit_preview(geometry, kind, replace_visible);
    }

    /// AR01 admission for the picker preview: the mutable resolver
    /// requests the bounded read, then the visible window's syntax
    /// analysis is admitted with exactly the bounds paint will query.
    /// Paint reads only the cached twins, so without this the preview
    /// stays on `loading…` forever.
    fn admit_preview(
        &mut self,
        geometry: ViewGeometry,
        kind: strop_picker::Kind,
        replace_visible: bool,
    ) {
        let Some((_, focus_line, source)) = self.picker_preview() else {
            return;
        };
        let Some(preview_area) =
            picker_split(CellRect::from(geometry), kind, replace_visible).and_then(|(_, p)| p)
        else {
            return;
        };
        let visible = preview_area.height.saturating_sub(1) as usize;
        let width = usize::from(preview_area.width.saturating_sub(1));
        match source {
            super::PreviewSource::Buffer(document) => {
                let rope = self.doc(document).buf.snapshot();
                let window = preview_window(&rope, focus_line, visible);
                self.document_analysis(
                    document,
                    rope.line_to_byte(window.start),
                    rope.line_to_byte(window.end),
                    0,
                    width,
                );
            }
            super::PreviewSource::Cached(path) => {
                let Some(entry) = self.previews.get(&path) else {
                    return;
                };
                let rope = entry.rope.clone();
                let window = preview_window(&rope, focus_line, visible);
                self.preview_analysis(
                    &path,
                    rope.line_to_byte(window.start),
                    rope.line_to_byte(window.end),
                    width,
                );
            }
            _ => {}
        }
    }

    /// Declared bounds for one prepared pane, probed live: service
    /// results can land without a stamp change, so bounds refresh on
    /// every preparation pass.
    fn pane_bounds(&self, pane: &PreparedPane) -> WindowBounds {
        let Some(document) = self.docs.get(pane.doc) else {
            return WindowBounds::Error;
        };
        if pane.is_stale(document.buf.revision()) {
            return WindowBounds::Stale;
        }
        if document
            .remote_metadata()
            .is_some_and(|source| !source.window.is_complete())
        {
            return WindowBounds::Partial;
        }
        if self.window_analysis_pending(pane, document) {
            return WindowBounds::Loading;
        }
        WindowBounds::Complete
    }

    fn refresh_pane_bounds(&mut self) {
        // In-place, no per-frame allocation: PreparedPane is Copy, so the
        // probe borrows a value copy while the slot is written back.
        for index in 0..self.prepared.panes.len() {
            let pane = self.prepared.panes[index];
            let bounds = self.pane_bounds(&pane);
            self.prepared.panes[index].bounds = bounds;
        }
    }

    /// True when the pane's visible window needs analysis (guides, syntax
    /// or search) that is neither cached nor served stale — the window's
    /// decorations are still in flight.
    fn window_analysis_pending(&self, pane: &PreparedPane, document: &super::Document) -> bool {
        let guides = self.config.indent_guides
            && matches!(
                document.source,
                super::DocumentSource::File
                    | super::DocumentSource::Scratch
                    | super::DocumentSource::Remote(_)
            );
        let search = if pane.doc == self.current() {
            self.current_search_query().ok().flatten()
        } else {
            None
        };
        if !guides && document.syntax_path().is_none() && search.is_none() {
            return false;
        }
        let rows = usize::from(pane.budget.height);
        if rows == 0 {
            return false;
        }
        let last_line = pane
            .view_top
            .saturating_add(rows)
            .min(document.buf.len_lines());
        let first = document.buf.line_start(pane.view_top);
        let last = document.buf.line_end(last_line.saturating_sub(1));
        self.document_analysis_cached(
            pane.doc,
            first,
            last,
            pane.hscroll.get(),
            usize::from(pane.budget.width),
        )
        .is_none()
    }

    /// Everything stamp-guarded preparation depends on: geometry, focus,
    /// per-pane view state, document revisions and source identity,
    /// picker selection, the published hunk generation, config geometry
    /// inputs, and every input `refresh_hunks` keys on (git discovery,
    /// repo identity, git view, remote window completeness). Equal
    /// stamps mean preparation is already done for exactly this
    /// presentation state.
    fn view_stamp(&self, geometry: ViewGeometry) -> u64 {
        let mut stamp = std::collections::hash_map::DefaultHasher::new();
        (geometry.columns, geometry.rows).hash(&mut stamp);
        self.panes.len().hash(&mut stamp);
        self.active_pane.hash(&mut stamp);
        self.focus_epoch.hash(&mut stamp);
        usize::from(matches!(self.mode, Mode::Insert)).hash(&mut stamp);
        self.config.tab_size.hash(&mut stamp);
        self.config.indent_guides.hash(&mut stamp);
        for pane in &self.panes {
            (pane.doc.index(), pane.doc.generation()).hash(&mut stamp);
            let document = self.docs.get(pane.doc);
            document
                .map(|document| document.buf.revision().get())
                .unwrap_or(u64::MAX)
                .hash(&mut stamp);
            // Hunk-key inputs: a rebind/rename with an unchanged revision
            // still re-keys the gutter diff.
            document
                .map(|document| std::mem::discriminant(&document.source))
                .hash(&mut stamp);
            document
                .and_then(|document| document.buf.path.as_deref())
                .hash(&mut stamp);
            pane.sels.primary().head.hash(&mut stamp);
            pane.view_top.hash(&mut stamp);
            pane.hscroll.get().hash(&mut stamp);
            usize::from(pane.terminal_input).hash(&mut stamp);
            self.terminal_frame(pane.doc, true)
                .map(|frame| frame.revision)
                .unwrap_or(0)
                .hash(&mut stamp);
        }
        self.picker
            .as_ref()
            .map(|glue| {
                (
                    glue.picker.selected,
                    glue.picker.items.len(),
                    glue.picker.rows.len(),
                )
            })
            .hash(&mut stamp);
        self.hunks.identity().hash(&mut stamp);
        // Git discovery and repository identity: the first hunk load is
        // admitted when discovery lands, not one edit later.
        self.git.is_some().hash(&mut stamp);
        if let Some(context) = &self.git {
            context.head_sha.hash(&mut stamp);
            context.repo.workdir().hash(&mut stamp);
        }
        self.git_view.get().hash(&mut stamp);
        self.remote_window_complete().hash(&mut stamp);
        stamp.finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::Buffer;

    fn geometry() -> ViewGeometry {
        ViewGeometry {
            columns: 60,
            rows: 10,
        }
    }

    /// AR01: unchanged preparation admits zero tickets and moves nothing.
    #[test]
    fn unchanged_preparation_admits_zero_work() {
        let mut e = Editor::new(Buffer::from_text("fn main() {\n    let x = (1);\n}\n"));
        let first = e.prepare_view(geometry());
        assert_eq!(first.generation, 1, "the first preparation is accepted");
        e.wait_analysis();
        // Analysis completion lands without a stamped view change: the
        // next preparation is a pure no-op, epoch unchanged.
        assert_eq!(e.prepare_view(geometry()), first);
        e.wait_analysis();
        let probe = e.admission_probe();
        let (top, head) = (e.view().view_top, e.head());
        for _ in 0..8 {
            assert_eq!(e.prepare_view(geometry()), first);
        }
        assert_eq!(
            e.admission_probe(),
            probe,
            "zero tickets across unchanged preparations"
        );
        assert_eq!((e.view().view_top, e.head()), (top, head));
    }

    /// AR01/AR03: a geometry change is a new epoch; a stale prepared
    /// window reports Stale instead of acting.
    #[test]
    fn geometry_and_revision_changes_rekey_the_prepared_view() {
        let mut e = Editor::new(Buffer::from_text("one\ntwo\nthree\n"));
        let epoch = e.prepare_view(geometry());
        let resized = e.prepare_view(ViewGeometry {
            columns: 30,
            rows: 6,
        });
        assert!(resized.generation > epoch.generation);
        assert_eq!(resized.geometry.rows, 6);
        let doc = e.current();
        let prepared_revision = e.prepared_view().panes[0].revision;
        e.feed_text("x");
        let live = e.document(doc).map(|d| d.buf.revision()).unwrap();
        assert!(
            e.prepared_view().panes[0].is_stale(live),
            "an edit without preparation leaves the prepared window stale"
        );
        assert_eq!(e.prepared_view().panes[0].revision, prepared_revision);
        let reprepared = e.prepare_view(ViewGeometry {
            columns: 30,
            rows: 6,
        });
        assert!(reprepared.generation > resized.generation);
        assert!(!e.prepared_view().panes[0].is_stale(live));
    }

    /// AR03: bounds distinguish complete/loading — a syntax-backed buffer
    /// reports Loading until the admitted analysis lands, then Complete.
    #[test]
    fn pane_bounds_distinguish_loading_from_complete() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("probe.rs");
        std::fs::write(&path, "fn main() {}\n").unwrap();
        let mut e = Editor::new(Buffer::from_text(""));
        e.open_fixture(&path).unwrap();
        e.prepare_view(geometry());
        let initial = e.prepared_view().panes[0].bounds;
        e.wait_analysis();
        e.prepare_view(geometry());
        let settled = e.prepared_view().panes[0].bounds;
        assert_eq!(settled, WindowBounds::Complete);
        assert!(
            initial == WindowBounds::Loading || initial == WindowBounds::Complete,
            "bounds are declared, never empty: {initial:?}"
        );
        // If analysis was still in flight initially, the first paint is
        // honestly Loading; after the wait it must be Complete.
    }

    /// The moved picker geometry: exact arithmetic for the field split,
    /// stacking and percentage columns at their boundaries.
    #[test]
    fn picker_geometry_boundaries() {
        let area = CellRect::new(0, 0, 100, 30);
        let card = picker_card(area, strop_picker::Kind::Files);
        assert_eq!(card, CellRect::new(1, 0, 98, 29));
        let inner = picker_inner(card);
        assert_eq!(inner, CellRect::new(3, 1, 94, 27));
        let (fields, content) = picker_field_split(inner, 2);
        assert_eq!(fields.height, 2);
        assert_eq!(content, CellRect::new(3, 3, 94, 25));
        let (list, preview) = picker_split(area, strop_picker::Kind::Files, false).unwrap();
        // 55/45 of 94: 51.7 rounds half-up to 52, preview takes the rest.
        assert_eq!(list.width, 52);
        assert_eq!(preview.unwrap().width, 42);
        // grep 60/40
        let (list, _) = picker_split(area, strop_picker::Kind::Search, false).unwrap();
        assert_eq!(list.width, 56);
        // narrow terminal stacks: list keeps >= 8 rows, preview bottom 8
        let narrow = CellRect::new(0, 0, 60, 30);
        let (list, preview) = picker_split(narrow, strop_picker::Kind::Files, false).unwrap();
        let preview = preview.unwrap();
        assert_eq!(preview.height, 8);
        assert_eq!(preview.y, list.y + list.height);
        // A 20x6 area still fits one content row (inner height 3 =
        // input 2 + 1): the split exists. One row shorter, the field
        // owns the card and there is no results viewport.
        assert!(
            picker_split(CellRect::new(0, 0, 20, 6), strop_picker::Kind::Files, false).is_some()
        );
        assert!(
            picker_split(CellRect::new(0, 0, 20, 5), strop_picker::Kind::Files, false).is_none()
        );
    }
}
