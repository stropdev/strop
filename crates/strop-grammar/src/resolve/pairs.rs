//! Shared `%`/passive-overlay delimiter matching. Vim's default matchpairs
//! exclude angles, balance only the same kind, isolate balanced quoted text,
//! skip character literals from code, and require matching escape parity.
use std::ops::Range;
use strop_core::Buffer;

#[derive(Debug)]
pub struct MatchCancelled;

pub fn delimiter_pair(byte: u8) -> Option<(u8, u8)> {
    match byte {
        b'(' | b')' => Some((b'(', b')')),
        b'[' | b']' => Some((b'[', b']')),
        b'{' | b'}' => Some((b'{', b'}')),
        _ => None,
    }
}

fn escaped(line: ropey::RopeSlice<'_>, at: usize) -> bool {
    let mut before = at;
    while before > 0 && line.byte(before - 1) == b'\\' {
        before -= 1;
    }
    !(at - before).is_multiple_of(2)
}

fn quotes(
    line: ropey::RopeSlice<'_>,
    cancelled: &impl Fn() -> bool,
) -> Result<Vec<Range<usize>>, MatchCancelled> {
    let mut regions = Vec::new();
    let mut opening = None;
    let mut escape = false;
    let mut character_end = 0;
    for (at, byte) in line.bytes().enumerate() {
        if at % 1024 == 0 && cancelled() {
            return Err(MatchCancelled);
        }
        if at < character_end {
            continue;
        }
        if escape {
            escape = false;
            continue;
        }
        if byte == b'\\' {
            escape = true;
            continue;
        }
        if byte == b'"' {
            if let Some(start) = opening.take() {
                regions.push(start..at + 1);
            } else {
                opening = Some(at);
            }
        } else if byte == b'\'' && opening.is_none() && at + 1 < line.len_bytes() {
            let mut chars = line.byte_slice(at + 1..).chars();
            if let Some(first) = chars.next() {
                let mut end = at + 1 + first.len_utf8();
                if first == '\\' {
                    end += chars.next().map_or(0, char::len_utf8);
                }
                if end < line.len_bytes() && line.byte(end) == b'\'' {
                    character_end = end + 1;
                    regions.push(at..character_end);
                }
            }
        }
    }
    // An unmatched quote does not hide the rest of a line in Vim's matcher.
    Ok(regions)
}

/// Match exactly the delimiter at `from`, without `%`'s initial line search.
/// Cancellation is checked during quote classification as well as scanning.
pub fn matching_delimiter_at(
    buf: &Buffer,
    from: usize,
    cancelled: impl Fn() -> bool,
) -> Result<Option<usize>, MatchCancelled> {
    if cancelled() {
        return Err(MatchCancelled);
    }
    let Some(byte) = buf.byte_at(from) else {
        return Ok(None);
    };
    let Some((open, close)) = delimiter_pair(byte) else {
        return Ok(None);
    };
    let forward = byte == open;
    let probe_line = buf.line_of(from);
    let probe_column = from - buf.line_start(probe_line);
    let probe_text = buf.text().line(probe_line);
    let parity = escaped(probe_text, probe_column);
    let mut probe_quotes = quotes(probe_text, &cancelled)?;
    let scope = probe_quotes
        .iter()
        .find(|range| range.contains(&probe_column))
        .cloned();
    let mut line_number = probe_line;
    let mut depth = 0usize;
    loop {
        let line = buf.text().line(line_number);
        let quoted = if line_number == probe_line {
            std::mem::take(&mut probe_quotes)
        } else {
            quotes(line, &cancelled)?
        };
        let mut column = if forward {
            if line_number == probe_line {
                probe_column + 1
            } else {
                0
            }
        } else if line_number == probe_line {
            probe_column
        } else {
            line.len_bytes()
        };
        loop {
            if forward {
                if column >= line.len_bytes() {
                    break;
                }
            } else {
                let Some(previous) = column.checked_sub(1) else {
                    break;
                };
                column = previous;
            }
            if column % 1024 == 0 && cancelled() {
                return Err(MatchCancelled);
            }
            let candidate = line.byte(column);
            if candidate == open || candidate == close {
                let region = quoted.partition_point(|range| range.end <= column);
                let inside = quoted.get(region).filter(|range| range.contains(&column));
                let same_scope = match &scope {
                    Some(scope) => line_number == probe_line && inside == Some(scope),
                    None => inside.is_none(),
                };
                if same_scope && escaped(line, column) == parity {
                    let opens = candidate == if forward { open } else { close };
                    if opens {
                        depth += 1;
                    } else if depth == 0 {
                        return Ok(Some(buf.line_start(line_number) + column));
                    } else {
                        depth -= 1;
                    }
                }
            }
            if forward {
                column += 1;
            }
        }
        if scope.is_some() {
            return Ok(None);
        }
        if forward {
            line_number += 1;
            if line_number >= buf.len_lines() {
                return Ok(None);
            }
        } else {
            let Some(previous) = line_number.checked_sub(1) else {
                return Ok(None);
            };
            line_number = previous;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn neovim_quote_comment_and_escape_witnesses() {
        for (text, from, expected) in [
            ("x < a >", 2, None),
            ("{ let s = \"}\"; }", 0, Some(15)),
            ("let s = \"{ x }\";", 9, Some(13)),
            ("let s = \"{\"; }", 9, None),
            ("{ let c = '}'; }", 0, Some(15)),
            ("/* { */ int x; /* } */", 3, Some(18)),
            ("{ \\} }", 0, Some(5)),
            ("\\{ x }", 1, None),
            ("{ \"unterminated }", 0, Some(16)),
        ] {
            let buffer = Buffer::from_text(text);
            assert_eq!(
                matching_delimiter_at(&buffer, from, || false).unwrap(),
                expected,
                "{text}"
            );
            if let Some(mate) = expected {
                assert_eq!(
                    matching_delimiter_at(&buffer, mate, || false).unwrap(),
                    Some(from),
                    "reverse: {text}"
                );
            }
        }
    }
}
