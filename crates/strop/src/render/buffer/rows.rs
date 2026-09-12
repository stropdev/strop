//! Typed source rows, gutters, source syntax projection and pane overlays.
use super::*;

/// Render one pane's rows: gutter, syntax/decoration, overlays, guides.
/// Ordinary file and diff rows stream graphemes straight off the rope —
/// no whole-line String, no layout vector; only decorated surfaces
/// (help/log/stats) keep their existing owned styled text.
pub(super) fn render_pane(editor: &mut Editor, frame: &mut Frame, area: Rect, view: &PaneView) {
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
    // Aggregate each visible source window once. Per-row requests used to
    // supersede one another, leaving mixed-language collections uncolored.
    let collection_rows: Vec<Option<crate::editor::CollectionRowInfo>> = (0..rows)
        .map(|row| editor.collection_row_info(view.doc, view.view_top.saturating_add(row)))
        .collect();
    let mut windows: Vec<(DocumentId, usize, usize)> = Vec::new();
    for info in collection_rows.iter().flatten() {
        if let Some((source, start, end)) = info.source {
            if let Some((_, first, last)) = windows.iter_mut().find(|(doc, ..)| *doc == source) {
                *first = (*first).min(start);
                *last = (*last).max(end);
            } else {
                windows.push((source, start, end));
            }
        }
    }
    let source_analyses: Vec<_> = windows
        .into_iter()
        .map(|(source, first, last)| {
            (
                source,
                editor.document_analysis(
                    source,
                    first,
                    last,
                    view.hscroll.get(),
                    area.width as usize,
                ),
            )
        })
        .collect();
    // Translate into view coordinates ONCE per frame (borrowed by the
    // row loop, which must not re-admit jobs).
    let view_buf = &editor.doc(view.doc).buf;
    let translated_spans: Vec<Option<Vec<strop_syntax::Span>>> = collection_rows
        .iter()
        .enumerate()
        .map(|(row, info)| {
            let info = info.as_ref()?;
            let (source, src_start, src_end) = info.source?;
            let analysis = source_analyses
                .iter()
                .find(|(doc, _)| *doc == source)?
                .1
                .as_ref()?;
            let row_start = view_buf.line_start(view.view_top.saturating_add(row));
            Some(
                analysis
                    .spans
                    .iter()
                    .filter(|span| span.start < src_end && span.end > src_start)
                    .map(|span| strop_syntax::Span {
                        start: span.start.max(src_start) - src_start + row_start,
                        end: span.end.min(src_end) - src_start + row_start,
                        ..*span
                    })
                    .collect(),
            )
        })
        .collect();
    let translated_hits: Vec<Vec<strop_grammar::SearchMatch>> = collection_rows
        .iter()
        .enumerate()
        .map(|(row, info)| {
            let Some(info) = info else {
                return Vec::new();
            };
            let Some((_, src_start, _)) = info.source else {
                return Vec::new();
            };
            let row_start = view_buf.line_start(view.view_top.saturating_add(row));
            info.source_matches
                .iter()
                .map(|(m, len)| strop_grammar::SearchMatch {
                    start: strop_core::id::ByteOffset::new(row_start + (m - src_start)),
                    end: strop_core::id::ByteOffset::new(row_start + (m - src_start) + len),
                })
                .collect()
        })
        .collect();
    // matching delimiters (0051 §7 R09): the engine owns pairing
    // (source-aware, cancellable, collection-projected); this frame
    // only paints the endpoints that map into view bytes. Passive —
    // never scrolls, never blocks, active pane only. Computed before
    // the rope borrow below: a miss submits worker work (&mut editor)
    let [pair_first, pair_second] = if view.overlays {
        editor.pair_highlight(
            view.doc,
            view.cursor,
            matches!(editor.mode, crate::editor::Mode::Insert),
        )
    } else {
        [None, None]
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
        // occurrence selections (0049 §7): every stretched extra paints
        // its range like the primary's visual selection
        extra_selections: if view.overlays
            && view.doc == editor.current()
            && matches!(editor.mode, crate::editor::Mode::Visual)
        {
            editor
                .extra_selections()
                .iter()
                .filter(|s| !s.collapsed())
                .map(|s| {
                    let (start, last) = s.range();
                    let end = editor
                        .buf()
                        .ceil_boundary((last + 1).min(editor.buf().len_bytes()));
                    strop_core::Range::charwise(start, end.min(editor.buf().len_bytes()))
                })
                .collect()
        } else {
            Vec::new()
        },
        block: view.overlays.then(|| editor.block_rect_pub()).flatten(),
        search_hits,
        pair_first,
        pair_second,
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
    // 0051 R08: the fixed margins measure with the viewed document's
    // resolved width too — one setting, no render-side config shortcut.
    let margin_tab = editor.doc(view.doc).indent.width.max(1);
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
                diff::sidebar_row_spans(tree, files, current, line_idx, sidebar_focused),
                sidebar_w,
                margin_tab,
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
            left.extend(fixed_spans(vec![span], diff::BLAME_W, margin_tab));
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
        style.syn_spans = syn_spans;
        style.search_hits = search_hits;
        // Collection body rows: source-projected syntax + query hits
        if let Some(Some(_)) = collection_rows.get(row) {
            if let Some(spans) = translated_spans.get(row).and_then(Option::as_deref) {
                style.syn_spans = spans;
            }
            if let Some(hits) = translated_hits.get(row) {
                if !hits.is_empty() && search_hits.is_empty() {
                    style.search_hits = hits;
                }
            }
        }
        style.diags = if view.overlays {
            editor.diag_ranges_at(view.doc, line_idx + 1)
        } else {
            Vec::new()
        };
        match editor.review_row(view.doc, line_idx) {
            Some(crate::editor::ReviewRow::Heading | crate::editor::ReviewRow::File) => {
                style.row_fg = Some(ACCENT)
            }
            Some(crate::editor::ReviewRow::Hunk) => {
                style.row_fg = Some(MUTED);
                style.row_bg = Some(diff::BAND_BG);
            }
            Some(crate::editor::ReviewRow::Removed) => {
                style.row_fg = Some(diff::DEL_FG);
                style.row_bg = diff::origin_bg(strop_git::LineOrigin::Deletion);
            }
            Some(crate::editor::ReviewRow::Added) => {
                style.row_fg = Some(diff::ADD_FG);
                style.row_bg = diff::origin_bg(strop_git::LineOrigin::Addition);
            }
            Some(crate::editor::ReviewRow::Warning) => {
                style.row_fg = Some(severity_color(strop_lsp::Severity::Warning))
            }
            _ => {}
        }
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
                // Collection views gutter the SOURCE line number; chrome
                // rows (title/headers) stay blank (0049 §6).
                let number_cell = match editor.collection_source_lineno(view.doc, line_idx) {
                    Some(Some(n)) => format!("{:>digits$} ", n, digits = number_width - 2),
                    Some(None) => " ".repeat(number_width - 1),
                    None => format!("{:>digits$} ", line_idx + 1, digits = number_width - 2),
                };
                left.push(Span::styled(number_cell, num_style));
                // Collection cards (0049 §6): the title reads strong,
                // card borders mute, the gap rows whisper, and the top
                // border's path carries the accent.
                match editor.collection_row_kind(view.doc, line_idx) {
                    Some(crate::editor::CollectionRow::Title) => style.row_fg = Some(TEXT),
                    Some(crate::editor::CollectionRow::CardTop(_)) => {
                        // 0049 §6: focus reads through the active card's
                        // border + path, not a selection-colored chip
                        let focused = collection_rows
                            .get(row)
                            .and_then(Option::as_ref)
                            .is_some_and(|info| info.card_active);
                        style.row_fg = Some(if focused { TEXT } else { MUTED });
                        let mut text = text.to_string();
                        if let Some(info) = collection_rows.get(row).and_then(Option::as_ref) {
                            if info.source_dirty {
                                text.push_str(" · modified");
                            }
                            if info.source_readonly {
                                text.push_str(" · read-only");
                            }
                        }
                        let pad = width.saturating_sub(crate::render::text::width(&text) + 2);
                        let text = format!("{}─{}╮", text, "─".repeat(pad));
                        style.decorations = collection_card_top_spans(&text);
                    }
                    Some(crate::editor::CollectionRow::Gap) => {
                        style.row_fg = Some(MUTED);
                    }
                    Some(crate::editor::CollectionRow::CardBottom) => {
                        style.row_fg = Some(MUTED);
                        let pad = width.saturating_sub(2);
                        style.decorations = vec![Span::styled(
                            format!("╰{}╯", "─".repeat(pad)),
                            Style::default().fg(MUTED),
                        )];
                    }
                    _ => {}
                }
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
                    style.decorations =
                        crate::render::help::row_spans(&text, &help_section, width as u16);
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
        let chrome = matches!(editor.collection_row_kind(view.doc, line_idx),
            Some(kind) if kind != crate::editor::CollectionRow::Body);
        let fixed_view = PaneView {
            hscroll: DisplayColumn::new(0),
            ..*view
        };
        left.extend(content_spans(
            editor,
            if chrome { &fixed_view } else { view },
            start,
            text,
            &style,
            width,
        ));
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
    if width == crate::render::buffer::GUTTER as usize {
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

/// A card top border's spans: the path in accent, the border and badges
/// muted (0049 §6).
fn collection_card_top_spans(text: &str) -> Vec<Span<'static>> {
    let border = Style::default().fg(MUTED);
    match (text.find("╭─ "), text.find(" ──")) {
        (Some(lo), Some(hi)) => {
            let path_start = lo + "╭─ ".len();
            vec![
                Span::styled(text[..path_start].to_string(), border),
                Span::styled(
                    text[path_start..hi].to_string(),
                    Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
                ),
                Span::styled(text[hi..].to_string(), border),
            ]
        }
        _ => vec![Span::styled(text.to_string(), border)],
    }
}
