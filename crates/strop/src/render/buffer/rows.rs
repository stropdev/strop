//! Typed source rows, gutters, source syntax projection and pane overlays.
use super::*;

mod collection;
use collection::CollectionProjection;

/// Render one pane's rows: gutter, syntax/decoration, overlays, guides.
/// Ordinary file and diff rows stream graphemes straight off the rope —
/// no whole-line String, no layout vector; only decorated surfaces
/// (help/log/stats) keep their existing owned styled text.
pub(super) fn render_pane(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    area: Rect,
    view: &PaneView,
) {
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
    let analysis = editor.document_analysis_cached(
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
    let collection = CollectionProjection::gather(editor, view, rows, area.width as usize);
    // matching delimiters (0051 §7 R09): the engine owns pairing
    // (source-aware, cancellable, collection-projected); this frame
    // only paints the endpoints that map into view bytes. Passive —
    // never scrolls, never blocks, active pane only. Computed before
    // the rope borrow below: a miss submits worker work (&mut editor)
    let [pair_first, pair_second] = if view.overlays {
        editor.pair_highlight_cached(
            view.doc,
            view.cursor,
            matches!(editor.mode(), crate::editor::Mode::Insert),
        )
    } else {
        [None, None]
    };
    let buf = &editor.doc(view.doc).buf;
    let surface = editor.doc(view.doc).surface_payload();
    let directory = editor
        .doc(view.doc)
        .directory_metadata_ref()
        .filter(|source| source.draft.is_none());

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
        flash: view
            .overlays
            .then(|| editor.confirm_range().or_else(|| editor.flash_range()))
            .flatten(),
        selection: if view.overlays {
            // 0069 D05: in a snippet session the active placeholder reads
            // as the selection (insert mode has none otherwise)
            if editor.mode() == crate::editor::Mode::Insert && editor.snippet_active() {
                editor
                    .snippet_selection_ranges()
                    .0
                    .map(|(start, end)| strop_core::Range::charwise(start, end.max(start)))
            } else {
                editor.visual_range()
            }
        } else {
            None
        },
        // occurrence selections (0049 §7): every stretched extra paints
        // its range like the primary's visual selection
        extra_selections: if view.overlays
            && view.doc == editor.current()
            && editor.mode() == crate::editor::Mode::Insert
            && editor.snippet_active()
        {
            // 0069 D05: linked stops and mirrored targets paint like the
            // primary placeholder
            editor
                .snippet_selection_ranges()
                .1
                .into_iter()
                .filter(|(a, b)| a < b)
                .map(|(start, end)| strop_core::Range::charwise(start, end))
                .collect()
        } else if view.overlays
            && view.doc == editor.current()
            && matches!(editor.mode(), crate::editor::Mode::Visual)
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
    let (gutter_kind, number_width) = editor.gutter(view.doc);
    let inset = sidebar_w
        + blame.map_or(0, |_| strop_engine::editor::prepare::BLAME_GUTTER_WIDTH)
        + number_width;
    let width = usize::from(area.width).saturating_sub(inset);
    // 0051 R08: the fixed margins measure with the viewed document's
    // resolved width too — one setting, no render-side config shortcut.
    let margin_tab = editor.doc(view.doc).indent.width.max(1);
    let help = buf.name.as_deref() == Some("help");
    let mut lines: Vec<Line> = Vec::with_capacity(rows);
    for row in 0..rows {
        let line_idx = view.view_top.saturating_add(row);
        // the fixed margins: sidebar cell (or blank), then the blame
        // cell (or blank past the buffer's lines) — fitted to their
        // assigned width so wide/control text cannot move the inset
        let mut left: Vec<Span> = Vec::new();
        if let Some((tree, files, current)) = &sidebar {
            left.extend(fixed_spans(
                diff::sidebar_row_spans(t, tree, files, current, line_idx, sidebar_focused),
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
                        diff::blame_spans(t, bl, editor.tape().now().unix_seconds)
                    }
                }
                None => diff::blame_blank(),
            };
            left.extend(fixed_spans(
                vec![span],
                strop_engine::editor::prepare::BLAME_GUTTER_WIDTH,
                margin_tab,
            ));
        }
        if line_idx > buf.last_content_line() {
            if let Some(directory) = directory {
                if line_idx == 2 && directory.visible.is_empty() {
                    left.push(Span::raw(" ".repeat(number_width)));
                    left.push(Span::styled(
                        directory::empty_message(directory),
                        Style::default().fg(t.muted),
                    ));
                }
            } else {
                left.push(Span::styled("~", Style::default().fg(t.faint)));
            }
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
        style.row_fg = None;
        style.decorations.clear();
        style.note = None;
        style.syn_spans = syn_spans;
        style.search_hits = search_hits;
        // Collection body rows: source-projected syntax + query hits
        if let Some(Some(_)) = collection.rows.get(row) {
            if let Some(spans) = collection.spans.get(row).and_then(Option::as_deref) {
                style.syn_spans = spans;
            }
            if let Some(hits) = collection.hits.get(row) {
                if !hits.is_empty() && search_hits.is_empty() {
                    style.search_hits = hits;
                }
            }
        }
        match editor.review_row(view.doc, line_idx) {
            Some(crate::editor::ReviewRow::Heading | crate::editor::ReviewRow::File) => {
                style.row_fg = Some(t.accent)
            }
            Some(crate::editor::ReviewRow::Hunk) => {
                style.row_fg = Some(t.muted);
                style.row_bg = Some(t.surface);
            }
            Some(crate::editor::ReviewRow::Removed) => {
                style.row_fg = Some(t.diff_del);
                style.row_bg =
                    crate::render::block::diff_line::origin_bg(t, strop_git::LineOrigin::Deletion);
            }
            Some(crate::editor::ReviewRow::Added) => {
                style.row_fg = Some(t.diff_add);
                style.row_bg =
                    crate::render::block::diff_line::origin_bg(t, strop_git::LineOrigin::Addition);
            }
            Some(crate::editor::ReviewRow::Warning) => style.row_fg = Some(t.warning),
            _ => {}
        }
        // a commit message's `#` lines are commentary git leaves out
        if text.chars().next() == Some('#') && editor.is_commit_message(view.doc) {
            style.row_fg = Some(t.syn.comment);
        }
        let mut fixed = false;
        match surface.and_then(|surface| surface.diff_row(line_idx)) {
            Some(_) => {
                let cursor = view.overlays && line_idx == cur_line;
                if let Some(painted) = surface.and_then(|surface| {
                    diff::painted(t, surface, line_idx, cursor, number_width, width)
                }) {
                    left.extend(painted.gutter);
                    style.decorations = painted.text;
                    style.row_bg = painted.row_bg;
                    if let Some((line, emphasis)) = painted.line {
                        style.diff_line = Some(line);
                        style.emphasis = emphasis;
                    }
                    fixed = painted.fixed;
                }
            }
            None if gutter_kind == strop_engine::editor::prepare::Gutter::Source => {
                if let Some(info) = collection.rows.get(row).and_then(Option::as_ref) {
                    let (bar, bar_color) = gutter_mark(editor, t, view, line_idx);
                    let sign = Span::styled(
                        bar,
                        Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                    );
                    let painted = collection::painted(
                        editor,
                        t,
                        view,
                        info,
                        line_idx,
                        text,
                        sign,
                        (number_width, width),
                    );
                    left.extend(painted.gutter);
                    style.decorations = painted.text;
                    style.row_bg = painted.row_bg;
                    fixed = painted.fixed;
                }
            }
            None if gutter_kind == strop_engine::editor::prepare::Gutter::List => {
                // the list gutter (0100 §3.5): no numbers; the cursor row
                // is marked in the sign column, the directory buffer's idiom
                use crate::render::block::gutter::{self, Sign};
                let cursor = view.overlays && line_idx == cur_line;
                let painted = match surface {
                    Some(crate::editor::Surface::Status { view: status, .. }) => {
                        diff::status_painted(t, status, line_idx, cur_line, cursor, width)
                    }
                    _ => diff::surface_list_row(t, surface, line_idx, width, cursor).map(|row| {
                        crate::render::block::Painted {
                            gutter: Vec::new(),
                            text: row.spans,
                            row_bg: row.row_bg,
                            ..Default::default()
                        }
                    }),
                };
                let mut painted = painted.unwrap_or_default();
                if painted.gutter.is_empty() {
                    painted.gutter =
                        gutter::list(t, if cursor { Sign::Cursor } else { Sign::Blank });
                }
                left.extend(painted.gutter);
                style.decorations = painted.text;
                style.row_bg = painted.row_bg;
                if let Some((line, emphasis)) = painted.line {
                    style.diff_line = Some(line);
                    style.emphasis = emphasis;
                }
                fixed = painted.fixed;
            }
            None => {
                let num_style = if line_idx == cur_line {
                    Style::default().fg(t.accent)
                } else {
                    Style::default().fg(t.muted)
                };
                // Helix-grade gutter: a colored ▎ bar in the leftmost
                // column — diagnostics first, then git signs
                let (bar, bar_color) = gutter_mark(editor, t, view, line_idx);
                left.push(Span::styled(
                    bar,
                    Style::default().fg(bar_color).add_modifier(Modifier::BOLD),
                ));
                left.push(Span::styled(
                    format!("{:>digits$} ", line_idx + 1, digits = number_width - 2),
                    num_style,
                ));
                if let Some(row) = directory
                    .and_then(|source| {
                        directory::row(
                            t,
                            source,
                            line_idx,
                            text,
                            view.overlays && line_idx == cur_line,
                        )
                    })
                    .or_else(|| {
                        diff::surface_list_row(t, surface, line_idx, width, line_idx == cur_line)
                    })
                {
                    // the quiet cursor-row band rides under overlays
                    // (search/visual/flash still override per cell)
                    style.decorations = row.spans;
                    style.row_bg = row.row_bg;
                } else if help {
                    // the help buffer gets house-style decoration
                    // (render/help.rs): one accent, keycaps, no hues
                    style.decorations =
                        crate::render::help::row_spans(t, &text.to_string(), width as u16);
                } else if view.overlays && line_idx == cur_line && surface.is_none() {
                    // the cursor line is painted in file buffers (rule 1);
                    // selection/search/flash still override per cell
                    style.row_bg = Some(t.cursorline);
                }
            }
        }
        // cursor-line end-of-line diagnostic (scoped to the one line —
        // you see what the dot means without leaving the buffer). The
        // painter clips at the row's real width with an ellipsis
        // (0068 B1: the old 80-char pre-clip lied about the budget).
        if view.overlays && line_idx == cur_line {
            if let Some((sev, msg)) = editor.diag_message_at(view.doc, line_idx + 1) {
                let shown = msg.replace('\n', " · ");
                style.note = Some((
                    format!("  ▍ {shown}"),
                    Style::default()
                        .fg(t.dim(t.severity(sev)))
                        .add_modifier(Modifier::ITALIC),
                ));
            }
        }
        let fixed_view = PaneView {
            hscroll: DisplayColumn::new(0),
            ..*view
        };
        // the band under a directory row or the cursor line covers the
        // gutter too, so the row reads as one surface
        if directory.is_some() || (view.overlays && line_idx == cur_line && !help) {
            if let Some(bg) = style.row_bg {
                for span in &mut left {
                    span.style.bg = Some(bg);
                }
            }
        }
        left.extend(content_spans(
            editor,
            t,
            if fixed { &fixed_view } else { view },
            start,
            text,
            &style,
            width,
        ));
        lines.push(pad_row(Line::from(left), area.width));
    }
    frame.render_widget(
        Paragraph::new(lines).style(Style::default().bg(t.base)),
        area,
    );
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
                    cell.set_symbol("│").set_fg(t.guide);
                }
            }
        }
    }
}

/// The sign column: diagnostics win over git signs (merged gutter,
/// 0009), and only the pane's own buffer shows them.
fn gutter_mark(
    editor: &Editor,
    t: &Tokens,
    view: &PaneView,
    line_idx: usize,
) -> (&'static str, Color) {
    if let Some(directory) = editor
        .doc(view.doc)
        .directory_metadata_ref()
        .filter(|source| source.draft.is_none())
    {
        if let Some(entry) = directory.entry(strop_core::id::LineIndex::new(line_idx)) {
            if directory.marked.contains_key(&entry.name) {
                return ("●", t.accent);
            }
            if entry.error.is_some() {
                return ("!", t.diff_del);
            }
        }
        if view.overlays && editor.doc(view.doc).buf.line_of(view.cursor) == line_idx {
            return ("▌", t.accent);
        }
    }
    if let Some(sev) = editor.diag_severity_at(view.doc, line_idx + 1) {
        // severity dot (VSCode/gitui lesson: color reads faster than
        // letters) — the cursor line's EOL note carries the words
        return ("●", t.severity(sev));
    }
    // git signs: + add, ~ change, - deletion below (only for the
    // working buffer — surfaces have no path, so no leak)
    if view.doc == editor.current() {
        // the four states in one column: unstaged sign wins; staged-only
        // lines get the committed-adjacent tint (0014 wave 4)
        if editor.sign_at(line_idx + 1).is_none() && editor.sign_at_staged(line_idx + 1) {
            return ("▎", t.dim(t.diff_add));
        }
        match editor.sign_at(line_idx + 1) {
            Some('+') => return ("▎", t.diff_add),
            Some('~') => return ("▎", t.diff_change),
            Some('-') => return ("▎", t.diff_del),
            _ => {}
        }
    }
    (" ", t.muted)
}
