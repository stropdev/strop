//! The byte ranges one highlight request covers (0111 decision 6).
//!
//! Like vim's `synmaxcol`, a line is highlighted only up to a budget: the
//! rest of a longer line keeps the default colour. vim counts the budget
//! in bytes (its columns are byte indices) and defaults it to 3000; strop
//! uses the same number with the same unit. 3000 bytes is about 20 screen
//! widths of ASCII, far more than a reader scrolls through on one line,
//! while a minified 2.4 MB line no longer turns every keystroke into a
//! million captures. The budget is a property of the line, not of the
//! window, so a byte's colour still does not depend on where the window
//! starts.
use crate::Span;
use ropey::Rope;

/// Bytes of a line that are highlighted, counted from the line's start.
pub const LINE_BUDGET: usize = 3000;

/// `[first, last)` minus every byte past [`LINE_BUDGET`] on its line, as
/// sorted, disjoint, non-empty ranges. Cuts land on char boundaries.
pub(crate) fn segments(rope: &Rope, first: usize, last: usize) -> Vec<(usize, usize)> {
    let last = last.min(rope.len_bytes());
    if first >= last {
        return Vec::new();
    }
    let mut line = rope.byte_to_line(first);
    // fast path: no line touching the window can outgrow the budget
    if last - rope.line_to_byte(line) <= LINE_BUDGET {
        return vec![(first, last)];
    }
    let mut segments = Vec::new();
    let mut start = first;
    let lines = rope.len_lines();
    while line < lines {
        let line_start = rope.line_to_byte(line);
        if line_start >= last {
            break;
        }
        let next = rope.line_to_byte(line + 1);
        let cut = line_start + LINE_BUDGET;
        if next > cut {
            let cut = rope.char_to_byte(rope.byte_to_char(cut));
            let end = cut.min(last);
            if start < end {
                segments.push((start, end));
            }
            start = next.max(start);
        }
        line += 1;
    }
    if start < last {
        segments.push((start, last));
    }
    segments
}

/// Sorted disjoint `spans` intersected with sorted disjoint `segments`.
pub(crate) fn clip_to(spans: Vec<Span>, segments: &[(usize, usize)]) -> Vec<Span> {
    let mut out = Vec::with_capacity(spans.len());
    let mut at = 0;
    for span in spans {
        while at < segments.len() && segments[at].1 <= span.start {
            at += 1;
        }
        for &(a, b) in segments[at..].iter().take_while(|(a, _)| *a < span.end) {
            let (start, end) = (span.start.max(a), span.end.min(b));
            if start < end {
                out.push(Span { start, end, ..span });
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_lines_are_one_segment() {
        let rope = Rope::from_str("a\nbb\nccc\n");
        assert_eq!(segments(&rope, 0, rope.len_bytes()), vec![(0, 9)]);
        assert_eq!(segments(&rope, 2, 5), vec![(2, 5)]);
        assert_eq!(segments(&rope, 5, 5), Vec::<(usize, usize)>::new());
    }

    #[test]
    fn a_long_line_is_cut_at_the_budget() {
        let long = "x".repeat(LINE_BUDGET * 3);
        let text = format!("ab\n{long}\ncd\n");
        let rope = Rope::from_str(&text);
        let line1 = 3;
        let line2 = line1 + long.len() + 1;
        assert_eq!(
            segments(&rope, 0, rope.len_bytes()),
            vec![(0, line1 + LINE_BUDGET), (line2, rope.len_bytes())]
        );
        // a window starting past the cut on the long line covers nothing there
        assert_eq!(
            segments(&rope, line1 + LINE_BUDGET + 10, line2 + 1),
            vec![(line2, line2 + 1)]
        );
        // a window inside the budget is untouched
        assert_eq!(
            segments(&rope, line1 + 5, line1 + 50),
            vec![(line1 + 5, line1 + 50)]
        );
    }

    #[test]
    fn cuts_land_on_char_boundaries() {
        let text = format!(
            "{}é{}\n",
            "x".repeat(LINE_BUDGET - 1),
            "y".repeat(LINE_BUDGET)
        );
        let rope = Rope::from_str(&text);
        // the budget falls inside `é` (bytes LINE_BUDGET-1 .. LINE_BUDGET+1)
        assert_eq!(
            segments(&rope, 0, rope.len_bytes()),
            vec![(0, LINE_BUDGET - 1)]
        );
    }
}
