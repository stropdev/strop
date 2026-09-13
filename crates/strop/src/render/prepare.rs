//! AR01 frame preparation: the one place the paint path may admit work or
//! adjust viewports. Idempotent — a stamp over view-relevant state and
//! geometry skips viewport/work preparation when nothing changed; the one
//! admission that is externally keyed (git hunks, already internally
//! idempotent and ticket-free when settled) runs every prepare. `render`
//! runs this, then paints through readonly `&Editor` queries only.
use super::buffer::{self, PaneView};
use crate::editor::{Editor, Mode};
use ratatui::layout::Rect;
use std::hash::{Hash, Hasher};

pub(super) fn prepare_frame(editor: &mut Editor, area: Rect) {
    // AR01 invariant: paint never admits work. Viewport clamping below is
    // pure state adjustment; worker admission runs only when the stamp
    // changed (and each admitted path is additionally idempotent itself).
    editor.refresh_hunks();
    let stamp = frame_stamp(editor, area);
    let stamp_changed = stamp != editor.frame_stamp;
    editor.frame_stamp = stamp;

    // Active-pane viewport and terminal geometry own their exact pane rect.
    let rects = buffer::pane_rects(editor, area);
    for (index, rect) in rects.iter().enumerate() {
        if index != editor.active_pane {
            continue;
        }
        let pane_doc = editor.panes[index].doc;
        let terminal_input = editor.terminal_view_input(&editor.panes[index]);
        let h = usize::from(rect.height);
        if h == 0 {
            editor.view_rows = 0;
        } else if terminal_input {
            editor.view_rows = h;
        } else {
            editor.scroll_to_cursor(h);
        }
        if !terminal_input && h != 0 {
            let width = usize::from(rect.width)
                .saturating_sub(super::diff::left_inset(editor, editor.current()));
            let head = editor.head();
            let column_probe = editor
                .buf()
                .try_cell_col_with_tab(head, editor.indentation_at(pane_doc, head).width);
            if let Some(column) = column_probe {
                editor.view_mut().reveal_column(column, width);
            }
        }
        if terminal_input && rect.width != 0 && rect.height != 0 {
            editor.prepare_terminal_geometry(pane_doc, rect.width, rect.height);
        }
    }

    // Visible-window analysis and pair admission for every pane, after the
    // viewport settled. Paint reads the same windows back as pure cache hits.
    let views: Vec<PaneView> = editor
        .panes
        .iter()
        .enumerate()
        .map(|(index, pane)| PaneView {
            doc: pane.doc,
            cursor: pane.sels.primary().head,
            view_top: pane.view_top,
            hscroll: pane.hscroll,
            overlays: index == editor.active_pane,
        })
        .collect();
    if stamp_changed {
        for (index, view) in views.iter().enumerate() {
            buffer::admit_visible_work(editor, rects[index], view);
        }
    }

    super::picker::reveal_for_prepare(editor, area);
}

/// Everything stamp-guarded preparation depends on: geometry, focus,
/// per-pane view state, document revisions, picker selection and the
/// published hunk/terminal generations. Equal stamps mean preparation is
/// already done for exactly this presentation state.
fn frame_stamp(editor: &Editor, area: Rect) -> u64 {
    let mut stamp = std::collections::hash_map::DefaultHasher::new();
    (area.width, area.height).hash(&mut stamp);
    editor.panes.len().hash(&mut stamp);
    editor.active_pane.hash(&mut stamp);
    editor.focus_epoch().hash(&mut stamp);
    usize::from(matches!(editor.mode, Mode::Insert)).hash(&mut stamp);
    for pane in &editor.panes {
        (pane.doc.index(), pane.doc.generation()).hash(&mut stamp);
        editor
            .docs
            .get(pane.doc)
            .map(|document| document.buf.revision().get())
            .unwrap_or(u64::MAX)
            .hash(&mut stamp);
        pane.sels.primary().head.hash(&mut stamp);
        pane.view_top.hash(&mut stamp);
        pane.hscroll.get().hash(&mut stamp);
        usize::from(pane.terminal_input).hash(&mut stamp);
        editor
            .terminal_frame(pane.doc, true)
            .map(|frame| frame.revision)
            .unwrap_or(0)
            .hash(&mut stamp);
    }
    editor
        .picker
        .as_ref()
        .map(|glue| {
            (
                glue.picker.selected,
                glue.picker.items.len(),
                glue.picker.rows.len(),
            )
        })
        .hash(&mut stamp);
    editor.hunks.identity().hash(&mut stamp);
    stamp.finish()
}
