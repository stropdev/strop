//! 0064 §1: the per-pane border-column scrollbar — one thin,
//! transparent-track hint surface per editor and terminal pane, in the
//! reserved-vocabulary established for pickers (0050 §8): the column
//! is reserved before text budgets, the track is quiet, the thumb
//! carries viewport position and Git spans carry the change overview.
//!
//! Contract: the scrollbar is never an input owner; the hunk mapping
//! is fractional over the document's own line space (monotonic and
//! stable under resize); unavailable Git state renders an empty track,
//! never a false neutral.

use super::PaneView;
use super::{ACCENT, BASE};
use crate::editor::Editor;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The quiet track (0050's reserved-column vocabulary).
const TRACK: Color = Color::Rgb(0x2a, 0x2c, 0x3a);
/// Gutter palette: additions green, changes amber, deletions red.
const ADD: Color = Color::Rgb(0xa9, 0xc4, 0x7c);
const DELETE: Color = Color::Rgb(0xe8, 0x67, 0x7a);

/// The pane's reserved scrollbar column: the rightmost column of the
/// pane rect, excluded from every text budget the pane computes.
pub(super) fn reserved(rect: Rect) -> Rect {
    Rect {
        width: rect.width.saturating_sub(1),
        ..rect
    }
}

/// One editor pane: viewport thumb plus the Git overview of the
/// visible document's changed spans (added `▎`, changed `▎`, deleted
/// `▎`, gutter colors) — sparse over the hunk signs, never a scan of
/// every line.
pub(super) fn render(editor: &Editor, output: &mut ratatui::Frame, rect: Rect, view: &PaneView) {
    let Some(doc) = editor.docs.get(view.doc) else {
        return;
    };
    let total = doc.buf.len_lines();
    let track_h = usize::from(rect.height);
    if rect.width == 0 || track_h == 0 || total == 0 {
        return;
    }
    let track_x = rect.x + rect.width - 1;
    let rows = usize::from(reserved(rect).height);
    paint_track(output, track_x, rect.y, track_h);
    // The Git overview: only the document whose hunks are live —
    // another pane's document renders the honest empty track.
    if view.doc == editor.current() {
        for (line, sign) in editor.sign_lines() {
            let row = line_to_row(line.saturating_sub(1), total, track_h);
            let color = match sign {
                '+' => ADD,
                '~' => ACCENT,
                _ => DELETE,
            };
            let cell = &mut output.buffer_mut()[(track_x, rect.y + row as u16)];
            cell.set_symbol("▎");
            cell.set_fg(color);
            cell.set_bg(BASE);
        }
    }
    // Viewport thumb last: position is the primary signal.
    if total > rows {
        let scrollable = total - rows;
        let thumb = line_to_row(view.view_top, scrollable + 1, track_h);
        let cell = &mut output.buffer_mut()[(track_x, rect.y + thumb as u16)];
        cell.set_symbol("▮");
        cell.set_fg(ACCENT);
        cell.set_bg(BASE);
    }
}

/// One terminal pane: viewport position only (0064 §1) over the
/// projection's bounded history — no Git overlay, no illusion of
/// infinity.
pub(super) fn terminal(
    editor: &Editor,
    output: &mut ratatui::Frame,
    rect: Rect,
    document: strop_core::id::DocumentId,
) {
    let track_h = usize::from(rect.height);
    if rect.width == 0 || track_h == 0 {
        return;
    }
    let Some(frame) = editor.terminal_frame(document, true) else {
        return;
    };
    paint_track(output, rect.x + rect.width - 1, rect.y, track_h);
    let total = frame.rows.len();
    let rows = usize::from(reserved(rect).height);
    if total > rows {
        let scrollable = total - rows;
        let thumb = line_to_row(frame.history_rows, scrollable + 1, track_h);
        let cell = &mut output.buffer_mut()[(rect.x + rect.width - 1, rect.y + thumb as u16)];
        cell.set_symbol("▮");
        cell.set_fg(ACCENT);
        cell.set_bg(rgb(frame.palette.background));
    }
}

fn paint_track(output: &mut ratatui::Frame, x: u16, y: u16, height: usize) {
    for row in 0..height {
        let cell = &mut output.buffer_mut()[(x, y + row as u16)];
        cell.set_symbol("│");
        cell.set_fg(TRACK);
        cell.set_bg(BASE);
    }
}

/// Fractional, monotonic line→row mapping: stable under resize, and
/// wrapped and unwrapped layouts agree because it maps the document's
/// own line space, not rendered rows.
fn line_to_row(line: usize, total: usize, track_h: usize) -> usize {
    debug_assert!(total > 0);
    if track_h <= 1 {
        return 0;
    }
    ((line as f64 / total.max(1) as f64) * (track_h - 1) as f64).round() as usize
}

fn rgb(value: strop_terminal::model::Rgb) -> Color {
    Color::Rgb(value.red, value.green, value.blue)
}
