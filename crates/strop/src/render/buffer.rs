//! Pane rendering — one text renderer for every pane (0010 §3).
//! Active panes read live editor state with overlays; inactive panes
//! read their saved snapshot without. Same gutter, same guides, same
//! diff rows — the duplicated inactive-pane loop is gone, so panes
//! cannot drift apart again.

use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::{Editor, LayoutDir};

use super::diff;
use super::{class_color, dim_color, in_range, severity_color};
use super::{ACCENT, BASE, FLASH_BG, MUTED, PREVIEW_BG, SELECT_BG, TEXT};

/// Width of the standard gutter: sign column + 3-digit number + space.
pub(crate) const GUTTER: u16 = 5;

/// One pane's view of a buffer. `overlays` is false for inactive panes:
/// preview/search/selection/flash belong to the pane being driven.
struct PaneView {
    buffer: usize,
    cursor: usize,
    view_top: usize,
    overlays: bool,
}

/// Render all panes and return the active pane's rect (the native
/// cursor lives there — offsets included, which the full-area version
/// got wrong in splits).
pub(crate) fn render_panes(editor: &mut Editor, frame: &mut Frame, area: Rect) -> Rect {
    let n = editor.panes.len();
    let is_row = editor.layout == LayoutDir::Row;
    let total_w = area.width as usize;
    let total_h = area.height as usize - 1; // statusline
    let dividers = n - 1;
    let (mut x, mut y) = (area.x, area.y);
    let mut active_rect = Rect {
        x,
        y,
        width: total_w as u16,
        height: total_h as u16,
    };
    for i in 0..n {
        let (w, h): (u16, u16) = if is_row {
            let w = ((total_w - dividers) / n) as u16;
            let w = if i == n - 1 {
                (total_w - dividers) as u16 - w * (n as u16 - 1)
            } else {
                w
            };
            (w, total_h as u16)
        } else {
            let h = ((total_h - dividers) / n) as u16;
            let h = if i == n - 1 {
                (total_h - dividers) as u16 - h * (n as u16 - 1)
            } else {
                h
            };
            (total_w as u16, h)
        };
        let rect = Rect {
            x,
            y,
            width: w,
            height: h,
        };
        let view = if i == editor.active_pane {
            PaneView {
                buffer: editor.current,
                cursor: editor.cursor,
                view_top: editor.view_top,
                overlays: true,
            }
        } else {
            let pane = &editor.panes[i];
            PaneView {
                buffer: pane.buffer.min(editor.buffers.len().saturating_sub(1)),
                cursor: pane.cursor,
                view_top: pane.view_top,
                overlays: false,
            }
        };
        render_pane(editor, frame, rect, &view);
        if i == editor.active_pane {
            active_rect = rect;
            render_extra_cursors(editor, frame, rect, &view);
        } else {
            render_static_caret(editor, frame, rect, &view);
        }
        if i < n - 1 {
            // divider column/row
            if is_row {
                let dx = x + w;
                for dy in y..y + h {
                    let cell = &mut frame.buffer_mut()[(dx, dy)];
                    cell.set_symbol("│");
                    cell.set_fg(Color::Rgb(0x3a, 0x3d, 0x4d));
                }
                x = dx + 1;
            } else {
                let dy = y + h;
                for dx in x..x + w {
                    let cell = &mut frame.buffer_mut()[(dx, dy)];
                    cell.set_symbol("─");
                    cell.set_fg(Color::Rgb(0x3a, 0x3d, 0x4d));
                }
                y = dy + 1;
            }
        }
    }
    active_rect
}

/// The inactive pane's position, unfocused: a muted block on the saved
/// cursor cell, offsets pane-local (unlike the native cursor).
fn render_static_caret(editor: &Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    let buf = &editor.buffers[view.buffer];
    let line = buf.line_of(view.cursor);
    let row = line.saturating_sub(view.view_top) as u16;
    let gutter = diff::left_inset(editor, view.buffer) as u16;
    let col = gutter + buf.col_of(view.cursor) as u16;
    if row < area.height && col < area.width {
        let cell = &mut frame.buffer_mut()[(area.x + col, area.y + row)];
        cell.set_bg(Color::Rgb(0x3a, 0x3d, 0x4d));
    }
}

/// Secondary cursors (0013 §4): solid blocks on the active pane, like
/// the native block cursor but painted.
fn render_extra_cursors(editor: &Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    if view.buffer != editor.current || editor.extra_cursors.is_empty() {
        return;
    }
    let buf = &editor.buffers[view.buffer];
    let inset = diff::left_inset(editor, view.buffer) as u16;
    for &c in &editor.extra_cursors {
        let line = buf.line_of(c);
        if line < view.view_top {
            continue;
        }
        let row = (line - view.view_top) as u16;
        let col = inset + buf.col_of(c) as u16;
        if row < area.height && col < area.width {
            let cell = &mut frame.buffer_mut()[(area.x + col, area.y + row)];
            cell.set_bg(TEXT);
            cell.set_fg(BASE);
        }
    }
}

/// Render one pane's rows: gutter, syntax/decoration, overlays, guides.
fn render_pane(editor: &mut Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    let text_rows = area.height as usize;
    let buf = &editor.buffers[view.buffer];
    let cur_line = buf.line_of(view.cursor);
    let surface = editor.surfaces.get(view.buffer).and_then(|s| s.as_ref());

    // tree-sitter spans for the visible window (base layer, 0001 §5.8)
    let first_byte = buf.line_start(view.view_top);
    let last_line = (view.view_top + text_rows).min(buf.len_lines());
    let last_byte = buf.line_end(last_line.saturating_sub(1));
    let rope = editor.buffers[view.buffer].rope.clone();
    let syn_spans: Vec<strop_syntax::Span> = match editor.highlighters.get_mut(view.buffer) {
        Some(h) => h
            .as_mut()
            .map(|h| h.highlight(&rope, first_byte, last_byte))
            .unwrap_or_default(),
        None => Vec::new(),
    };

    // overlays read live editor state; only the active pane shows them
    let mut row_style = RowStyle {
        syn_spans: &syn_spans,
        preview: view
            .overlays
            .then(|| editor.preview())
            .flatten()
            .map(|r| r.range),
        flash: view.overlays.then(|| editor.flash_range()).flatten(),
        selection: view.overlays.then(|| editor.visual_range()).flatten(),
        search_hits: &[],
        find: view.overlays.then(|| editor.find_candidates()).flatten(),
        diags: Vec::new(),
        emphasis: None,
        diff_line: None,
    };
    let search_hits: Vec<usize> = if view.overlays {
        editor
            .search_pattern()
            .map(|p| strop_grammar::search_all(editor.buf(), p))
            .unwrap_or_default()
    } else {
        Vec::new()
    };
    row_style.search_hits = &search_hits;

    let mut lines: Vec<Line> = Vec::with_capacity(text_rows);
    let diff_digits = diff_digits(surface);
    // 0011 left-margin columns: the commit file sidebar (Diff surfaces
    // from the dive chain) and the blame gutter (file buffers) prepend
    // to every row; content width shrinks by what they take
    let sidebar = match surface {
        Some(crate::editor::Surface::Diff {
            commit: Some(cf),
            label,
            ..
        }) => Some((cf.files.as_slice(), label.as_str())),
        _ => None,
    };
    let sidebar_w = sidebar.map_or(0, |(files, _)| diff::sidebar_width(files) + 1);
    let blame = editor.blame_gutter_for(view.buffer);
    let blame_w = if blame.is_some() { diff::BLAME_W } else { 0 };
    let content_width = area.width.saturating_sub((sidebar_w + blame_w) as u16);
    // :help rows color by the section they sit under (render/help.rs)
    let mut help_section = String::new();
    for row in 0..text_rows {
        let line_idx = view.view_top + row;
        // the margin columns: sidebar cell (or blank), then the blame
        // cell (or blank past the buffer's lines)
        let mut left: Vec<Span> = sidebar
            .map(|(files, label)| diff::sidebar_spans(files, label, line_idx))
            .unwrap_or_default();
        if let Some(gutter) = blame {
            left.push(match gutter.lines.get(line_idx) {
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
                        diff::blame_spans(bl)
                    }
                }
                None => diff::blame_blank(),
            });
        }
        if line_idx > buf.last_content_line() {
            left.push(Span::styled("~", Style::default().fg(MUTED)));
            lines.push(Line::from(left));
            continue;
        }
        let start = buf.line_start(line_idx);
        let text = buf.line_text(line_idx);

        // git memory surfaces decorate their rows from typed data
        // (0010 §4/§5): diff rows re-gutter, log/files rows re-color
        match diff::diff_row(surface, line_idx) {
            Some(diff::DiffRow::Stats | diff::DiffRow::HunkHeader) => {
                let mut line = diff::structural_row(surface.unwrap(), line_idx, content_width);
                line.spans.splice(0..0, left);
                lines.push(line);
                continue;
            }
            Some(diff::DiffRow::Line(dl)) => {
                let mut spans = diff::diff_gutter(dl, line_idx == cur_line, diff_digits);
                row_style.diff_line = Some(dl);
                row_style.emphasis = diff::emphasis_span(surface, line_idx);
                spans.extend(content_spans(
                    editor,
                    view,
                    start,
                    &text,
                    &row_style,
                    content_width,
                ));
                left.extend(spans);
                lines.push(Line::from(left));
                continue;
            }
            None => {}
        }

        let num_style = if line_idx == cur_line {
            Style::default().fg(ACCENT)
        } else {
            Style::default().fg(MUTED)
        };
        // Helix-grade gutter: a colored ▎ bar in the leftmost column —
        // diagnostics first, then git signs (green/amber/red)
        let (bar, bar_color) = gutter_mark(editor, view, line_idx);
        left.push(Span::styled(
            bar,
            Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
        ));
        left.push(Span::styled(format!("{:>3} ", line_idx + 1), num_style));
        row_style.diff_line = None;
        row_style.emphasis = None;
        row_style.diags = if view.overlays {
            editor.diag_ranges_at(view.buffer, line_idx + 1)
        } else {
            Vec::new()
        };
        if let Some(content) = diff::surface_content_spans(surface, line_idx, content_width) {
            left.extend(content);
        } else if buf.name.as_deref() == Some("help") {
            // the :help buffer gets house-style color (render/help.rs)
            if text.starts_with('[') && text.ends_with(']') {
                help_section = text.trim_matches(['[', ']']).to_string();
            }
            left.extend(super::help::row_spans(&text, &help_section, content_width));
        } else {
            left.extend(content_spans(
                editor,
                view,
                start,
                &text,
                &row_style,
                content_width,
            ));
        }
        // cursor-line end-of-line diagnostic (scoped to the one line —
        // you see what the dot means without leaving the buffer)
        if view.overlays && line_idx == cur_line {
            if let Some((sev, msg)) = editor.diag_message_at(view.buffer, line_idx + 1) {
                let shown: String = msg.replace('\n', " · ").chars().take(80).collect();
                left.push(Span::styled(
                    format!("  ▍ {shown}"),
                    Style::default()
                        .fg(dim_color(severity_color(sev)))
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }
        lines.push(Line::from(left));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(BASE)),
        Rect {
            height: text_rows as u16,
            ..area
        },
    );
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
    if let Some(sev) = editor.diag_severity_at(view.buffer, line_idx + 1) {
        // severity dot (VSCode/gitui lesson: color reads faster than
        // letters) — the cursor line's EOL note carries the words
        return ("●", severity_color(sev));
    }
    // git signs: + add, ~ change, - deletion below (only for the
    // working buffer — surfaces have no path, so no leak)
    if view.buffer == editor.current {
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
    preview: Option<strop_core::Range>,
    flash: Option<strop_core::Range>,
    selection: Option<strop_core::Range>,
    search_hits: &'a [usize],
    find: Option<(u8, bool)>,
    /// Diagnostic spans on this row: (col, end_col, severity) — the
    /// undercurl layer (0009 UX).
    diags: Vec<(usize, usize, u8)>,
    /// Set on diff-surface rows: typed origin drives colors (0010 §4).
    diff_line: Option<&'a strop_git::DiffLine>,
    /// Intra-line changed range on a diff row (delta-style emphasis).
    emphasis: Option<(usize, usize)>,
}

/// Content spans for one row: syntax or diff decoration, then overlays
/// composed on top (search < preview < flash, 0001 §5.8). Diff rows get
/// a full-width background pad.
fn content_spans(
    editor: &Editor,
    view: &PaneView,
    start: usize,
    text: &str,
    style: &RowStyle,
    width: u16,
) -> Vec<Span<'static>> {
    let buf = &editor.buffers[view.buffer];
    let cur_line = buf.line_of(view.cursor);
    // indent guides: dim │ at each indent level within leading
    // whitespace (spaces only, v1)
    let lead_ws = if editor.config.indent_guides {
        text.chars().take_while(|c| *c == ' ').count()
    } else {
        0
    };
    let tab = editor.config.tab_size.max(1);
    let syn_spans = style.syn_spans;
    let mut syn_idx = syn_spans.partition_point(|s| s.end <= start);
    let mut spans = Vec::with_capacity(text.len() / 2 + 4);
    let mut chars = 0usize;
    for (i, ch) in text.chars().enumerate() {
        let pos = start + i; // prototype is ASCII-honest (0001 §5.9 later)
        while syn_idx < syn_spans.len() && syn_spans[syn_idx].end <= pos {
            syn_idx += 1;
        }
        let mut cell = Style::default().fg(TEXT);
        if let Some(dl) = style.diff_line {
            cell = cell.fg(diff::origin_fg(dl.origin));
            if let Some(bg) = diff::origin_bg(dl.origin) {
                cell = cell.bg(bg);
            }
            // intra-line emphasis overrides the row tint (delta two-tier)
            if let Some((s, e)) = style.emphasis {
                if s <= i && i < e {
                    let strong = match dl.origin {
                        strop_git::LineOrigin::Addition => diff::ADD_STRONG_BG,
                        _ => diff::DEL_STRONG_BG,
                    };
                    cell = cell.bg(strong).add_modifier(Modifier::BOLD);
                }
            }
        } else if syn_idx < syn_spans.len() && syn_spans[syn_idx].start <= pos {
            let class = syn_spans[syn_idx].class;
            cell = cell.fg(class_color(class));
            if class == strop_syntax::Class::Comment {
                cell = cell.add_modifier(Modifier::ITALIC);
            }
        }
        if let Some((_, _, sev)) = style.diags.iter().find(|(c, e, _)| *c <= i && i < *e) {
            cell = cell
                .add_modifier(Modifier::UNDERLINED)
                .underline_color(severity_color(*sev));
        }
        if style.selection.is_some_and(|r| in_range(r, pos)) {
            cell = cell.bg(SELECT_BG);
        }
        if style
            .search_hits
            .iter()
            .any(|&h| pos >= h && pos < h + editor.search_pattern().map_or(0, str::len))
        {
            cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
        }
        if let Some((_, backward)) = style.find {
            // leap-style: candidates bold-accent on the pending side
            let on_line = buf.line_of(pos) == cur_line;
            let ahead = if backward {
                pos < view.cursor
            } else {
                pos > view.cursor
            };
            if on_line && ahead && !ch.is_whitespace() {
                cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
            }
        }
        if style.preview.is_some_and(|r| in_range(r, pos)) {
            cell = cell.fg(ACCENT).bg(PREVIEW_BG);
        }
        if style.flash.is_some_and(|r| in_range(r, pos)) {
            cell = cell.bg(FLASH_BG);
        }
        let is_guide = i < lead_ws && (i + 1) % tab == 0;
        if is_guide {
            spans.push(Span::styled("│", cell.fg(Color::Rgb(0x2e, 0x30, 0x42))));
        } else {
            spans.push(Span::styled(ch.to_string(), cell));
        }
        chars += 1;
    }
    // full-row backgrounds for add/del rows run past the text (0010 §4)
    if let Some(dl) = style.diff_line {
        if let Some(bg) = diff::origin_bg(dl.origin) {
            let used =
                diff::gutter_width(editor.surfaces.get(view.buffer).and_then(|s| s.as_ref()))
                    + chars;
            let pad = (width as usize).saturating_sub(used);
            if pad > 0 {
                spans.push(Span::styled(
                    " ".repeat(pad),
                    Style::default().fg(bg).bg(bg),
                ));
            }
        }
    }
    spans
}

#[cfg(test)]
mod tests {
    use crate::editor::Editor;
    use strop_core::Buffer;

    #[test]
    fn cursor_line_shows_eol_diagnostic() {
        let mut e = Editor::new(Buffer::from_text("let x = 1;\n"));
        let rel = "strop-eol-diag-test.rs";
        e.buf_mut().path = Some(rel.into());
        let abs = e.cwd.join(rel);
        e.diags.insert(
            abs,
            vec![strop_lsp::Diag {
                line: 0,
                col: 4,
                severity: 1,
                end_line: 0,
                end_col: 8,
                message: "mismatched types".into(),
            }],
        );
        let frame = crate::headless::frame_string(&mut e, 60, 10);
        assert!(frame.contains("●"), "gutter sign: {frame}");
        assert!(frame.contains("▍ mismatched types"), "eol note: {frame}");
    }
}
