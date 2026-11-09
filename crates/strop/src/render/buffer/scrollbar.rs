//! 0064 §1 / 0088 §2 rule 4: the per-pane scrollbar column — reserved
//! before text budgets, painted only when it carries information. No
//! track: a thumb whose length is the visible fraction and whose start
//! is the viewport's fraction of the travel space, plus the Git overview
//! of changed spans, and nothing at all when the content fits.
//!
//! Contract: the scrollbar is never an input owner; the hunk mapping
//! is fractional over the document's own line space (monotonic and
//! stable under resize).

use super::PaneView;
use crate::editor::Editor;
use crate::render::style::Tokens;
use ratatui::layout::Rect;
use ratatui::style::Color;

/// The pane's reserved scrollbar column: the rightmost column of the
/// pane rect, excluded from every text budget the pane computes.
pub(super) fn reserved(rect: Rect) -> Rect {
    Rect {
        width: rect.width.saturating_sub(1),
        ..rect
    }
}

/// One editor pane: when the document scrolls, the Git overview of the
/// visible document's changed spans (`▎` in the sign colours — sparse
/// over the hunk signs, never a scan of every line) under the viewport
/// thumb. A document that fits paints nothing: the gutter already shows
/// every sign.
pub(super) fn render(
    editor: &Editor,
    t: &Tokens,
    output: &mut ratatui::Frame,
    rect: Rect,
    view: &PaneView,
) {
    let Some(doc) = editor.document(view.doc) else {
        return;
    };
    let total = doc.buf.len_lines();
    let track_h = usize::from(rect.height);
    let rows = usize::from(reserved(rect).height);
    if rect.width == 0 || track_h == 0 || total <= rows {
        return;
    }
    let track_x = rect.x + rect.width - 1;
    // The Git overview: only the document whose hunks are live —
    // another pane's document shows the thumb alone.
    if view.doc == editor.current() {
        for (line, sign) in editor.sign_lines() {
            let row = line_to_row(line.saturating_sub(1), total, track_h);
            let color = match sign {
                '+' => t.diff_add,
                '~' => t.diff_change,
                _ => t.diff_del,
            };
            let cell = &mut output.buffer_mut()[(track_x, rect.y + row as u16)];
            cell.set_symbol("▎");
            cell.set_fg(color);
            cell.set_bg(t.base);
        }
    }
    // Viewport thumb last: position is the primary signal. The thumb's
    // LENGTH is the visible fraction of the document (0068 B2: a
    // 1-cell thumb says nothing about how much you're seeing).
    let thumb = Thumb {
        track_h,
        visible: rows,
        scrollable: total - rows,
    };
    thumb.paint(output, track_x, rect.y, view.view_top, t.muted, t.base);
}

/// The viewport thumb: `visible/total` of the track, minimum one cell,
/// scrolling the remaining travel space with the pane's position.
struct Thumb {
    track_h: usize,
    visible: usize,
    scrollable: usize,
}

impl Thumb {
    fn paint(
        &self,
        output: &mut ratatui::Frame,
        x: u16,
        y: u16,
        top: usize,
        color: Color,
        bg: Color,
    ) {
        let total = self.scrollable + self.visible;
        let len = (self.visible * self.track_h / total.max(1))
            .max(1)
            .min(self.track_h);
        let room = self.track_h - len;
        let start = if self.scrollable == 0 {
            0
        } else {
            (top.min(self.scrollable) * room)
                .checked_div(self.scrollable)
                .unwrap_or(0)
        };
        for row in start..(start + len).min(self.track_h) {
            let cell = &mut output.buffer_mut()[(x, y + row as u16)];
            cell.set_symbol("▮");
            cell.set_fg(color);
            cell.set_bg(bg);
        }
    }
}

/// One terminal pane: viewport position only (0064 §1) over the
/// projection's bounded history — no Git overlay, no illusion of
/// infinity, nothing when the history fits.
pub(super) fn terminal(
    editor: &Editor,
    t: &Tokens,
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
    let total = frame.rows.len();
    let rows = usize::from(reserved(rect).height);
    if total <= rows {
        return;
    }
    let thumb = Thumb {
        track_h,
        visible: rows,
        scrollable: total - rows,
    };
    thumb.paint(
        output,
        rect.x + rect.width - 1,
        rect.y,
        frame.history_rows,
        t.muted,
        rgb(frame.palette.background),
    );
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
