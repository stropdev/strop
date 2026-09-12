//! Shared cell-clipped content and fixed-margin rendering.
use super::*;

/// Content spans for one row, clipped to the pane's visible cell
/// window: syntax/decoration base, overlays composed on top (search <
/// preview < flash, 0001 §5.8). Styles are evaluated only for the
/// visible interval; a partially clipped wide grapheme becomes styled
/// blanks, never half a glyph; a zero-width cluster occupies no cell.
pub(super) fn content_spans(
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
    let tab = editor.indentation_at(view.doc, start).width.max(1);
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
            cell = cell.patch(crate::render::syntax_style(&style.syn_spans[syn_idx]));
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
                let covers = |r: &strop_core::Range| {
                    r.start.get() < pos + grapheme.len() && pos < r.end.get()
                };
                style.selection.as_ref().is_some_and(covers)
                    || style.extra_selections.iter().any(covers)
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
            // matching delimiters (0051 §7 R09): a quiet slate wash on
            // both endpoints — under find/preview/flash, over the
            // selection and search layers
            if style.pair_first == Some(pos) || style.pair_second == Some(pos) {
                cell = cell.bg(PAIR_BG).add_modifier(Modifier::BOLD);
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
pub(super) fn fixed_spans(
    input: Vec<Span<'static>>,
    width: usize,
    tab: usize,
) -> Vec<Span<'static>> {
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
