//! The live overlays a pane's rows compose over their owner's paint:
//! preview, flash, the selections, the block rectangle, search hits,
//! matching delimiters and pending find candidates. Only the active pane
//! shows them; an inactive pane gets none.

use ratatui::style::{Modifier, Style};

use super::{Editor, PaneView, RowStyle, Tokens};

/// The pane-wide part of every row's style.
pub(super) fn pane_style<'a>(
    editor: &Editor,
    view: &PaneView,
    syn_spans: &'a [strop_syntax::Span],
    search_hits: &'a [strop_grammar::SearchMatch],
    [pair_first, pair_second]: [Option<usize>; 2],
) -> RowStyle<'a> {
    RowStyle {
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
        extra_selections: extra_selections(editor, view),
        block: view.overlays.then(|| editor.block_rect_pub()).flatten(),
        search_hits,
        pair_first,
        pair_second,
        find: view.overlays.then(|| editor.find_candidates()).flatten(),
        ..Default::default()
    }
}

/// Occurrence selections (0049 §7): every stretched extra paints its
/// range like the primary's visual selection; in a snippet session the
/// linked stops and mirrored targets paint like the active placeholder
/// (0069 D05).
fn extra_selections(editor: &Editor, view: &PaneView) -> Vec<strop_core::Range> {
    if !view.overlays || view.doc != editor.current() {
        return Vec::new();
    }
    match editor.mode() {
        crate::editor::Mode::Insert if editor.snippet_active() => editor
            .snippet_selection_ranges()
            .1
            .into_iter()
            .filter(|(a, b)| a < b)
            .map(|(start, end)| strop_core::Range::charwise(start, end))
            .collect(),
        crate::editor::Mode::Visual => editor
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
            .collect(),
        _ => Vec::new(),
    }
}

/// The cursor line's end-of-line diagnostic (scoped to the one line —
/// you see what the dot means without leaving the buffer). The painter
/// clips at the row's real width with an ellipsis (0068 B1).
pub(super) fn cursor_note(
    editor: &Editor,
    t: &Tokens,
    view: &PaneView,
    line: usize,
) -> Option<(String, Style)> {
    let (severity, message) = editor.diag_message_at(view.doc, line + 1)?;
    Some((
        format!("  ▍ {}", message.replace('\n', " · ")),
        Style::default()
            .fg(t.dim(t.severity(severity)))
            .add_modifier(Modifier::ITALIC),
    ))
}
