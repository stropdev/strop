//! Cell-budgeted match windows. Seek around the known byte span rather than
//! allocating/scanning a complete long line on every frame.
use super::super::text;
use strop_core::layout::{printable_grapheme, RopeGraphemes};
use unicode_segmentation::{GraphemeCursor, UnicodeSegmentation};

fn char_floor(input: &str, at: usize) -> usize {
    let mut at = at.min(input.len());
    while !input.is_char_boundary(at) {
        at -= 1;
    }
    at
}
fn grapheme_floor(input: &str, at: usize) -> usize {
    let at = char_floor(input, at);
    let mut cursor = GraphemeCursor::new(at, input.len(), true);
    if matches!(cursor.is_boundary(input, 0), Ok(true)) {
        at
    } else {
        input[..at]
            .grapheme_indices(true)
            .next_back()
            .map_or(0, |(start, _)| start)
    }
}
fn before(input: &str, end: usize, cells: usize, tab: usize) -> usize {
    let mut used = 0;
    let mut start = end;
    for (at, grapheme) in input[..end].grapheme_indices(true).rev() {
        let width = if grapheme == "\t" {
            tab
        } else {
            text::width(grapheme)
        };
        if used + width > cells {
            break;
        }
        used += width;
        start = at;
    }
    start
}
fn after(input: &str, start: usize, cells: usize, tab: usize) -> usize {
    let mut end = start;
    for (glyph, grapheme) in RopeGraphemes::new(input[start..].into(), tab) {
        if glyph.cell.get() + glyph.width > cells {
            break;
        }
        end = start + glyph.byte + grapheme.len();
    }
    end
}

pub(super) fn match_window(
    line: &str,
    start: usize,
    end: usize,
    budget: usize,
    tab: usize,
) -> (String, (usize, usize)) {
    if budget == 0 {
        return (String::new(), (0, 0));
    }
    let start = char_floor(line, start);
    let end = char_floor(line, end.max(start));
    let whole_fits = after(line, 0, budget, tab) == line.len();
    let match_start = grapheme_floor(line, start);
    let from = if whole_fits {
        0
    } else if after(line, match_start, budget.saturating_sub(2), tab) < end {
        match_start
    } else {
        before(line, match_start, budget / 3, tab)
    };
    let mut output = String::new();
    let mut used = 0;
    let mut chars = 0;
    if from > 0 {
        output.push('…');
        used = 1;
        chars = 1;
    }
    let mut match_lo = None;
    let mut match_hi = 0;
    let mut reached = from;
    for (glyph, grapheme) in RopeGraphemes::new(line[from..].into(), tab) {
        let source_start = from + glyph.byte;
        let source_end = source_start + grapheme.len();
        let tail = usize::from(source_end < line.len());
        if used + glyph.width + tail > budget {
            break;
        }
        let count = if grapheme == "\t" {
            output.extend(std::iter::repeat_n(' ', glyph.width));
            glyph.width
        } else {
            let shown = printable_grapheme(&grapheme);
            output.push_str(shown);
            shown.chars().count()
        };
        if source_start < end && start < source_end {
            match_lo.get_or_insert(chars);
            match_hi = chars + count;
        }
        chars += count;
        used += glyph.width;
        reached = source_end;
    }
    if reached < line.len() && used < budget {
        output.push('…');
    }
    (output, match_lo.map_or((0, 0), |first| (first, match_hi)))
}

pub(super) fn replacement_window(
    line: &str,
    start: usize,
    end: usize,
    replacement: &str,
    budget: usize,
    tab: usize,
) -> (String, (usize, usize)) {
    if budget == 0 {
        return (String::new(), (0, 0));
    }
    let prefix_start = before(line, grapheme_floor(line, start), budget, tab);
    let replacement_end = after(replacement, 0, budget.saturating_mul(2), tab);
    let suffix_end = if replacement_end == replacement.len() {
        after(line, end, budget, tab)
    } else {
        end
    };
    let mut excerpt = String::new();
    if prefix_start > 0 {
        excerpt.push('…');
    }
    excerpt.push_str(&line[prefix_start..start]);
    let first = excerpt.len();
    excerpt.push_str(&replacement[..replacement_end]);
    let last = excerpt.len();
    excerpt.push_str(&line[end..suffix_end]);
    if replacement_end < replacement.len() || suffix_end < line.len() {
        excerpt.push('…');
    }
    match_window(&excerpt, first, last, budget, tab)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn whitespace_and_unicode_keep_exact_match_evidence() {
        let text = "    界e\u{301} match";
        let start = text.find("match").unwrap();
        let (window, span) = match_window(text, start, start + 5, 30, 4);
        assert!(window.starts_with("    界e\u{301}"));
        assert_eq!(
            window
                .chars()
                .skip(span.0)
                .take(span.1 - span.0)
                .collect::<String>(),
            "match"
        );
    }
    #[test]
    fn zero_width_and_end_of_line_matches_never_index_past_text() {
        for budget in 0..5 {
            let (window, _) = match_window("界界", 6, 6, budget, 4);
            assert!(text::width(&window) <= budget);
            let (window, _) = match_window("", 0, 0, budget, 4);
            assert!(window.is_empty());
        }
    }
    #[test]
    fn long_replacements_keep_only_visible_context() {
        let line = format!("{}needle{}", "x".repeat(100_000), "z".repeat(100_000));
        let (window, span) = replacement_window(&line, 100_000, 100_006, "界new", 20, 4);
        assert!(text::width(&window) <= 20);
        assert_eq!(
            window
                .chars()
                .skip(span.0)
                .take(span.1 - span.0)
                .collect::<String>(),
            "界new"
        );
    }
}
