//! `:s` in place (plan 0110 §3.2, Neovim's `inccommand=nosplit`): while
//! a substitution is typed, each visible line it changes is drawn as it
//! will read, the new text in `diff.add`. The text comes prepared with
//! the plan `Enter` applies (`PreviewLine`); this only lays it out.

use ratatui::style::Style;
use ratatui::text::Span;

use crate::editor::PreviewLine;
use crate::render::style::Tokens;

/// The row's text spans: tabs to `tab` stops, `hscroll` cells skipped,
/// at most `width` cells.
pub(super) fn spans(
    t: &Tokens,
    line: &PreviewLine,
    hscroll: usize,
    width: usize,
    tab: usize,
) -> Vec<Span<'static>> {
    let plain = Style::default().fg(t.text);
    let added = Style::default().fg(t.diff_add);
    let mut spans: Vec<Span<'static>> = Vec::new();
    let mut run = String::new();
    let mut run_added = false;
    let mut column = 0;
    for (byte, c) in line.text.char_indices() {
        let is_added = line
            .added
            .iter()
            .any(|&(start, end)| (start..end).contains(&byte));
        let cells = if c == '\t' {
            tab - column % tab
        } else {
            crate::render::clip::width(c.encode_utf8(&mut [0u8; 4]))
        };
        let next = column + cells;
        if next <= hscroll {
            column = next;
            continue;
        }
        if next > hscroll + width {
            break;
        }
        if is_added != run_added && !run.is_empty() {
            let style = if run_added { added } else { plain };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        run_added = is_added;
        if c == '\t' || column < hscroll {
            // a tab, or a wide character cut by the left edge
            run.extend(std::iter::repeat_n(' ', next - column.max(hscroll)));
        } else if c.is_control() {
            run.push('\u{fffd}');
        } else {
            run.push(c);
        }
        column = next;
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_added { added } else { plain }));
    }
    spans
}
