//! Paragraphs, sentences and unmatched brackets: `{` `}` `(` `)` `[(`
//! `[{` `])` `]}` and the `ip` / `ap` object — ports of Vim's `findpar`,
//! `findsent` and `current_par`.

use strop_core::Buffer;

use super::walk::{line_white, Step, Walk};

/// nroff macros that start a paragraph or section (Vim's default
/// 'paragraphs' and 'sections'), as two-character names.
const MACROS: &str = "IPLPPPQPP TPHPLIPpLpItpplpipbpSHNHH HUnhsh";

/// Vim's `startPS`: an empty line, a form feed, or an nroff macro line.
fn starts_paragraph(buf: &Buffer, line: usize) -> bool {
    let (start, end) = (buf.line_start(line), buf.line_end(line));
    if start == end {
        return true;
    }
    match buf.byte(start) {
        0x0c => true,
        b'.' => {
            let at = |offset: usize| buf.byte_at(start + offset).filter(|_| start + offset < end);
            let (first, second) = (at(1), at(2));
            MACROS.as_bytes().chunks(2).any(|pair| {
                let blank = |byte: Option<u8>| byte.is_none() || byte == Some(b' ');
                (first == Some(pair[0]) || (pair[0] == b' ' && blank(first)))
                    && (second == Some(pair[1])
                        || (pair[1] == b' ' && (blank(first) || blank(second))))
            })
        }
        _ => false,
    }
}

/// `{` / `}`: the target position and whether the motion became
/// inclusive (it does on the buffer's last line). `None` when the count
/// runs past the buffer's edge.
pub(crate) fn paragraph(
    buf: &Buffer,
    cursor: usize,
    forward: bool,
    count: usize,
) -> Option<(usize, bool)> {
    let last = buf.last_content_line();
    let mut line = buf.line_of(cursor).min(last);
    for remaining in (0..count).rev() {
        let mut skipped = false;
        let mut first = true;
        loop {
            if buf.line_start(line) != buf.line_end(line) {
                skipped = true;
            }
            if !first && skipped && starts_paragraph(buf, line) {
                break;
            }
            let edge = if forward { line == last } else { line == 0 };
            if edge {
                if remaining > 0 {
                    return None;
                }
                break;
            }
            line = if forward { line + 1 } else { line - 1 };
            first = false;
        }
    }
    let (start, end) = (buf.line_start(line), buf.line_end(line));
    if forward && line == last && end > start {
        return Some((buf.clamp_boundary(end - 1).max(start), true));
    }
    Some((start, false))
}

const CLOSERS: &str = ")]\"'";
const ENDERS: &str = ".!?";

/// `(` / `)`: where the count-th sentence boundary is. `None` when the
/// count runs past the buffer's edge.
pub(crate) fn sentence(buf: &Buffer, cursor: usize, forward: bool, count: usize) -> Option<usize> {
    let mut pos = Walk::new(buf, cursor);
    let step = |walk: &mut Walk| if forward { walk.incl() } else { walk.decl() };
    for remaining in (0..count).rev() {
        let mut skip_white = true;
        'found: {
            if pos.at_end() {
                // On an empty line: skip to the next non-empty one.
                loop {
                    if step(&mut pos) == Step::Edge || !pos.at_end() {
                        break;
                    }
                }
                if forward {
                    break 'found;
                }
            } else if forward && pos.col() == 0 && starts_paragraph(buf, pos.line()) {
                if pos.on_last_line() {
                    return None;
                }
                pos.goto_line(pos.line() + 1);
                break 'found;
            } else if !forward {
                pos.decl();
            }

            // Back to the previous character that is not blank or
            // closing punctuation.
            let mut found_dot = false;
            while let Some(c) = pos.ch() {
                if !(c == ' ' || c == '\t' || ENDERS.contains(c) || CLOSERS.contains(c)) {
                    break;
                }
                let mut before = pos.clone();
                if before.decl() == Step::Edge || (before.on_empty_line() && forward) {
                    break;
                }
                if found_dot {
                    break;
                }
                if ENDERS.contains(c) {
                    found_dot = true;
                }
                if CLOSERS.contains(c)
                    && !before
                        .ch()
                        .is_some_and(|b| ENDERS.contains(b) || CLOSERS.contains(b))
                {
                    break;
                }
                pos.decl();
            }

            let start_line = pos.line();
            loop {
                let Some(c) = pos.ch() else {
                    if !forward && pos.line() != start_line {
                        pos.goto_line(pos.line() + 1);
                    }
                    break;
                };
                if pos.col() == 0 && starts_paragraph(buf, pos.line()) {
                    if !forward && pos.line() != start_line {
                        pos.goto_line(pos.line() + 1);
                    }
                    break;
                }
                if ENDERS.contains(c) {
                    let mut after = pos.clone();
                    let mut edge;
                    loop {
                        edge = after.inc() == Step::Edge;
                        if edge || !after.ch().is_some_and(|c| CLOSERS.contains(c)) {
                            break;
                        }
                    }
                    if edge || matches!(after.ch(), None | Some(' ' | '\t')) {
                        pos = after;
                        if pos.at_end() {
                            pos.inc();
                        }
                        break;
                    }
                }
                if step(&mut pos) == Step::Edge {
                    if remaining > 0 {
                        return None;
                    }
                    skip_white = false;
                    break;
                }
            }
        }
        while skip_white && matches!(pos.ch(), Some(' ' | '\t')) {
            if pos.incl() == Step::Edge {
                break;
            }
        }
    }
    Some(pos.pos)
}

/// `[(` `[{` (backward) and `])` `]}` (forward): the count-th unmatched
/// bracket of this kind, or the last one there is short of the count.
pub(crate) fn unmatched_upto(
    buf: &Buffer,
    cursor: usize,
    open: u8,
    close: u8,
    forward: bool,
    count: usize,
) -> Option<usize> {
    let mut found = None;
    let mut at = cursor;
    // Vim: from inside a double-quoted string, only brackets inside
    // that string match.
    let line = buf.line_of(cursor);
    let (line_start, line_end) = (buf.line_start(line), buf.line_end(line));
    let quotes: Vec<usize> = (line_start..line_end)
        .filter(|&i| buf.byte(i) == b'"' && (i == line_start || buf.byte(i - 1) != b'\\'))
        .collect();
    let before = quotes.iter().filter(|&&quote| quote < cursor).count();
    let within =
        (before % 2 == 1 && before < quotes.len()).then(|| (quotes[before - 1], quotes[before]));
    for _ in 0..count {
        match unmatched(buf, at, open, close, forward, 1).filter(|next| {
            // (brackets on other lines are not the string's concern)
            !(line_start..line_end).contains(next)
                || within.is_none_or(|(low, high)| low < *next && *next < high)
        }) {
            Some(next) => {
                at = next;
                found = Some(next);
            }
            None => break,
        }
    }
    found
}

/// The count-th unmatched bracket of this kind before (or, `forward`,
/// after) the cursor.
pub(crate) fn unmatched(
    buf: &Buffer,
    cursor: usize,
    open: u8,
    close: u8,
    forward: bool,
    count: usize,
) -> Option<usize> {
    let n = buf.len_bytes();
    let mut pos = cursor;
    for _ in 0..count {
        let mut depth = 0usize;
        loop {
            if forward {
                pos += 1;
                if pos >= n {
                    return None;
                }
            } else {
                if pos == 0 {
                    return None;
                }
                pos -= 1;
            }
            let byte = buf.byte(pos);
            let (same, other) = if forward {
                (close, open)
            } else {
                (open, close)
            };
            if byte == other {
                depth += 1;
            } else if byte == same {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
        }
    }
    Some(pos)
}

/// `ip` / `ap` outside Visual: the first and last line of the object.
pub(crate) fn paragraph_object(
    buf: &Buffer,
    cursor: usize,
    count: usize,
    around: bool,
) -> Option<(usize, usize)> {
    let last = buf.last_content_line();
    let white = |line: usize| line_white(buf, line);
    let mut start = buf.line_of(cursor).min(last);
    let white_in_front = white(start);
    while start > 0 {
        if white_in_front {
            if !white(start - 1) {
                break;
            }
        } else if white(start - 1) || starts_paragraph(buf, start) {
            break;
        }
        start -= 1;
    }
    // One past the white lines at the start, as Vim counts it.
    let mut end = start;
    while end <= last && white(end) {
        end += 1;
    }
    let mut end = end as isize - 1;
    let mut remaining = count;
    if !around && white_in_front {
        remaining -= 1;
    }
    while remaining > 0 {
        remaining -= 1;
        if end == last as isize {
            return None;
        }
        let next = (end + 1) as usize;
        let do_white = !around && white(next);
        if around || !do_white {
            end += 1;
            while (end as usize) < last
                && !white(end as usize + 1)
                && !starts_paragraph(buf, end as usize + 1)
            {
                end += 1;
            }
        }
        if remaining == 0 && white_in_front && around {
            break;
        }
        if around || do_white {
            while (end as usize) < last && white(end as usize + 1) {
                end += 1;
            }
        }
    }
    let end = end.max(start as isize) as usize;
    if !white_in_front && !white(end) && around {
        while start > 0 && white(start - 1) {
            start -= 1;
        }
    }
    Some((start, end))
}

/// `ip` / `ap` extending a Visual selection whose head is on another
/// line than its anchor: the line the head moves to.
pub(crate) fn paragraph_extend(
    buf: &Buffer,
    anchor: usize,
    head: usize,
    count: usize,
    around: bool,
) -> Option<usize> {
    let last = buf.last_content_line();
    let white = |line: usize| line_white(buf, line);
    let from = buf.line_of(head).min(last);
    let mut line = from;
    let forward = line >= buf.line_of(anchor);
    let edge = |line: usize| if forward { line == last } else { line == 0 };
    let step = |line: usize| if forward { line + 1 } else { line - 1 };
    for _ in 0..count {
        if edge(line) {
            break;
        }
        let mut previous: Option<bool> = None;
        for _ in 0..2 {
            line = step(line);
            let is_white = white(line);
            if previous == Some(is_white) {
                line = if forward { line - 1 } else { line + 1 };
                break;
            }
            while !edge(line) {
                let next = step(line);
                let boundary = if forward { next } else { line };
                if is_white != white(next) || (!is_white && starts_paragraph(buf, boundary)) {
                    break;
                }
                line = next;
            }
            if !around || edge(line) {
                break;
            }
            previous = Some(is_white);
        }
    }
    (line != from).then_some(line)
}
