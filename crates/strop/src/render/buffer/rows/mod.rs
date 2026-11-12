//! One pane's rows: the fixed margins, each row's owner painted into a
//! gutter and styled text (`owner.rs`), the live overlays composed over
//! it (`overlays.rs`), and the indent guides. Ordinary file and diff rows
//! stream graphemes straight off the rope — no whole-line String, no
//! layout vector; only decorated rows (headings, help, lists) carry their
//! own styled text.
use super::*;

mod collection;
mod commit_message;
mod overlays;
mod owner;
use collection::CollectionProjection;
use owner::{Cells, Pane, Row};

/// Render one pane's rows: margins, gutter, syntax/decoration, overlays,
/// guides.
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
    let pair = if view.overlays {
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
    let mut style = overlays::pane_style(editor, view, syn_spans, search_hits, pair);

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
    let cells = Cells {
        gutter: number_width,
        text: width,
        // a commit's files are one block as wide as its content (§3.9)
        files_block: match surface {
            Some(crate::editor::Surface::ChangedFiles { sha, files, .. }) => {
                diff::files_block(files, sha, number_width + width)
            }
            _ => 0,
        },
    };
    let pane = Pane {
        gutter: gutter_kind,
        surface,
        directory,
        help: buf.name.as_deref() == Some("help"),
        commit_message: editor.is_commit_message(view.doc),
    };
    // 0051 R08: the fixed margins measure with the viewed document's
    // resolved width too — one setting, no render-side config shortcut.
    let margin_tab = editor.doc(view.doc).indent.width.max(1);
    let fixed_view = PaneView {
        hscroll: DisplayColumn::new(0),
        ..*view
    };
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
        let info = collection.rows.get(row).and_then(Option::as_ref);

        style.syn_spans = syn_spans;
        style.search_hits = search_hits;
        // Collection body rows: source-projected syntax + query hits
        if info.is_some() {
            if let Some(spans) = collection.spans.get(row).and_then(Option::as_deref) {
                style.syn_spans = spans;
            }
            if let Some(hits) = collection.hits.get(row) {
                if !hits.is_empty() && search_hits.is_empty() {
                    style.search_hits = hits;
                }
            }
        }
        let cursor = view.overlays && line_idx == cur_line;
        let owner = pane.owner(editor, view.doc, line_idx, info);
        let painted = owner.paint(
            editor,
            t,
            view,
            &Row {
                line: line_idx,
                text,
                cursor,
                cursor_line: cur_line,
            },
            cells,
        );
        left.extend(painted.gutter);
        style.decorations = painted.text;
        style.row_bg = painted.row_bg;
        style.row_fg = painted.row_fg;
        (style.diff_line, style.emphasis) = match painted.line {
            Some((line, emphasis)) => (Some(line), emphasis),
            None => (None, None),
        };
        style.ellipsis = painted.cut;
        style.note = cursor
            .then(|| overlays::cursor_note(editor, t, view, line_idx))
            .flatten();
        if owner.band_covers_gutter(cursor) {
            if let Some(bg) = style.row_bg {
                for span in &mut left {
                    span.style.bg = Some(bg);
                }
            }
        }
        left.extend(content_spans(
            editor,
            t,
            if painted.fixed { &fixed_view } else { view },
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
