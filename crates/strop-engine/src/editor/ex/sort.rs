//! `:[range]sort[!] [u][n][i]`: Vim's line sort. Stable; `!` is the exact
//! reverse; `n` sorts by the first decimal number on the line (lines
//! without one come first); `i` ignores case; `u` keeps the first of
//! equal neighbours.

use std::cmp::Ordering;

use strop_core::Range;

use super::ExCall;
use crate::editor::ex::*;
use crate::editor::Editor;

/// The first decimal number on the line, with a `-` directly before it.
fn number(line: &str) -> Option<i128> {
    let start = line.find(|c: char| c.is_ascii_digit())?;
    let digits = line[start..].len()
        - line[start..]
            .trim_start_matches(|c: char| c.is_ascii_digit())
            .len();
    // A number too long to hold sorts as the largest.
    let value = line[start..start + digits]
        .parse::<i128>()
        .unwrap_or(i128::MAX);
    Some(if line[..start].ends_with('-') {
        -value
    } else {
        value
    })
}

pub(crate) fn ex_sort(editor: &mut Editor, call: &ExCall<'_>) {
    let (mut unique, mut numeric, mut ignore_case) = (false, false, false);
    for flag in call.arg.chars() {
        match flag {
            'u' => unique = true,
            'n' => numeric = true,
            'i' => ignore_case = true,
            ' ' => {}
            other => {
                editor.message = format!("unsupported :sort argument: {other}");
                return;
            }
        }
    }
    let (first, last) = call
        .range
        .map_or((0, editor.buf().last_content_line()), |range| range.lines());
    let compare = |a: &String, b: &String| -> Ordering {
        if numeric {
            number(a).cmp(&number(b))
        } else if ignore_case {
            a.to_lowercase().cmp(&b.to_lowercase())
        } else {
            a.cmp(b)
        }
    };
    let original: Vec<String> = (first..=last)
        .map(|line| editor.buf().line_text(line))
        .collect();
    let mut sorted = original.clone();
    sorted.sort_by(compare);
    if call.bang {
        sorted.reverse();
    }
    if unique {
        sorted.dedup_by(|later, earlier| compare(earlier, later) == Ordering::Equal);
    }
    if sorted != original {
        let start = editor.buf().line_start(first);
        let end = editor.buf().line_end(last);
        editor.tx_begin();
        editor.buf_mut().delete(Range::charwise(start, end));
        editor.buf_mut().insert(start, &sorted.join("\n"));
        editor.tx_commit();
    }
    ex_land(editor, first);
}
