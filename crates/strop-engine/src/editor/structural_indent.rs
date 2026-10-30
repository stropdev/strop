//! Source-aware indentation decisions, independent of input dispatch and syntax
//! ownership. Plans use pre-edit byte coordinates and explicit caret landings.
use super::Editor;
use strop_core::{id::ByteOffset, Buffer};
use strop_syntax::{DelimiterKind, IndentContext, OpeningDelimiter};
mod source;
pub(crate) use source::Batch;

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) enum Trigger {
    Enter,
    Closer(DelimiterKind),
    Below,
    Above,
}

#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub(crate) struct Request {
    pub trigger: Trigger,
    pub syntax_path: Option<std::path::PathBuf>,
    /// Trailing-colon policy from the language registry (0072): one
    /// punctuation rule for Python blocks and C-family labels.
    pub colons: strop_syntax::ColonPolicy,
    pub unit: String,
    pub newline: String,
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub(crate) struct Edit {
    pub cursor: ByteOffset,
    pub start: ByteOffset,
    pub end: ByteOffset,
    pub text: String,
    /// Byte distance from start after replacement, not necessarily text.len().
    pub landing: usize,
}

pub(crate) fn probes(buffer: &Buffer, cursors: &[usize], trigger: Trigger) -> Vec<ByteOffset> {
    let mut positions: Vec<_> = cursors
        .iter()
        .map(|&at| {
            ByteOffset::new(match trigger {
                Trigger::Below => buffer.line_end(buffer.line_of(at)),
                _ => at,
            })
        })
        .collect();
    positions.sort_unstable();
    positions.dedup();
    positions
}

fn whitespace(buffer: &Buffer, start: usize, end: usize) -> usize {
    start
        + buffer
            .text()
            .byte_slice(start..end)
            .bytes()
            .take_while(|byte| matches!(byte, b' ' | b'\t'))
            .count()
}

fn leading(buffer: &Buffer, at: usize) -> String {
    let start = buffer.line_start(buffer.line_of(at));
    let end = whitespace(buffer, start, at);
    buffer.text().byte_slice(start..end).to_string()
}

/// Tiny fast path: only lexically unambiguous prefixes. Quotes, comments,
/// escapes, raw/regex/template/markup syntax all go to the parser owner. This
/// deliberately does not attempt to be a second language lexer.
pub(crate) fn inline_contexts(
    buffer: &Buffer,
    request: &Request,
    positions: &[ByteOffset],
) -> Option<Vec<IndentContext>> {
    const INLINE_BYTES: usize = 4096;
    let limit = positions.last().map_or(0, |at| at.get());
    if limit > INLINE_BYTES || positions.len() > 32 {
        return None;
    }
    if request.syntax_path.is_none() || request.trigger == Trigger::Above {
        return Some(
            positions
                .iter()
                .map(|&cursor| IndentContext {
                    cursor,
                    code: false,
                    opener: None,
                })
                .collect(),
        );
    }
    let mut contexts = Vec::with_capacity(positions.len());
    let mut stack = Vec::new();
    let mut next = 0;
    for (at, byte) in buffer.text().byte_slice(..limit).bytes().enumerate() {
        while next < positions.len() && positions[next].get() == at {
            contexts.push(IndentContext {
                cursor: positions[next],
                code: true,
                opener: stack.last().copied(),
            });
            next += 1;
        }
        match byte {
            b'\'' | b'"' | b'/' | b'#' | b'`' | b'\\' | b'<' => return None,
            b'{' | b'[' | b'(' => {
                if let Some(kind) = DelimiterKind::opening(byte) {
                    stack.push(OpeningDelimiter {
                        position: ByteOffset::new(at),
                        kind,
                    });
                }
            }
            b'}' | b']' | b')' => {
                if stack
                    .last()
                    .is_some_and(|open| open.kind.closes(char::from(byte)))
                {
                    stack.pop();
                } else {
                    stack.clear();
                }
            }
            _ => {}
        }
    }
    for &cursor in &positions[next..] {
        contexts.push(IndentContext {
            cursor,
            code: true,
            opener: stack.last().copied(),
        });
    }
    Some(contexts)
}

pub(crate) fn plan(
    buffer: &Buffer,
    cursors: &[usize],
    request: &Request,
    contexts: &[IndentContext],
) -> Vec<Edit> {
    let mut result = Vec::with_capacity(cursors.len());
    let mut sorted = cursors.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    for cursor in sorted {
        let line = buffer.line_of(cursor);
        let line_start = buffer.line_start(line);
        let line_end = buffer.line_end(line);
        let at = if request.trigger == Trigger::Below {
            line_end
        } else {
            cursor
        };
        let context = contexts
            .binary_search_by_key(&at, |context| context.cursor.get())
            .ok()
            .map(|index| contexts[index]);
        let code = context.is_some_and(|context| context.code);
        let opener = context.and_then(|context| context.opener);
        let mut base = leading(buffer, at);
        let mut start = at;
        let mut end = at;
        let mut text = String::new();
        let landing;
        match request.trigger {
            Trigger::Closer(close) => {
                if code && whitespace(buffer, line_start, at) == at {
                    if let Some(open) = opener.filter(|open| open.kind == close) {
                        base = leading(buffer, open.position.get());
                        start = line_start;
                        text.push_str(&base);
                    }
                }
                text.push(close.closing_character());
                landing = text.len();
            }
            Trigger::Above => {
                start = line_start;
                end = start;
                base = leading(buffer, line_end);
                text.push_str(&base);
                landing = text.len();
                text.push_str(&request.newline);
            }
            Trigger::Enter | Trigger::Below => {
                let mut trimmed = at;
                while trimmed > line_start
                    && matches!(buffer.byte_at(trimmed - 1), Some(b' ' | b'\t'))
                {
                    trimmed -= 1;
                }
                let last = trimmed.checked_sub(1).filter(|&byte| byte >= line_start);
                let open = opener.filter(|open| Some(open.position.get()) == last);
                let colon = match request.colons {
                    strop_syntax::ColonPolicy::None => false,
                    // Python: a block colon only at bracket depth
                    // zero — inside brackets it is a slice or
                    // annotation, not a block.
                    strop_syntax::ColonPolicy::Statement => {
                        opener.is_none() && last.is_some_and(|at| buffer.byte_at(at) == Some(b':'))
                    }
                    // The label colon opens its body from anywhere in
                    // code: `private:`, `case x:`, `default:`, `name:`.
                    strop_syntax::ColonPolicy::Label => {
                        last.is_some_and(|at| buffer.byte_at(at) == Some(b':'))
                    }
                };
                let deepen = code && (open.is_some() || colon);
                text.push_str(&request.newline);
                text.push_str(&base);
                if deepen {
                    text.push_str(&request.unit);
                }
                landing = text.len();
                if deepen && request.trigger == Trigger::Enter {
                    let after = whitespace(buffer, at, line_end);
                    if open.is_some_and(|open| {
                        buffer
                            .byte_at(after)
                            .is_some_and(|close| open.kind.closes(char::from(close)))
                    }) {
                        end = after;
                        text.push_str(&request.newline);
                        text.push_str(&base);
                    }
                }
            }
        }
        result.push(Edit {
            cursor: ByteOffset::new(cursor),
            start: ByteOffset::new(start),
            end: ByteOffset::new(end),
            text,
            landing,
        });
    }
    result
}

impl Editor {
    pub(crate) fn closer_needs_alignment(&self) -> bool {
        self.all_cursors().into_iter().any(|at| {
            let start = self.buf().line_start(self.buf().line_of(at));
            self.buf()
                .text()
                .byte_slice(start..at)
                .bytes()
                .take(4096)
                .all(|byte| matches!(byte, b' ' | b'\t'))
        })
    }
    pub(crate) fn apply_indentation(&mut self, edits: &[Edit]) {
        let map = |old: usize| {
            let mut delta = 0isize;
            for edit in edits {
                if old == edit.cursor.get() {
                    return (edit.start.get() as isize + delta + edit.landing as isize).max(0)
                        as usize;
                }
                if old < edit.start.get() {
                    break;
                }
                if old < edit.end.get() {
                    return (edit.start.get() as isize + delta + edit.text.len() as isize).max(0)
                        as usize;
                }
                delta += edit.text.len() as isize - (edit.end.get() - edit.start.get()) as isize;
            }
            (old as isize + delta).max(0) as usize
        };
        self.sels_mut().map_positions(map);
        if self.snippet.is_some() {
            self.snippet_map(map);
        }
        for edit in edits.iter().rev() {
            if edit.start != edit.end {
                self.buf_mut()
                    .delete(strop_core::Range::charwise(edit.start, edit.end));
            }
            self.buf_mut().insert(edit.start, &edit.text);
        }
    }
}
