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

/// Invariant guard: every pane row is written out to the pane's full
/// width. ratatui's Paragraph clears the cells its lines don't touch
/// today, but a renderer relying on that is one widget swap away from
/// resurrecting two-frames-old glyphs (the double-buffer keeps frame
/// N-2's cells) — pad explicitly instead.
fn pad_row(mut line: Line<'static>, width: u16) -> Line<'static> {
    let used = line.width() as u16;
    if used < width {
        line.spans
            .push(Span::raw(" ".repeat((width - used) as usize)));
    }
    line
}

/// One pane's view of a buffer. `overlays` is false for inactive panes:
/// preview/search/selection/flash belong to the pane being driven.
struct PaneView {
    doc: strop_core::id::DocumentId,
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
                doc: editor.current(),
                cursor: editor.head(),
                view_top: editor.view_top(),
                overlays: true,
            }
        } else {
            let pane = &editor.panes[i];
            PaneView {
                doc: if editor.docs.get(pane.doc).is_some() {
                    pane.doc
                } else {
                    editor.current()
                },
                cursor: pane.sels.primary().head,
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
    let buf = &editor.doc(view.doc).buf;
    let line = buf.line_of(view.cursor);
    let row = line.saturating_sub(view.view_top) as u16;
    let gutter = diff::left_inset(editor, view.doc) as u16;
    let col = gutter + buf.cell_col_of(view.cursor);
    if row < area.height && col < area.width {
        let cell = &mut frame.buffer_mut()[(area.x + col, area.y + row)];
        cell.set_bg(Color::Rgb(0x3a, 0x3d, 0x4d));
    }
}

/// Secondary cursors (0013 §4): solid blocks on the active pane, like
/// the native block cursor but painted.
fn render_extra_cursors(editor: &Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
    if view.doc != editor.current() || editor.extra_selections().is_empty() {
        return;
    }
    let buf = &editor.doc(view.doc).buf;
    let inset = diff::left_inset(editor, view.doc) as u16;
    for c in editor.extra_selections().iter().map(|s| s.head) {
        let line = buf.line_of(c);
        if line < view.view_top {
            continue;
        }
        let row = (line - view.view_top) as u16;
        let col = inset + buf.cell_col_of(c);
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
    // tree-sitter takes the mutable borrow first; everything below it
    // reads immutably (one borrow discipline per pane render)
    let (cur_line, first_byte, last_byte, rope) = {
        let buf = &editor.doc(view.doc).buf;
        let last_line = (view.view_top + text_rows).min(buf.len_lines());
        (
            buf.line_of(view.cursor),
            buf.line_start(view.view_top),
            buf.line_end(last_line.saturating_sub(1)),
            buf.rope.clone(),
        )
    };
    let syn_spans: Vec<strop_syntax::Span> =
        match editor.docs.get_mut(view.doc).map(|d| &mut d.highlighter) {
            Some(h) => h
                .as_mut()
                .map(|h| h.highlight(&rope, first_byte, last_byte))
                .unwrap_or_default(),
            None => Vec::new(),
        };
    let buf = &editor.doc(view.doc).buf;
    let surface = editor.doc(view.doc).surface.as_ref();

    // overlays read live editor state; only the active pane shows them
    let block = if view.overlays {
        editor.block_rect_pub()
    } else {
        None
    };
    let mut row_style = RowStyle {
        syn_spans: &syn_spans,
        preview: if view.overlays {
            editor
                .preview()
                .map(|(ranges, _)| ranges)
                .unwrap_or_default()
        } else {
            Vec::new()
        },
        flash: view.overlays.then(|| editor.flash_range()).flatten(),
        selection: view.overlays.then(|| editor.visual_range()).flatten(),
        block,
        search_hits: &[],
        find: view.overlays.then(|| editor.find_candidates()).flatten(),
        diags: Vec::new(),
        emphasis: None,
        diff_line: None,
    };
    // highlight the pending search (incsearch) or, persistently, the
    // last committed search (rootle rule: matches stay lit; the current
    // one is underlined by content_spans)
    let search_hits: Vec<usize> = if view.overlays {
        if let Some(p) = editor.search_pattern() {
            strop_grammar::search_all(editor.buf(), p)
        } else if let Some(ls) = &editor.last_search {
            let mut hits = strop_grammar::search_all(editor.buf(), &ls.pattern);
            if ls.whole_word {
                let word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
                let len = ls.pattern.len();
                let buf = editor.buf();
                hits.retain(|&h| {
                    let before_ok = h == 0 || !word_char(buf.byte(h - 1));
                    let after_ok = h + len >= buf.len_bytes() || !word_char(buf.byte(h + len));
                    before_ok && after_ok
                });
            }
            hits
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    row_style.search_hits = &search_hits;

    let mut lines: Vec<Line> = Vec::with_capacity(text_rows);
    let diff_digits = diff_digits(surface);
    // 0011 left-margin columns: the commit file sidebar (Diff surfaces
    // from the dive chain) and the blame gutter (file buffers) prepend
    // to every row; content width shrinks by what they take
    let (sidebar, sidebar_focused) = match surface {
        Some(crate::editor::Surface::Diff {
            commit: Some(cf),
            label,
            sidebar_focus,
            ..
        }) => (Some((cf.files.as_slice(), label.as_str())), *sidebar_focus),
        _ => (None, false),
    };
    let sidebar_w = sidebar.map_or(0, |(files, _)| diff::sidebar_width(files) + 1);
    let blame = editor.blame_gutter_for(view.doc);
    let blame_w = if blame.is_some() { diff::BLAME_W } else { 0 };
    let content_width = area.width.saturating_sub((sidebar_w + blame_w) as u16);
    // :help rows color by the section they sit under (render/help.rs)
    let mut help_section = String::new();
    for row in 0..text_rows {
        let line_idx = view.view_top + row;
        // the margin columns: sidebar cell (or blank), then the blame
        // cell (or blank past the buffer's lines)
        let mut left: Vec<Span> = sidebar
            .map(|(files, label)| diff::sidebar_spans(files, label, line_idx, sidebar_focused))
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
            lines.push(pad_row(Line::from(left), area.width));
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
                lines.push(pad_row(line, area.width));
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
                lines.push(pad_row(Line::from(left), area.width));
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
            editor.diag_ranges_at(view.doc, line_idx + 1)
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
            if let Some((sev, msg)) = editor.diag_message_at(view.doc, line_idx + 1) {
                let shown: String = msg.replace('\n', " · ").chars().take(80).collect();
                left.push(Span::styled(
                    format!("  ▍ {shown}"),
                    Style::default()
                        .fg(dim_color(severity_color(sev)))
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }
        lines.push(pad_row(Line::from(left), area.width));
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
    /// ctrl-v rectangle: (first line, last line, left cell, right cell)
    /// — per-row byte ranges derive through LineLayout (0017).
    block: Option<(usize, usize, u16, u16)>,
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
    let buf = &editor.doc(view.doc).buf;
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
    // pending search or the last committed one (persistent highlight)
    let pat_len = editor
        .search_pattern()
        .map(str::len)
        .or_else(|| editor.last_search.as_ref().map(|ls| ls.pattern.len()))
        .unwrap_or(0);
    let mut spans = Vec::with_capacity(text.len() / 2 + 4);
    let mut chars = 0usize;
    // 0017: walk GRAPHEMES with byte offsets — char indices drifted
    // every overlay after the first multibyte char
    let trimmed = text.strip_suffix('\n').unwrap_or(text);
    for (i, ch) in unicode_segmentation::UnicodeSegmentation::grapheme_indices(trimmed, true) {
        let pos = start + i;
        while syn_idx < syn_spans.len() && syn_spans[syn_idx].end <= pos {
            syn_idx += 1;
        }
        let mut cell = Style::default().fg(TEXT);
        if let Some(dl) = style.diff_line {
            cell = cell.fg(diff::origin_fg(dl.origin));
            // syntax colors ride under the origin tint (delta's look)
            if syn_idx < syn_spans.len() && syn_spans[syn_idx].start <= pos {
                cell = cell.fg(class_color(syn_spans[syn_idx].class));
            }
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
        let selected = if let Some((la, lh, cl, cr)) = style.block {
            let line_idx = buf.line_of(pos);
            (la..=lh).contains(&line_idx) && {
                let layout = strop_core::layout::LineLayout::build(trimmed, 8);
                let cell = layout.cell_at_byte(pos - start);
                cl <= cell && cell <= cr
            }
        } else {
            style.selection.is_some_and(|r| in_range(r, pos))
        };
        if selected {
            cell = cell.bg(SELECT_BG);
        }
        // search hits light up (accent bold); the match under the
        // cursor — the "current" one n/N walks — wears an underline
        if pat_len > 0 {
            if let Some(h) = style
                .search_hits
                .iter()
                .find(|&&h| pos >= h && pos < h + pat_len)
            {
                cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
                let is_current = view.cursor >= *h && view.cursor < h + pat_len;
                if is_current {
                    cell = cell.add_modifier(Modifier::UNDERLINED);
                }
            }
        }
        if let Some((_, backward)) = style.find {
            // leap-style: candidates bold-accent on the pending side
            let on_line = buf.line_of(pos) == cur_line;
            let ahead = if backward {
                pos < view.cursor
            } else {
                pos > view.cursor
            };
            if on_line && ahead && !ch.chars().all(|c| c.is_whitespace()) {
                cell = cell.fg(ACCENT).add_modifier(Modifier::BOLD);
            }
        }
        if style.preview.iter().any(|&r| in_range(r, pos)) {
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
            let used = diff::gutter_width(editor.doc(view.doc).surface.as_ref()) + chars;
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

    #[test]
    fn switching_buffers_leaves_no_stale_cells() {
        // invariant: every pane row is written full-width, so ratatui's
        // double-buffer can never resurrect two-frames-old glyphs on a
        // buffer switch (the user-reported "lingering >" symptom class)
        let dir = tempfile::tempdir().unwrap();
        let wide = dir.path().join("wide.txt");
        let narrow = dir.path().join("narrow.txt");
        let junk = format!("{}\n", ">".repeat(60)).repeat(30);
        std::fs::write(&wide, &junk).unwrap();
        std::fs::write(&narrow, "hi\n").unwrap();
        let mut e = Editor::new(Buffer::from_text(""));
        e.open_buffer(wide.to_str().unwrap()).unwrap();
        e.open_buffer(narrow.to_str().unwrap()).unwrap();
        // same terminal, two frames: ratatui TestBackend diffing is the
        // real path, so drive both frames through one terminal
        let backend = ratatui::backend::TestBackend::new(40, 12);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        let draw =
            |e: &mut Editor, terminal: &mut ratatui::Terminal<ratatui::backend::TestBackend>| {
                terminal.draw(|f| crate::render::render(e, f)).unwrap();
                let buf = terminal.backend().buffer();
                (0..12)
                    .map(|y| {
                        (0..40)
                            .map(|x| buf[(x, y)].symbol().to_string())
                            .collect::<String>()
                    })
                    .collect::<Vec<_>>()
            };
        let wide_id = e
            .mru
            .iter()
            .copied()
            .find(|&id| {
                e.doc(id)
                    .buf
                    .path
                    .as_deref()
                    .is_some_and(|p| p.ends_with("wide.txt"))
            })
            .unwrap();
        e.view_mut().doc = wide_id;
        let wide_frame = draw(&mut e, &mut terminal);
        assert!(
            wide_frame[0].contains(">>>"),
            "wide buffer rendered: {}",
            wide_frame[0]
        );
        let narrow_id = e
            .mru
            .iter()
            .copied()
            .find(|&id| {
                e.doc(id)
                    .buf
                    .path
                    .as_deref()
                    .is_some_and(|p| p.ends_with("narrow.txt"))
            })
            .unwrap();
        e.view_mut().doc = narrow_id; // narrow
                                      // ratatui double-buffers: stale cells surface one swap later,
                                      // on the SECOND narrow frame
        let _ = draw(&mut e, &mut terminal);
        let narrow_frame = draw(&mut e, &mut terminal);
        let leftover = narrow_frame.iter().filter(|row| row.contains('>')).count();
        assert_eq!(leftover, 0, "stale cells: {}", narrow_frame.join("\n"));
    }
    #[test]
    fn block_mode_highlights_the_rectangle() {
        // ctrl-v lj selects cells 0-1 on rows 0-1 — the SELECT_BG must
        // land on exactly those cells (0017: the rect, not the bytes)
        let mut e = Editor::new(Buffer::from_text("aabb\nccdd\n"));
        e.feed_text("<c-v>lj");
        let backend = ratatui::backend::TestBackend::new(30, 6);
        let mut terminal = ratatui::Terminal::new(backend).unwrap();
        terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
        let buf = terminal.backend().buffer();
        let bg = |x: u16, y: u16| buf[(x, y)].bg;
        // text starts after the 5-cell gutter ("▎  1 ")
        let sel = crate::render::SELECT_BG;
        assert_eq!(bg(5, 0), sel, "block corner");
        assert_eq!(bg(6, 0), sel, "block col 2 row 0");
        assert_eq!(bg(5, 1), sel, "block row 1");
        assert_ne!(bg(7, 0), sel, "outside the rectangle");
        assert_ne!(bg(5, 2), sel, "past the rectangle's last row");
    }

    #[test]
    fn cursor_cell_tracks_wide_chars() {
        // 0017: l through a wide char lands on the next char, and the
        // caret's display CELL tracks layout, not byte columns
        let mut e = Editor::new(Buffer::from_text("a界b\n"));
        e.feed_text("l"); // onto 界
        assert_eq!(e.buf().cell_col_of(e.head()), 1); // 界 starts at cell 1
        e.feed_text("l"); // onto b (byte 4)
        assert_eq!(e.buf().cell_col_of(e.head()), 3); // b at cell 3
    }
}
