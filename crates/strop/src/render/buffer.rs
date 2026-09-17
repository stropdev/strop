//! Pane rendering — one text renderer for every pane (0010 §3).
//! Active panes read live editor state with overlays; inactive panes
//! read their saved snapshot without. Same gutter, same guides, same
//! diff rows — the duplicated inactive-pane loop is gone, so panes
//! cannot drift apart again.
//!
//! 0031 R6: every pane owns a horizontal display-cell origin
//! (`Pane.hscroll`). Glyphs, overlays and all three caret kinds
//! (native, extra, static) project through the ONE `clip` seam in
//! strop-core — tabs, wide, combining and control graphemes included.
//! The fixed left margins (sign+number gutter, blame column, sidebar)
//! never scroll. Content columns stay `usize` end to end; narrowing to
//! u16 happens only at the final terminal-coordinate conversion.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use strop_core::id::{DisplayColumn, DocumentId};
use strop_core::layout::{clip, RopeGraphemes};

use crate::editor::{Editor, LayoutDir};

use super::diff;
use super::{dim_color, severity_color};
use super::{ACCENT, BASE, FLASH_BG, MUTED, PAIR_BG, PREVIEW_BG, SELECT_BG, TEXT};

mod content;
mod directory;
mod rows;
mod scrollbar;
use content::{content_spans, fixed_spans};
use rows::render_pane;

/// Width of the standard gutter: sign column + 3-digit number + space
/// (engine-owned; preparation clamps against the same constant).
pub(crate) const GUTTER: u16 = strop_engine::editor::prepare::STANDARD_GUTTER as u16;

/// Invariant guard: every pane row is written out to the pane's full
/// width. ratatui's Paragraph clears the cells its lines don't touch
/// today, but a renderer relying on that is one widget swap away from
/// resurrecting two-frames-old glyphs (the double-buffer keeps frame
/// N-2's cells) — pad explicitly instead.
fn pad_row(mut line: Line<'static>, width: u16) -> Line<'static> {
    let pad = usize::from(width).saturating_sub(line.width());
    if pad != 0 {
        line.spans.push(Span::raw(" ".repeat(pad)));
    }
    line
}

/// One pane's view of a buffer. `overlays` is false for inactive panes:
/// preview/search/selection/flash belong to the pane being driven.
#[derive(Clone, Copy)]
pub(crate) struct PaneView {
    pub(crate) doc: DocumentId,
    pub(crate) cursor: usize,
    pub(crate) view_top: usize,
    pub(crate) hscroll: DisplayColumn,
    pub(crate) overlays: bool,
}

/// Paint the prepared panes (AR03): the published windows preparation
/// clamped and admitted work for — doc, revision-keyed scroll/cursor and
/// the exact cell rects. Paint never recomputes or adjusts them.
pub(crate) fn render_panes(editor: &Editor, frame: &mut Frame, area: Rect) -> Rect {
    let prepared = editor.prepared_view();
    let n = prepared.panes.len();
    let mut active_rect = Rect::new(area.x, area.y, 0, 0);
    if n == 0 {
        return active_rect;
    }
    let is_row = editor.layout() == LayoutDir::Row;
    let axis = if is_row {
        usize::from(area.width)
    } else {
        usize::from(area.height.saturating_sub(1))
    };
    let mut offset = 0usize;
    for (i, window) in prepared.panes.iter().enumerate() {
        let rect = super::from_cells(window.rect);
        let rect = &rect;
        let active = i == prepared.active_pane;
        let w = usize::from(rect.width);
        let h = usize::from(rect.height);
        if n > 1 && h > 0 && w > 0 {
            let document = editor.doc(window.doc);
            let name = document
                .remote_metadata()
                .map(|remote| remote.file.to_string())
                .or_else(|| {
                    document
                        .buf
                        .path
                        .as_ref()
                        .map(|path| path.display().to_string())
                })
                .or_else(|| document.buf.name.clone())
                .unwrap_or_else(|| "[No Name]".into());
            let flags = format!(
                "{}{}",
                if document.buf.dirty { " *" } else { "" },
                if document.buf.readonly { " [RO]" } else { "" }
            );
            let budget = w.saturating_sub(super::text::width(&flags) + 2);
            let title = format!(
                "{} {}{}",
                if active { "▌" } else { " " },
                super::text::clip_end(&name, budget),
                flags
            );
            let spans = fixed_spans(
                vec![Span::styled(
                    title,
                    Style::default()
                        .fg(if active { TEXT } else { MUTED })
                        .bg(BASE),
                )],
                w,
                document.indent.width.max(1),
            );
            frame.render_widget(
                Paragraph::new(Line::from(spans)),
                Rect {
                    y: rect.y.saturating_sub(1),
                    height: 1,
                    width: rect.width,
                    x: rect.x,
                },
            );
        }
        if active {
            active_rect = *rect;
        }
        let pane = &editor.panes()[i];
        let view = PaneView {
            doc: window.doc,
            cursor: window.cursor,
            view_top: window.view_top,
            hscroll: window.hscroll,
            overlays: active,
        };
        let terminal_input = editor.terminal_view_input(pane);
        if w != 0 && h != 0 && editor.document(view.doc).is_some() {
            // 0064 §1: the scrollbar column is reserved before every
            // text budget the pane computes — content, carets and the
            // terminal grid all live inside it.
            let text_rect = scrollbar::reserved(*rect);
            if terminal_input {
                super::terminal::render(editor, frame, text_rect, view.doc, active);
                scrollbar::terminal(editor, frame, *rect, view.doc);
            } else {
                render_pane(editor, frame, text_rect, &view);
                scrollbar::render(editor, frame, *rect, &view);
                if active {
                    render_extra_cursors(editor, frame, text_rect, &view);
                } else {
                    render_static_caret(editor, frame, text_rect, &view);
                }
            }
        }
        let size = if is_row { w } else { h };
        let divider = offset.saturating_add(size);
        if i + 1 < n && divider < axis {
            if is_row {
                let dx = area.x + divider as u16;
                let bottom = area.y + area.height.saturating_sub(1);
                for dy in area.y..bottom {
                    let cell = &mut frame.buffer_mut()[(dx, dy)];
                    cell.set_symbol("│");
                    cell.set_fg(Color::Rgb(0x3a, 0x3d, 0x4d));
                    cell.set_bg(BASE);
                }
            } else {
                let dy = area.y + divider as u16;
                for dx in area.x..area.x + area.width {
                    let cell = &mut frame.buffer_mut()[(dx, dy)];
                    cell.set_symbol("─");
                    cell.set_fg(Color::Rgb(0x3a, 0x3d, 0x4d));
                    cell.set_bg(BASE);
                }
            }
        }
        offset = divider.saturating_add(1);
    }
    active_rect
}

/// The single content-to-terminal coordinate conversion: project a byte
/// offset through the pane's vertical top and horizontal origin, add
/// the fixed left inset, and narrow to u16 exactly once. Rows above
/// `top` and columns outside the pane are `None`, never aliased.
pub(crate) fn caret_position(
    editor: &Editor,
    area: Rect,
    doc: DocumentId,
    byte: usize,
    top: usize,
    origin: DisplayColumn,
) -> Option<(u16, u16)> {
    let buf = &editor.doc(doc).buf;
    let row = buf.line_of(byte).checked_sub(top)?;
    if row >= usize::from(area.height) {
        return None;
    }
    let column = buf.try_cell_col_with_tab(byte, editor.indentation_at(doc, byte).width)?;
    let relative = column.get().checked_sub(origin.get())?;
    let col = editor.left_inset(doc).checked_add(relative)?;
    if col >= usize::from(area.width) {
        return None;
    }
    Some((area.x + col as u16, area.y + row as u16))
}

/// The inactive pane's position, unfocused: a muted block on the saved
/// cursor cell, offsets pane-local (unlike the native cursor).
fn render_static_caret(editor: &Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    if let Some(at) = caret_position(
        editor,
        area,
        view.doc,
        view.cursor,
        view.view_top,
        view.hscroll,
    ) {
        frame.buffer_mut()[at].set_bg(Color::Rgb(0x3a, 0x3d, 0x4d));
    }
}

/// Secondary cursors (0013 §4): solid blocks on the active pane, like
/// the native block cursor but painted.
fn render_extra_cursors(editor: &Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    if view.doc != editor.current() {
        return;
    }
    for byte in editor.extra_selections().iter().map(|s| s.head) {
        if let Some(at) = caret_position(editor, area, view.doc, byte, view.view_top, view.hscroll)
        {
            frame.buffer_mut()[at].set_bg(TEXT);
            frame.buffer_mut()[at].set_fg(BASE);
        }
    }
}

/// The per-pane, per-frame style inputs one content row composes:
/// base layers (syntax spans or a diff line) plus the active pane's
/// overlays. Inactive panes get the default (no overlays).
#[derive(Default)]
struct RowStyle<'a> {
    syn_spans: &'a [strop_syntax::Span],
    preview: Vec<strop_core::Range>,
    flash: Option<strop_core::Range>,
    selection: Option<strop_core::Range>,
    /// Occurrence selections' extra ranges (0049 §7): same paint as
    /// the primary selection, only on the active pane in Visual mode.
    extra_selections: Vec<strop_core::Range>,
    /// ctrl-v rectangle (0013 §4): cell columns (0031 R6) — a grapheme
    /// selects when its nonzero absolute cell interval intersects the
    /// inclusive rectangle.
    block: Option<crate::editor::BlockRect>,
    /// Explicit match ranges (0031: never pattern-length assumptions).
    search_hits: &'a [strop_grammar::SearchMatch],
    /// Matching-delimiter overlay (0051 §7 R09): the pair's endpoints
    /// in VIEW bytes, each only when the engine mapped it into the
    /// viewed document (collection excerpt of the same source).
    pair_first: Option<usize>,
    pair_second: Option<usize>,
    find: Option<crate::editor::FindPending>,
    /// Diagnostic spans on this row: (col, end_col, severity) — the
    /// undercurl layer (0009 UX).
    diags: Vec<(usize, usize, strop_lsp::Severity)>,
    /// Set on diff-surface rows: typed origin drives colors (0010 §4).
    diff_line: Option<&'a strop_git::DiffLine>,
    /// Intra-line changed range on a diff row (delta-style emphasis).
    emphasis: Option<(usize, usize)>,
    /// Full-row background (diff add/del, structural band).
    row_bg: Option<Color>,
    row_fg: Option<Color>,
    /// Decorated-surface text (help/log/files/stats): styled spans
    /// whose concatenated content keeps the buffer line's exact byte
    /// prefix; anything past it is virtual EOL decoration.
    decorations: Vec<Span<'static>>,
    /// The cursor line's end-of-line diagnostic note, laid out at the
    /// line's ABSOLUTE end cell (tabs continue the line's stops).
    note: Option<(String, Style)>,
}

#[cfg(test)]
mod tests;
