//! The AR09 view serialization: a [`ViewSnapshot`] over the engine's
//! prepared view (AR03), keyed by view generation + per-pane document
//! revision, and the sparse delta between two snapshots. Never a
//! whole-Editor clone: bounded text windows from the pane's own budget.

use strop_engine::editor::prepare::{CellRect, WindowBounds};
use strop_engine::editor::Editor;
use strop_ui_protocol::{
    Geometry, PaneDelta, PaneSnapshot, Rect, ViewBounds, ViewDelta, ViewSnapshot,
};

/// Serialize the current prepared view. Callers prepare first; the
/// generation is the accepted preparation's.
pub fn build(editor: &Editor, generation: u64) -> ViewSnapshot {
    let prepared = editor.prepared_view();
    let panes = prepared
        .panes
        .iter()
        .map(|pane| {
            let (bounds, lines) = match editor.document(pane.doc) {
                Some(document) => {
                    let top = pane
                        .view_top
                        .min(document.buf.len_lines().saturating_sub(1));
                    let end = top
                        .saturating_add(usize::from(pane.budget.height))
                        .min(document.buf.len_lines());
                    let lines = (top..end)
                        .map(|line| document.buf.line_text(line))
                        .collect();
                    (bounds(pane.bounds), lines)
                }
                // A vanished source is explicit, never an empty success.
                None => (ViewBounds::Error, Vec::new()),
            };
            PaneSnapshot {
                document: pane.doc,
                revision: pane.revision,
                bounds,
                cursor: pane.cursor,
                view_top: pane.view_top,
                hscroll: pane.hscroll.get(),
                terminal_input: pane.terminal_input,
                overlays: pane.overlays,
                rect: rect(pane.rect),
                budget: rect(pane.budget),
                window_top: pane.view_top,
                lines,
            }
        })
        .collect();
    let state = match serde_json::from_str(&strop_engine::editor::state_json(editor)) {
        Ok(state) => state,
        Err(_) => unreachable!("state_json serializes a serde_json::Value"),
    };
    ViewSnapshot {
        generation,
        geometry: Geometry {
            columns: prepared.geometry.columns,
            rows: prepared.geometry.rows,
        },
        active_pane: prepared.active_pane,
        panes,
        state,
    }
}

/// The sparse update from `base` to `next`. Callers guarantee the pane
/// sets align (same length); a changed pane set ships as a snapshot.
pub fn diff(base: &ViewSnapshot, next: &ViewSnapshot) -> ViewDelta {
    debug_assert_eq!(base.panes.len(), next.panes.len());
    ViewDelta {
        base: base.generation,
        generation: next.generation,
        geometry: (base.geometry != next.geometry).then_some(next.geometry),
        active_pane: (base.active_pane != next.active_pane).then_some(next.active_pane),
        panes: base
            .panes
            .iter()
            .zip(&next.panes)
            .map(|(before, after)| {
                if before == after {
                    PaneDelta::Unchanged
                } else {
                    PaneDelta::Changed(after.clone())
                }
            })
            .collect(),
        state: (base.state != next.state).then(|| next.state.clone()),
    }
}

fn bounds(bounds: WindowBounds) -> ViewBounds {
    match bounds {
        WindowBounds::Complete => ViewBounds::Complete,
        WindowBounds::Partial => ViewBounds::Partial,
        WindowBounds::Loading => ViewBounds::Loading,
        WindowBounds::Stale => ViewBounds::Stale,
        WindowBounds::Error => ViewBounds::Error,
    }
}

fn rect(rect: CellRect) -> Rect {
    Rect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}
