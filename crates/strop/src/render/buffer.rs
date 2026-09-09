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
use super::{ACCENT, BASE, FLASH_BG, MUTED, PREVIEW_BG, SELECT_BG, TEXT};

/// Width of the standard gutter: sign column + 3-digit number + space.
pub(crate) const GUTTER: u16 = 5;

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
struct PaneView {
    doc: DocumentId,
    cursor: usize,
    view_top: usize,
    hscroll: DisplayColumn,
    overlays: bool,
}

/// Render all panes and return the active pane's rect (the native
/// cursor lives there — offsets included, which the full-area version
/// got wrong in splits). Geometry is computed in `usize` and narrowed
/// once, per pane, at the `Rect` boundary.
pub(crate) fn render_panes(editor: &mut Editor, frame: &mut Frame, area: Rect) -> Rect {
    let n = editor.panes.len();
    let mut active_rect = Rect::new(area.x, area.y, 0, 0);
    if n == 0 {
        return active_rect;
    }
    let is_row = editor.layout == LayoutDir::Row;
    let total_w = usize::from(area.width);
    let total_h = usize::from(area.height.saturating_sub(1)); // statusline
    let axis = if is_row { total_w } else { total_h };
    let usable = axis.saturating_sub(n - 1);
    let base = usable / n;
    let mut offset = 0usize;
    for i in 0..n {
        let size = if i + 1 == n {
            usable - base * (n - 1)
        } else {
            base
        };
        let size = size.min(axis.saturating_sub(offset));
        let offset_in_area = offset.min(axis);
        let (x, y, w, h) = if is_row {
            (
                usize::from(area.x) + offset_in_area,
                usize::from(area.y),
                size,
                total_h,
            )
        } else {
            (
                usize::from(area.x),
                usize::from(area.y) + offset_in_area,
                total_w,
                size,
            )
        };
        let rect = Rect::new(x as u16, y as u16, w as u16, h as u16);
        let active = i == editor.active_pane;
        if active {
            // The pane's own height feeds the vertical viewport (a
            // Column split pane is not the terminal height); a
            // zero-height pane cannot reveal a row and keeps its top.
            if h == 0 {
                editor.view_rows = 0;
            } else {
                editor.scroll_to_cursor(h);
            }
            if let Some(column) = editor
                .buf()
                .try_cell_col_with_tab(editor.head(), editor.config.tab_size)
            {
                let width = w.saturating_sub(diff::left_inset(editor, editor.current()));
                editor.view_mut().reveal_column(column, width);
            }
            active_rect = rect;
        }
        let pane = &editor.panes[i];
        let view = PaneView {
            doc: pane.doc,
            cursor: pane.sels.primary().head,
            view_top: pane.view_top,
            hscroll: pane.hscroll,
            overlays: active,
        };
        if w != 0 && h != 0 && editor.docs.get(view.doc).is_some() {
            render_pane(editor, frame, rect, &view);
            if active {
                render_extra_cursors(editor, frame, rect, &view);
            } else {
                render_static_caret(editor, frame, rect, &view);
            }
        }
        let divider = offset.saturating_add(size);
        if i + 1 < n && divider < axis {
            if is_row {
                let dx = area.x + divider as u16;
                for dy in area.y..area.y + total_h as u16 {
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
    let column = buf.try_cell_col_with_tab(byte, editor.config.tab_size)?;
    let relative = column.get().checked_sub(origin.get())?;
    let col = diff::left_inset(editor, doc).checked_add(relative)?;
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

/// Render one pane's rows: gutter, syntax/decoration, overlays, guides.
/// Ordinary file and diff rows stream graphemes straight off the rope —
/// no whole-line String, no layout vector; only decorated surfaces
/// (help/log/stats) keep their existing owned styled text.
fn render_pane(editor: &mut Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    let rows = usize::from(area.height);
    let (cur_line, first, last) = {
        let buf = &editor.doc(view.doc).buf;
        let last_line = view.view_top.saturating_add(rows).min(buf.len_lines());
        (
            buf.line_of(view.cursor),
            buf.line_start(view.view_top),
            buf.line_end(last_line.saturating_sub(1)),
        )
    };
    let analysis = editor.document_analysis(
        view.doc,
        first,
        last,
        view.hscroll.get(),
        area.width as usize,
    );
    let syn_spans = analysis
        .as_ref()
        .map_or(&[][..], |analysis| analysis.spans.as_slice());
    // one search entry point (0031: match ranges are explicit — the
    // current match is the hit containing the caret, never "pattern
    // length from the caret"); a query that cannot compile renders no
    // highlights and lets the modeline carry the error
    let search_hits = if view.overlays {
        analysis
            .as_ref()
            .and_then(|analysis| analysis.search.as_ref())
            .and_then(|summary| summary.as_ref().ok())
            .map_or(&[][..], |summary| summary.hits.as_slice())
    } else {
        &[]
    };
    let buf = &editor.doc(view.doc).buf;
    let surface = editor.doc(view.doc).surface_payload();

    // overlays read live editor state; only the active pane shows them
    let mut style = RowStyle {
        syn_spans,
        preview: if view.overlays {
            match editor.preview() {
                Ok(Some((ranges, _))) => ranges,
                Ok(None) | Err(_) => Vec::new(),
            }
        } else {
            Vec::new()
        },
        flash: view.overlays.then(|| editor.flash_range()).flatten(),
        selection: view.overlays.then(|| editor.visual_range()).flatten(),
        block: view.overlays.then(|| editor.block_rect_pub()).flatten(),
        search_hits,
        find: view.overlays.then(|| editor.find_candidates()).flatten(),
        ..Default::default()
    };
    // 0011 left-margin columns: the commit file sidebar (Diff surfaces
    // from the dive chain) and the blame gutter (file buffers) prepend
    // to every row; content width shrinks by what they take. The tree
    // was prepared by the worker; selection retains native path identity.
    let (sidebar, sidebar_focused) = match surface {
        Some(crate::editor::Surface::Diff {
            commit: Some(cf),
            sidebar_focus,
            ..
        }) => (
            Some((cf.files.sidebar(), &cf.files[..], cf.current.as_path())),
            *sidebar_focus,
        ),
        _ => (None, false),
    };
    let sidebar_w = sidebar
        .as_ref()
        .map_or(0, |(tree, _, _)| tree.outer_width());
    let blame = editor.blame_gutter_for(view.doc);
    let number_width = diff::number_gutter_width(editor, view.doc);
    let inset = sidebar_w + blame.map_or(0, |_| diff::BLAME_W) + number_width;
    let width = usize::from(area.width).saturating_sub(inset);
    // :help rows color by the section they sit under (render/help.rs)
    let mut help_section = String::new();
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for row in 0..rows {
        let line_idx = view.view_top.saturating_add(row);
        // the fixed margins: sidebar cell (or blank), then the blame
        // cell (or blank past the buffer's lines) — fitted to their
        // assigned width so wide/control text cannot move the inset
        let mut left: Vec<Span> = Vec::new();
        if let Some((tree, files, current)) = &sidebar {
            left.extend(fixed_spans(
                tree.row_spans(files, current, line_idx, sidebar_focused),
                sidebar_w,
                editor.config.tab_size,
            ));
        }
        if let Some(gutter) = blame {
            let span = match gutter.lines.get(line_idx) {
                // rootle rule: a commit's cell prints only on the first
                // line of its run — the gutter breathes, the run reads
                Some(bl) => {
                    let repeats_prev = line_idx > 0
                        && gutter
                            .lines
                            .get(line_idx - 1)
                            .is_some_and(|p| p.sha == bl.sha && p.author == bl.author);
                    if repeats_prev {
                        diff::blame_blank()
                    } else {
                        diff::blame_spans(bl, editor.tape.now().unix_seconds)
                    }
                }
                None => diff::blame_blank(),
            };
            left.extend(fixed_spans(
                vec![span],
                diff::BLAME_W,
                editor.config.tab_size,
            ));
        }
        if line_idx > buf.last_content_line() {
            left.push(Span::styled("~", Style::default().fg(MUTED)));
            lines.push(pad_row(Line::from(left), area.width));
            continue;
        }
        let start = buf.line_start(line_idx);
        let text = buf.text().byte_slice(start..buf.line_end(line_idx));

        // git memory surfaces decorate their rows from typed data
        // (0010 §4/§5): diff rows re-gutter, log/files rows re-color
        style.diff_line = None;
        style.emphasis = None;
        style.row_bg = None;
        style.row_fg =
            editor
                .doc(view.doc)
                .directory_metadata_ref()
                .map(|directory| {
                    match directory
                        .entry(strop_core::id::LineIndex::new(line_idx))
                        .map(|entry| entry.kind)
                    {
                        Some(strop_remote::RemoteEntryKind::Directory) => ACCENT,
                        Some(strop_remote::RemoteEntryKind::SymbolicLink) => {
                            Color::Rgb(0x89, 0xb4, 0xfa)
                        }
                        Some(strop_remote::RemoteEntryKind::File) => TEXT,
                        _ => MUTED,
                    }
                });
        style.decorations.clear();
        style.note = None;
        style.diags = if view.overlays {
            editor.diag_ranges_at(view.doc, line_idx + 1)
        } else {
            Vec::new()
        };
        match surface.and_then(|surface| surface.diff_row(line_idx)) {
            Some(crate::editor::DiffRow::Stats | crate::editor::DiffRow::HunkHeader(_)) => {
                // structural text uses the SAME fixed inset its caret
                // would; the band is row background, the text scrolls
                left.push(Span::raw(" ".repeat(number_width)));
                let decorated = diff::structural_row(surface.unwrap(), line_idx);
                style.row_bg = decorated.style.bg;
                style.decorations = decorated.spans;
            }
            Some(crate::editor::DiffRow::Line(dl)) => {
                left.extend(diff::diff_gutter(
                    dl,
                    line_idx == cur_line,
                    diff_digits(surface),
                ));
                style.diff_line = Some(dl);
                style.emphasis = diff::emphasis_span(surface, line_idx);
                style.row_bg = diff::origin_bg(dl.origin);
            }
            None => {
                let num_style = if line_idx == cur_line {
                    Style::default().fg(ACCENT)
                } else {
                    Style::default().fg(MUTED)
                };
                // Helix-grade gutter: a colored ▎ bar in the leftmost
                // column — diagnostics first, then git signs
                let (bar, bar_color) = gutter_mark(editor, view, line_idx);
                left.push(Span::styled(
                    bar,
                    Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                ));
                left.push(Span::styled(
                    format!("{:>digits$} ", line_idx + 1, digits = number_width - 2),
                    num_style,
                ));
                if let Some(row) =
                    diff::surface_list_row(surface, line_idx, width, line_idx == cur_line)
                {
                    // the quiet cursor-row band rides under overlays
                    // (search/visual/flash still override per cell)
                    style.decorations = row.spans;
                    style.row_bg = row.row_bg;
                } else if buf.name.as_deref() == Some("help") {
                    // the :help buffer gets house-style color
                    // (render/help.rs)
                    let text = text.to_string();
                    if text.starts_with('[') && text.ends_with(']') {
                        help_section = text.trim_matches(['[', ']']).to_string();
                    }
                    style.decorations = super::help::row_spans(&text, &help_section, width as u16);
                }
            }
        }
        // cursor-line end-of-line diagnostic (scoped to the one line —
        // you see what the dot means without leaving the buffer)
        if view.overlays && line_idx == cur_line {
            if let Some((sev, msg)) = editor.diag_message_at(view.doc, line_idx + 1) {
                let shown: String = msg.replace('\n', " · ").chars().take(80).collect();
                style.note = Some((
                    format!("  ▍ {shown}"),
                    Style::default()
                        .fg(dim_color(severity_color(sev)))
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }
        left.extend(content_spans(editor, view, start, text, &style, width));
        lines.push(pad_row(Line::from(left), area.width));
    }
    frame.render_widget(Paragraph::new(lines).style(Style::default().bg(BASE)), area);
    if let Some(analysis) = &analysis {
        for row in 0..rows {
            let line = view.view_top.saturating_add(row);
            if line > buf.last_content_line() {
                break;
            }
            for column in analysis.guides.columns(line) {
                let Some(x) = column
                    .get()
                    .checked_sub(view.hscroll.get())
                    .filter(|x| *x < width)
                else {
                    continue;
                };
                let at = (area.x + (inset + x) as u16, area.y + row as u16);
                let cell = &mut frame.buffer_mut()[at];
                if cell.symbol() == " " {
                    cell.set_symbol("│").set_fg(Color::Rgb(0x2e, 0x30, 0x42));
                }
            }
        }
    }
}

/// Digits per side for a Diff surface's number columns.
fn diff_digits(surface: Option<&crate::editor::Surface>) -> usize {
    let width = diff::gutter_width(surface);
    if width == super::buffer::GUTTER as usize {
        3
    } else {
        (width - 3) / 2
    }
}
/// The sign column: diagnostics win over git signs (merged gutter,
/// 0009), and only the pane's own buffer shows them.
fn gutter_mark(editor: &Editor, view: &PaneView, line_idx: usize) -> (&'static str, Color) {
    if let Some(sev) = editor.diag_severity_at(view.doc, line_idx + 1) {
        // severity dot (VSCode/gitui lesson: color reads faster than
        // letters) — the cursor line's EOL note carries the words
        return ("●", severity_color(sev));
    }
    // git signs: + add, ~ change, - deletion below (only for the
    // working buffer — surfaces have no path, so no leak)
    if view.doc == editor.current() {
        // the four states in one column: unstaged sign wins; staged-only
        // lines get the committed-adjacent tint (0014 wave 4)
        if editor.sign_at(line_idx + 1).is_none() && editor.sign_at_staged(line_idx + 1) {
            return ("▎", dim_color(Color::Rgb(0xa9, 0xc4, 0x7c)));
        }
        match editor.sign_at(line_idx + 1) {
            Some('+') => return ("▎", Color::Rgb(0xa9, 0xc4, 0x7c)),
            Some('~') => return ("▎", ACCENT),
            Some('-') => return ("▎", Color::Rgb(0xe8, 0x67, 0x7a)),
            _ => {}
        }
    }
    (" ", MUTED)
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
    /// ctrl-v rectangle (0013 §4): cell columns (0031 R6) — a grapheme
    /// selects when its nonzero absolute cell interval intersects the
    /// inclusive rectangle.
    block: Option<crate::editor::BlockRect>,
    /// Explicit match ranges (0031: never pattern-length assumptions).
    search_hits: &'a [strop_grammar::SearchMatch],
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

/// Content spans for one row, clipped to the pane's visible cell
/// window: syntax/decoration base, overlays composed on top (search <
/// preview < flash, 0001 §5.8). Styles are evaluated only for the
/// visible interval; a partially clipped wide grapheme becomes styled
/// blanks, never half a glyph; a zero-width cluster occupies no cell.
fn content_spans(
    editor: &Editor,
    view: &PaneView,
    start: usize,
    text: ropey::RopeSlice<'_>,
    style: &RowStyle<'_>,
    width: usize,
) -> Vec<Span<'static>> {
    if width == 0 {
        return Vec::new();
    }
    let tab = editor.config.tab_size.max(1);
    let buf = &editor.doc(view.doc).buf;
    let row = buf.line_of(start);
    let cur_line = buf.line_of(view.cursor);
    let decorated = (!style.decorations.is_empty()).then(|| {
        style
            .decorations
            .iter()
            .map(|s| s.content.as_ref())
            .collect::<String>()
    });
    let source = decorated
        .as_ref()
        .map_or(text, |s| ropey::RopeSlice::from(s.as_str()));
    let right = view.hscroll.get().saturating_add(width);
    let Some(checkpoint) =
        buf.layout_checkpoint(strop_core::id::LineIndex::new(row), view.hscroll, tab)
    else {
        return vec![Span::styled("layout pending", Style::default().fg(MUTED))];
    };
    if checkpoint.byte.get() > source.len_bytes() {
        return Vec::new();
    }
    let mut syn_idx = style
        .syn_spans
        .partition_point(|span| span.end <= start + checkpoint.byte.get());
    let mut decoration_index = 0usize;
    let mut decoration_end = style.decorations.first().map_or(0, |s| s.content.len());
    let mut spans = Vec::with_capacity(width);
    let mut used = 0usize;
    let mut end_cell = checkpoint.cell;
    let mut reached_end = true;
    for (glyph, grapheme) in RopeGraphemes::from_checkpoint(source, tab, checkpoint) {
        end_cell = glyph.cell + glyph.width;
        if glyph.cell.get() >= right {
            reached_end = false;
            break;
        }
        let Some(visible) = clip(glyph, view.hscroll, width) else {
            continue;
        };
        let i = glyph.byte;
        let pos = start + i;
        let is_source = i < text.len_bytes();
        let mut cell = Style::default().fg(style.row_fg.unwrap_or(TEXT));
        if let Some(bg) = style.row_bg {
            cell = cell.bg(bg);
        }
        while syn_idx < style.syn_spans.len() && style.syn_spans[syn_idx].end <= pos {
            syn_idx += 1;
        }
        if let Some(line) = style.diff_line {
            cell = cell.fg(diff::origin_fg(line.origin));
        }
        if is_source && syn_idx < style.syn_spans.len() && style.syn_spans[syn_idx].start <= pos {
            cell = cell.patch(super::syntax_style(&style.syn_spans[syn_idx]));
        }
        while decoration_index < style.decorations.len() && i >= decoration_end {
            decoration_index += 1;
            decoration_end += style
                .decorations
                .get(decoration_index)
                .map_or(0, |s| s.content.len());
        }
        if let Some(decoration) = style.decorations.get(decoration_index) {
            cell = cell.patch(decoration.style);
        }
        // intra-line emphasis overrides the row tint (delta two-tier)
        if let (Some(line), Some((a, b))) = (style.diff_line, style.emphasis) {
            if a < i + grapheme.len() && i < b {
                let bg = match line.origin {
                    strop_git::LineOrigin::Addition => diff::ADD_STRONG_BG,
                    _ => diff::DEL_STRONG_BG,
                };
                cell = cell.bg(bg).add_modifier(Modifier::BOLD);
            }
        }
        if is_source {
            if let Some((_, _, sev)) = style
                .diags
                .iter()
                .find(|(a, b, _)| *a < i + grapheme.len() && i < *b)
            {
                cell = cell
                    .add_modifier(Modifier::UNDERLINED)
                    .underline_color(severity_color(*sev));
            }
            let selected = if let Some(block) = style.block {
                (block.first_line..=block.last_line).contains(&row)
                    && glyph.cell <= block.right_cell
                    && end_cell > block.left_cell
            } else {
                style
                    .selection
                    .is_some_and(|r| r.start.get() < pos + grapheme.len() && pos < r.end.get())
            };
            if selected {
                cell = cell.bg(SELECT_BG);
            }
            // search hits light up (accent bold); the match under the
            // cursor — the "current" one n/N walks — wears an underline
            let hit_index = style
                .search_hits
                .partition_point(|hit| hit.end.get() <= pos);
            if let Some(hit) = style
                .search_hits
                .get(hit_index)
                .filter(|hit| hit.start.get() < pos + grapheme.len() && pos < hit.end.get())
            {
                cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
                if hit.start.get() <= view.cursor && view.cursor < hit.end.get() {
                    cell = cell.add_modifier(Modifier::UNDERLINED);
                }
            }
            if let Some(find) = style.find {
                // leap-style: candidates bold-accent on the pending side
                let ahead = if find.backward {
                    pos < view.cursor
                } else {
                    pos > view.cursor
                };
                if row == cur_line && ahead && !grapheme.chars().all(|c| c.is_whitespace()) {
                    cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
                }
            }
            if style
                .preview
                .iter()
                .any(|r| r.start.get() < pos + grapheme.len() && pos < r.end.get())
            {
                cell = cell.fg(ACCENT).bg(PREVIEW_BG);
            }
            if style
                .flash
                .is_some_and(|r| r.start.get() < pos + grapheme.len() && pos < r.end.get())
            {
                cell = cell.bg(FLASH_BG);
            }
        }
        let symbol = if !visible.complete || grapheme == "\t" {
            // the layout owns tab width: glyph and caret can't disagree
            " ".repeat(visible.width)
        } else {
            strop_core::layout::printable_grapheme(&grapheme).to_string()
        };
        spans.push(Span::styled(symbol, cell));
        used = visible.x + visible.width;
    }
    if reached_end {
        if let Some((note, cell)) = &style.note {
            // virtual EOL annotation at the line's absolute end cell —
            // tabs in the note continue the line's stops, so it cannot
            // drift when the origin scrolls
            for (glyph, grapheme) in RopeGraphemes::new_at(note.as_str().into(), tab, end_cell) {
                if glyph.cell.get() >= right {
                    break;
                }
                let Some(visible) = clip(glyph, view.hscroll, width) else {
                    continue;
                };
                if visible.x > used {
                    spans.push(Span::raw(" ".repeat(visible.x - used)));
                }
                let symbol = if visible.complete && grapheme != "\t" {
                    strop_core::layout::printable_grapheme(&grapheme).to_string()
                } else {
                    " ".repeat(visible.width)
                };
                spans.push(Span::styled(symbol, *cell));
                used = visible.x + visible.width;
            }
        }
    }
    // full-row backgrounds for add/del/band rows run past the text
    // (0010 §4), measured in CELLS, not graphemes
    if let Some(bg) = style.row_bg {
        if used < width {
            spans.push(Span::styled(
                " ".repeat(width - used),
                Style::default().fg(bg).bg(bg),
            ));
        }
    }
    spans
}

/// Fixed margins never inherit `Pane.hscroll`. Fit printable display
/// cells to the assigned width so a wide author name or path cannot
/// change the content/caret inset.
fn fixed_spans(input: Vec<Span<'static>>, width: usize, tab: usize) -> Vec<Span<'static>> {
    let text = input.iter().map(|s| s.content.as_ref()).collect::<String>();
    let mut output = Vec::new();
    let mut index = 0usize;
    let mut end = input.first().map_or(0, |s| s.content.len());
    let mut used = 0usize;
    for (glyph, grapheme) in RopeGraphemes::new(text.as_str().into(), tab) {
        if glyph.cell.get() >= width {
            break;
        }
        let Some(visible) = clip(glyph, DisplayColumn::new(0), width) else {
            continue;
        };
        while index < input.len() && glyph.byte >= end {
            index += 1;
            end += input.get(index).map_or(0, |s| s.content.len());
        }
        let style = input.get(index).map_or_else(Style::default, |s| s.style);
        let symbol = if visible.complete && grapheme != "\t" {
            strop_core::layout::printable_grapheme(&grapheme).to_string()
        } else {
            " ".repeat(visible.width)
        };
        output.push(Span::styled(symbol, style));
        used = visible.x + visible.width;
    }
    if used < width {
        output.push(Span::raw(" ".repeat(width - used)));
    }
    output
}

#[cfg(test)]
#[path = "buffer_tests.rs"]
mod tests;
