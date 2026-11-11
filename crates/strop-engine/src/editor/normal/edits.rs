//! Normal-mode commands that are not operators: `J` `gJ`, `~`, `Ctrl-A`
//! `Ctrl-X`, `gp` `gP`, `]p` `[p`. Each takes its count as a parameter —
//! `3J` joins three lines, it does not join three times.

use strop_core::Range;

use crate::editor::registers::RegisterShape;
use crate::editor::Editor;

/// Comment leaders Neovim's default 'comments' removes from a joined
/// line when the line above is a comment too ('formatoptions' has `j`).
/// `true`: the leader must be followed by a blank.
const LEADERS: &[(&str, bool)] = &[("//", false), ("#", true), ("%", false), ("XCOMM", false)];

/// The length of the comment leader (and the blanks after it) that
/// starts `text`, which has no indent.
fn leader_len(text: &str) -> Option<usize> {
    LEADERS.iter().find_map(|(leader, blank)| {
        let rest = text.strip_prefix(leader)?;
        if *blank && !rest.is_empty() && !rest.starts_with([' ', '\t']) {
            return None;
        }
        let blanks = rest.len() - rest.trim_start_matches([' ', '\t']).len();
        Some(leader.len() + blanks)
    })
}

/// A number in a line: its byte range and how it is written.
struct Number {
    start: usize,
    end: usize,
    radix: u32,
    negative: bool,
}

/// The number at or after byte column `col` (Neovim's default
/// 'nrformats': decimal, `0x` hex, `0b` binary).
fn number_at(line: &str, col: usize) -> Option<Number> {
    let bytes = line.as_bytes();
    let digit = |i: usize, radix: u32| bytes.get(i).is_some_and(|b| char::from(*b).is_digit(radix));
    let prefixed = |i: usize, radix: u32, letters: &[u8]| {
        bytes.get(i) == Some(&b'0')
            && bytes.get(i + 1).is_some_and(|b| letters.contains(b))
            && digit(i + 2, radix)
    };
    // A prefixed literal the cursor is inside or on.
    for (radix, letters) in [(16u32, b"xX"), (2, b"bB")] {
        let mut start = col.min(bytes.len().saturating_sub(1));
        while start > 0 && (digit(start, radix) || letters.contains(&bytes[start])) {
            start -= 1;
        }
        for at in [start, start + 1] {
            if prefixed(at, radix, letters) {
                let mut end = at + 2;
                while digit(end, radix) {
                    end += 1;
                }
                if end > col {
                    return Some(Number {
                        start: at,
                        end,
                        radix,
                        negative: false,
                    });
                }
            }
        }
    }
    let mut start = col;
    while start > 0 && digit(start, 10) && digit(start - 1, 10) {
        start -= 1;
    }
    while start < bytes.len() && !digit(start, 10) {
        start += 1;
    }
    if start >= bytes.len() {
        return None;
    }
    for (radix, letters) in [(16u32, b"xX"), (2, b"bB")] {
        if prefixed(start, radix, letters) {
            let mut end = start + 2;
            while digit(end, radix) {
                end += 1;
            }
            return Some(Number {
                start,
                end,
                radix,
                negative: false,
            });
        }
    }
    let mut end = start;
    while digit(end, 10) {
        end += 1;
    }
    let negative = start > 0 && bytes[start - 1] == b'-';
    Some(Number {
        start: start - usize::from(negative),
        end,
        radix: 10,
        negative,
    })
}

/// `J` (`spaces`) / `gJ`: join `count` lines, at least two.
pub(crate) fn join_lines(editor: &mut Editor, count: Option<usize>, spaces: bool) {
    let first = editor.buf().line_of(editor.head());
    let last_line = editor.buf().last_content_line();
    let mut lines = count.unwrap_or(2).max(2);
    if first + lines - 1 > last_line {
        if count.unwrap_or(0) <= 2 {
            return;
        }
        lines = last_line - first + 1;
    }
    // One edit per line break: (range removed, whether a space replaces it).
    let mut edits: Vec<(Range, bool)> = Vec::with_capacity(lines - 1);
    let head = editor.buf().line_text(first);
    let mut comment = leader_len(head.trim_start_matches([' ', '\t'])).is_some();
    // what the joined line ends with so far, and where the cursor goes
    let mut ends_blank = head.ends_with([' ', '\t']);
    let mut empty = head.is_empty();
    let mut column = head.len();
    let mut length = head.len();
    for line in first + 1..first + lines {
        let text = editor.buf().line_text(line);
        let start = editor.buf().line_start(line);
        let mut skip = 0;
        let mut space = false;
        if spaces {
            let rest = text.trim_start_matches([' ', '\t']);
            skip = text.len() - rest.len();
            let leader = leader_len(rest);
            if let (true, Some(len)) = (comment, leader) {
                skip += len;
            }
            comment = leader.is_some();
            let rest = &text[skip..];
            space = !rest.is_empty() && !rest.starts_with(')') && !empty && !ends_blank;
        }
        let rest = &text[skip..];
        column = length;
        length += usize::from(space) + rest.len();
        if !rest.is_empty() {
            ends_blank = rest.ends_with([' ', '\t']);
            empty = false;
        } else if space {
            ends_blank = true;
        }
        edits.push((
            Range::charwise(editor.buf().line_end(line - 1), start + skip),
            space,
        ));
    }
    let start = editor.buf().line_start(first);
    editor.tx_begin();
    for (range, space) in edits.iter().rev() {
        editor.buf_mut().delete(*range);
        if *space {
            editor.buf_mut().insert(range.start, " ");
        }
    }
    editor.tx_commit();
    editor.set_head(start + column);
    editor.clamp_cursor();
    editor.flash(Range::charwise(editor.head(), editor.head()));
    let key = if spaces { "J" } else { "gJ" };
    record_keys(
        editor,
        match count {
            Some(count) => format!("{count}{key}"),
            None => key.into(),
        },
    );
}

/// `~`: toggle the case of `count` characters and move past them.
pub(crate) fn toggle_case(editor: &mut Editor, count: usize) {
    let line = editor.buf().line_of(editor.head());
    let line_end = editor.buf().line_end(line);
    let start = editor.head();
    if start >= line_end {
        return;
    }
    let mut end = start;
    for _ in 0..count {
        if end >= line_end {
            break;
        }
        end = editor.buf().ceil_boundary(end + 1);
    }
    let text = editor.buf().slice_string(Range::charwise(start, end));
    let toggled: String = text
        .chars()
        .map(|c| {
            let mut other = if c.is_lowercase() {
                c.to_uppercase().collect::<Vec<_>>()
            } else {
                c.to_lowercase().collect::<Vec<_>>()
            };
            if other.len() == 1 {
                other.remove(0)
            } else {
                c
            }
        })
        .collect();
    editor.tx_begin();
    if toggled != text {
        editor.buf_mut().delete(Range::charwise(start, end));
        editor.buf_mut().insert(start, &toggled);
    }
    editor.tx_commit();
    // past the toggled text, not one byte — a multibyte char would
    // otherwise park the cursor mid-char for the next edit
    editor.set_head(start + toggled.len());
    editor.clamp_cursor();
    record_keys(
        editor,
        if count > 1 {
            format!("{count}~")
        } else {
            "~".into()
        },
    );
}

/// The edit that adds `delta` to the number at or after `at` on its
/// line: the number's range and its new text.
pub(crate) fn number_edit(editor: &Editor, at: usize, delta: i64) -> Option<(Range, String)> {
    let line = editor.buf().line_of(at);
    let start = editor.buf().line_start(line);
    let text = editor.buf().line_text(line);
    let number = number_at(&text, at - start)?;
    let written = &text[number.start..number.end];
    let replacement = if number.radix == 10 {
        let digits = written.trim_start_matches('-');
        let value = digits
            .parse::<i64>()
            .map(|value| if number.negative { -value } else { value })
            .unwrap_or(if number.negative { i64::MIN } else { i64::MAX });
        let value = value.saturating_add(delta);
        if digits.len() > 1 && digits.starts_with('0') {
            let width = digits.len();
            format!(
                "{}{:0width$}",
                if value < 0 { "-" } else { "" },
                value.unsigned_abs()
            )
        } else {
            value.to_string()
        }
    } else {
        let digits = &written[2..];
        let value = u64::from_str_radix(digits, number.radix).unwrap_or(u64::MAX);
        let value = value.wrapping_add_signed(delta);
        let width = digits.len();
        let body = match number.radix {
            16 if digits.chars().any(|c| c.is_ascii_uppercase()) => {
                format!("{value:0width$X}")
            }
            16 => format!("{value:0width$x}"),
            _ => format!("{value:0width$b}"),
        };
        format!("{}{body}", &written[..2])
    };
    Some((
        Range::charwise(start + number.start, start + number.end),
        replacement,
    ))
}

/// `Ctrl-A` / `Ctrl-X`: add `delta` to the number at or after the
/// cursor on its line; the cursor lands on the number's last digit.
pub(crate) fn add_to_number(editor: &mut Editor, delta: i64) {
    let Some((range, replacement)) = number_edit(editor, editor.head(), delta) else {
        return;
    };
    editor.tx_begin();
    editor.buf_mut().delete(range);
    editor.buf_mut().insert(range.start, &replacement);
    editor.tx_commit();
    editor.set_head(range.start.get() + replacement.len() - 1);
    editor.flash(Range::charwise(editor.head(), editor.head()));
    let key = if delta < 0 { '\x18' } else { '\x01' };
    record_keys(
        editor,
        match delta.unsigned_abs() {
            1 => key.to_string(),
            count => format!("{count}{key}"),
        },
    );
}

/// `gp` / `gP`: put, and leave the cursor just after the new text.
pub(crate) fn paste_and_pass(editor: &mut Editor, name: Option<char>, count: usize, before: bool) {
    let register = editor.register(name);
    let lines = editor.buf().len_lines();
    let bytes = editor.buf().len_bytes();
    editor.paste_named(name, count, before);
    if editor.buf().len_bytes() == bytes {
        return;
    }
    let after = match register.shape {
        RegisterShape::Linewise => {
            let first = editor.buf().line_of(editor.head());
            let added = editor.buf().len_lines() - lines;
            let line = first + added;
            if line > editor.buf().last_content_line() {
                editor.buf().line_start(editor.buf().last_content_line())
            } else {
                editor.buf().line_start(line)
            }
        }
        _ => editor.buf().ceil_boundary(editor.head() + 1),
    };
    editor.set_head(after);
    editor.clamp_cursor();
    record_keys(editor, if before { "gP" } else { "gp" }.into());
}

/// `]p` / `[p`: put a linewise register with its first line's indent
/// changed to the current line's, the other lines by the same amount.
pub(crate) fn paste_indented(editor: &mut Editor, name: Option<char>, count: usize, before: bool) {
    let mut register = editor.register(name);
    if register.shape == RegisterShape::Linewise {
        let indent_of = |text: &str| text.len() - text.trim_start_matches([' ', '\t']).len();
        let here = editor.buf().line_text(editor.buf().line_of(editor.head()));
        let target = &here[..indent_of(&here)];
        let source = indent_of(register.text.lines().next().unwrap_or(""));
        register.text = register
            .text
            .split_inclusive('\n')
            .map(|line| {
                // the first line's indent becomes the target; deeper
                // lines keep what they had beyond it
                let own = indent_of(line);
                format!("{target}{}", &line[source.min(own)..])
            })
            .collect();
    }
    if register.is_empty() || editor.buf().readonly {
        return;
    }
    editor.paste_register(&register, count, before);
    record_keys(editor, if before { "[p" } else { "]p" }.into());
}

/// A direct (non-grammar) command's dot-repeat record.
pub(crate) fn record_keys(editor: &mut Editor, keys: String) {
    editor.last_cmd_keys = keys;
    editor.last_change = None;
    editor.last_insert = None;
}
