//! Query-selected documentation containers, expressed in host-rope byte ranges.
use super::HighlightError;
use ropey::Rope;
use tree_sitter::{Node, Point, Range};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Policy {
    RustLine,
    Block,
    GoBlock,
    Python,
}

/// Only repeated captures from the same query pattern share an owning group.
/// Other patterns may select the same bytes with a different extraction policy.
pub(super) struct Group {
    pattern: usize,
    start: usize,
    end: usize,
}

impl Policy {
    pub(super) fn from_property(value: Option<&str>) -> Option<Self> {
        match value {
            Some("rust-line") => Some(Self::RustLine),
            Some("block") => Some(Self::Block),
            Some("python") => Some(Self::Python),
            Some("go-block") => Some(Self::GoBlock),
            _ => None,
        }
    }

    pub(super) fn ranges(
        self,
        rope: &Rope,
        node: Node<'_>,
        pattern: usize,
        seen: &mut Vec<Group>,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Range>, HighlightError> {
        if seen.iter().any(|group| {
            group.pattern == pattern
                && group.start <= node.start_byte()
                && node.end_byte() <= group.end
        }) {
            return Ok(Vec::new());
        }
        let mut ranges = Vec::new();
        match self {
            Self::RustLine => {
                let marker = rope.byte(node.start_byte() + 2);
                let same_kind = |other: Node<'_>| {
                    other.kind() == "line_comment"
                        && other.child_by_field_name("doc").is_some()
                        && rope.byte(other.start_byte() + 2) == marker
                };
                let mut first = node;
                while let Some(previous) = first.prev_named_sibling() {
                    if cancelled() {
                        return Err(HighlightError::Cancelled);
                    }
                    if !same_kind(previous)
                        || previous.start_position().row + 1 != first.start_position().row
                        || !whitespace_between(rope, previous.end_byte(), first.start_byte())
                    {
                        break;
                    }
                    first = previous;
                }
                let mut current = first;
                loop {
                    if cancelled() {
                        return Err(HighlightError::Cancelled);
                    }
                    let mut start = current.start_byte() + 3;
                    let mut end = current.end_byte();
                    if start < end && rope.byte(start) == b' ' {
                        start += 1;
                    }
                    // Some grammar versions include the newline in the doc node,
                    // others leave it between comments. Keep exactly one newline.
                    if end < rope.len_bytes() && rope.byte(end) == b'\r' {
                        end += 1;
                    }
                    if end < rope.len_bytes() && rope.byte(end) == b'\n' {
                        end += 1;
                    }
                    push(rope, start, end, &mut ranges);
                    let Some(next) = current.next_named_sibling() else {
                        break;
                    };
                    if !same_kind(next)
                        || current.start_position().row + 1 != next.start_position().row
                        || !whitespace_between(rope, current.end_byte(), next.start_byte())
                    {
                        break;
                    }
                    current = next;
                }
                seen.push(Group {
                    pattern,
                    start: first.start_byte(),
                    end: current.end_byte(),
                });
            }
            Self::Block | Self::GoBlock => {
                if self == Self::GoBlock
                    && !node.next_named_sibling().is_some_and(|next| {
                        next.start_position().row <= node.end_position().row + 1
                    })
                {
                    return Ok(ranges);
                }
                let mut start = node.start_byte() + 2;
                let mut end = node.end_byte();
                if end >= start + 2 && rope.byte(end - 2) == b'*' && rope.byte(end - 1) == b'/' {
                    end -= 2;
                }
                if start < end && matches!(rope.byte(start), b'*' | b'!') {
                    start += 1;
                }
                if start < end && rope.byte(start) == b' ' {
                    start += 1;
                }
                let mut offset = start;
                for (index, line) in rope.byte_slice(start..end).lines().enumerate() {
                    if cancelled() {
                        return Err(HighlightError::Cancelled);
                    }
                    let mut skip = 0;
                    if index > 0 {
                        let padding = line
                            .bytes()
                            .take_while(|byte| matches!(byte, b' ' | b'\t'))
                            .count();
                        if padding < line.len_bytes() && line.byte(padding) == b'*' {
                            skip = padding + 1;
                            if skip < line.len_bytes() && line.byte(skip) == b' ' {
                                skip += 1;
                            }
                        }
                    }
                    push(rope, offset + skip, offset + line.len_bytes(), &mut ranges);
                    offset += line.len_bytes();
                }
                seen.push(Group {
                    pattern,
                    start: node.start_byte(),
                    end: node.end_byte(),
                });
            }
            Self::Python => {
                let Some(open) = node.named_child(0) else {
                    return Ok(ranges);
                };
                let Some(close) = node.named_child(node.named_child_count().saturating_sub(1))
                else {
                    return Ok(ranges);
                };
                let prefix = rope.byte_slice(open.start_byte()..open.end_byte());
                if open.kind() != "string_start"
                    || close.kind() != "string_end"
                    || ![
                        "\"\"\"", "'''", "r\"\"\"", "r'''", "R\"\"\"", "R'''", "u\"\"\"", "u'''",
                        "U\"\"\"", "U'''",
                    ]
                    .iter()
                    .any(|allowed| prefix == *allowed)
                    || close.end_byte() - close.start_byte() != 3
                {
                    return Ok(ranges);
                }
                let start = open.end_byte();
                let end = close.start_byte();
                let mut indent = usize::MAX;
                for line in rope.byte_slice(start..end).lines().skip(1) {
                    if cancelled() {
                        return Err(HighlightError::Cancelled);
                    }
                    let padding = line
                        .bytes()
                        .take_while(|byte| matches!(byte, b' ' | b'\t'))
                        .count();
                    if line
                        .bytes()
                        .skip(padding)
                        .any(|byte| !matches!(byte, b'\r' | b'\n'))
                    {
                        indent = indent.min(padding);
                    }
                }
                let mut offset = start;
                for (index, line) in rope.byte_slice(start..end).lines().enumerate() {
                    if cancelled() {
                        return Err(HighlightError::Cancelled);
                    }
                    let padding = if index == 0 {
                        0
                    } else {
                        line.bytes()
                            .take_while(|byte| matches!(byte, b' ' | b'\t'))
                            .count()
                            .min(indent)
                    };
                    push(
                        rope,
                        offset + padding,
                        offset + line.len_bytes(),
                        &mut ranges,
                    );
                    offset += line.len_bytes();
                }
                seen.push(Group {
                    pattern,
                    start: node.start_byte(),
                    end: node.end_byte(),
                });
            }
        }
        Ok(ranges)
    }
}

fn whitespace_between(rope: &Rope, start: usize, end: usize) -> bool {
    rope.byte_slice(start..end)
        .bytes()
        .all(|byte| byte.is_ascii_whitespace())
}

fn push(rope: &Rope, start: usize, end: usize, ranges: &mut Vec<Range>) {
    if start >= end {
        return;
    }
    let point = |byte| {
        let row = rope.byte_to_line(byte);
        Point {
            row,
            column: byte - rope.line_to_byte(row),
        }
    };
    ranges.push(Range {
        start_byte: start,
        end_byte: end,
        start_point: point(start),
        end_point: point(end),
    });
}
