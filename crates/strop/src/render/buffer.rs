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
//!
//! Chrome (0088 §2 rule 4): with two or more panes each pane's identity
//! sits in its top border (`╭ 1 statusline.rs ● ───╮`), focused border
//! `fg.muted`, unfocused `fg.faint`; one divider idiom separates panes.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use strop_core::id::{DisplayColumn, DocumentId};
use strop_core::layout::{clip, RopeGraphemes};

use crate::editor::{Editor, LayoutDir};

use super::diff;
use super::style::Tokens;

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
pub(crate) fn render_panes(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) -> Rect {
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
        let active = i == prepared.active_pane;
        let w = usize::from(rect.width);
        let h = usize::from(rect.height);
        if n > 1 && h > 0 && w > 0 {
            render_title_border(editor, t, frame, rect, i, window.doc, active);
        }
        if active {
            active_rect = rect;
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
            let text_rect = scrollbar::reserved(rect);
            if terminal_input {
                super::terminal::render(editor, t, frame, text_rect, view.doc, active);
                scrollbar::terminal(editor, t, frame, rect, view.doc);
            } else {
                render_pane(editor, t, frame, text_rect, &view);
                scrollbar::render(editor, t, frame, rect, &view);
                if active {
                    render_extra_cursors(editor, t, frame, text_rect, &view);
                } else {
                    render_static_caret(editor, t, frame, text_rect, &view);
                }
            }
        }
        let size = if is_row { w } else { h + usize::from(n > 1) };
        let divider = offset.saturating_add(size);
        if i + 1 < n && divider < axis {
            render_divider(t, frame, area, rect, is_row, divider, active);
        }
        offset = divider.saturating_add(1);
    }
    active_rect
}

/// The pane's identity in its top border: number, workspace-relative
/// name, dirty `●` (warning) and `[RO]`. `Document::label` is the shared
/// document-aware projection — local paths workspace-relative, outside
/// paths absolute, remote resources keeping their authority;
/// never the active-document modeline projection for an inactive pane.
#[allow(clippy::too_many_arguments)]
fn render_title_border(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    rect: Rect,
    index: usize,
    doc: DocumentId,
    active: bool,
) {
    let document = editor.doc(doc);
    let root = document
        .buf
        .path
        .as_deref()
        .and_then(|path| editor.workspace_root_path(&strop_workspace::Filesystem::Local, path))
        .unwrap_or(editor.cwd());
    let name = document.label(root);
    let w = usize::from(rect.width);
    // A terminal buffer is read-only by nature, not by flag: `[RO]` would
    // say nothing (0090 §4).
    let is_terminal = matches!(
        document.source,
        strop_engine::editor::DocumentSource::Terminal(_)
    );
    let readonly = document.buf.readonly && !is_terminal;
    // `╭ N ` + name + flags + ` ─╮`: the flags and corners are reserved
    // before the name clips, so focus, dirty and read-only survive narrow.
    let flags = format!(
        "{}{}",
        if document.buf.is_dirty() { " ●" } else { "" },
        if readonly { " [RO]" } else { "" }
    );
    let number = format!("{} ", index + 1);
    let fixed = 2 + super::clip::width(&number) + super::clip::width(&flags);
    let outside_workspace = document
        .buf
        .path
        .as_deref()
        .is_some_and(|path| path.is_absolute() && path.strip_prefix(root).is_err());
    let retain_authority = outside_workspace || document.remote_metadata().is_some() || is_terminal;
    // The title is the pane's heading (0088 §2 "File names"): authority
    // muted, directory secondary, name bold. An inactive pane is all
    // muted, the name still bold; a terminal's `term://` identity is one
    // name, clipped from its end.
    let label = if is_terminal {
        super::file_name::Label::local(name)
    } else {
        super::file_name::Label::parse(&name)
    };
    // the closing ` ─╮` is reserved only while the whole name fits; a
    // tight pane spends every cell on identity instead
    let closes = fixed + label.width() + 3 <= w;
    let budget = w.saturating_sub(fixed + if closes { 3 } else { 0 });
    let label = if is_terminal {
        super::file_name::Label::local(super::clip::clip_end(&label.path, budget))
    } else if retain_authority {
        label.clipped(budget)
    } else {
        super::file_name::Label::local(super::clip::path_suffix(&label.path, budget))
    };
    let (dir, leaf) = if is_terminal {
        ("", label.path.as_str())
    } else {
        super::file_name::split(&label.path)
    };
    let border = Style::default().fg(if active { t.muted } else { t.faint });
    let ink = Style::default().fg(if active { t.text } else { t.muted });
    let (dir_ink, leaf_ink) = if active {
        (
            super::file_name::dir_style(t),
            super::file_name::name_style(super::file_name::Role::Heading, t),
        )
    } else {
        (ink, ink.add_modifier(Modifier::BOLD))
    };
    let mut spans = vec![Span::styled("╭ ", border), Span::styled(number, ink)];
    if let Some(authority) = &label.authority {
        spans.push(Span::styled(
            format!("{authority} "),
            if active {
                super::file_name::authority_style(t)
            } else {
                ink
            },
        ));
    }
    spans.push(Span::styled(dir.to_owned(), dir_ink));
    spans.push(Span::styled(leaf.to_owned(), leaf_ink));
    if document.buf.is_dirty() {
        spans.push(Span::styled(" ●", Style::default().fg(t.warning)));
    }
    if readonly {
        spans.push(Span::styled(" [RO]", Style::default().fg(t.warning)));
    }
    if closes {
        let used: usize = spans.iter().map(|span| span.width()).sum();
        let rule = w.saturating_sub(used + 1);
        if rule > 0 {
            spans.push(Span::styled(
                format!(" {}", "─".repeat(rule.saturating_sub(1))),
                border,
            ));
        }
        spans.push(Span::styled("╮", border));
    }
    let spans = fixed_spans(spans, w, document.indent.width.max(1));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.base)),
        Rect {
            y: rect.y.saturating_sub(1),
            height: 1,
            width: rect.width,
            x: rect.x,
        },
    );
}

/// The one divider idiom: a `│` column between side-by-side panes (blank
/// on the title row, where the corners already separate the cards), and
/// a `╰───╯` closing border under a stacked pane — the pane it belongs to
/// decides the colour.
#[allow(clippy::too_many_arguments)]
fn render_divider(
    t: &Tokens,
    frame: &mut Frame,
    area: Rect,
    rect: Rect,
    is_row: bool,
    divider: usize,
    active: bool,
) {
    let color = if active { t.muted } else { t.faint };
    if is_row {
        let dx = area.x + divider as u16;
        let bottom = area.y + area.height.saturating_sub(1);
        for dy in area.y..bottom {
            let cell = &mut frame.buffer_mut()[(dx, dy)];
            let symbol = if dy + 1 == rect.y { " " } else { "│" };
            cell.set_symbol(symbol);
            cell.set_fg(t.faint);
            cell.set_bg(t.base);
        }
    } else {
        let dy = area.y + divider as u16;
        let right = area.x + area.width;
        for dx in area.x..right {
            let cell = &mut frame.buffer_mut()[(dx, dy)];
            let symbol = if dx == area.x {
                "╰"
            } else if dx + 1 == right {
                "╯"
            } else {
                "─"
            };
            cell.set_symbol(symbol);
            cell.set_fg(color);
            cell.set_bg(t.base);
        }
    }
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

/// The inactive pane's position, unfocused: a faint block on the saved
/// cursor cell, offsets pane-local (unlike the native cursor).
fn render_static_caret(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    area: Rect,
    view: &PaneView,
) {
    if let Some(at) = caret_position(
        editor,
        area,
        view.doc,
        view.cursor,
        view.view_top,
        view.hscroll,
    ) {
        frame.buffer_mut()[at].set_bg(t.faint);
    }
}

/// Secondary cursors (0013 §4): solid blocks on the active pane, like
/// the native block cursor but painted.
fn render_extra_cursors(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    area: Rect,
    view: &PaneView,
) {
    if view.doc != editor.current() {
        return;
    }
    for byte in editor.extra_selections().iter().map(|s| s.head) {
        if let Some(at) = caret_position(editor, area, view.doc, byte, view.view_top, view.hscroll)
        {
            frame.buffer_mut()[at].set_bg(t.text);
            frame.buffer_mut()[at].set_fg(t.base);
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
    /// Set on diff-surface rows: typed origin drives colors (0010 §4).
    diff_line: Option<&'a strop_git::DiffLine>,
    /// Intra-line changed range on a diff row (delta-style emphasis).
    emphasis: Option<(usize, usize)>,
    /// Full-row background (diff add/del, structural band, cursor line).
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
