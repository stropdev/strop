//! Structural indentation reads on the parser's CPU owner. No text copies,
//! highlight-query execution, or language-independent string/comment lexer.
use crate::{HighlightError, Highlighter};
use ropey::Rope;
use strop_core::id::{BufferRevision, ByteOffset};

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum DelimiterKind {
    Brace,
    Bracket,
    Parenthesis,
}

impl DelimiterKind {
    pub fn opening(byte: u8) -> Option<Self> {
        match byte {
            b'{' => Some(Self::Brace),
            b'[' => Some(Self::Bracket),
            b'(' => Some(Self::Parenthesis),
            _ => None,
        }
    }
    pub fn closes(self, character: char) -> bool {
        matches!(
            (self, character),
            (Self::Brace, '}') | (Self::Bracket, ']') | (Self::Parenthesis, ')')
        )
    }
    pub fn closing(character: char) -> Option<Self> {
        match character {
            '}' => Some(Self::Brace),
            ']' => Some(Self::Bracket),
            ')' => Some(Self::Parenthesis),
            _ => None,
        }
    }
    pub fn closing_character(self) -> char {
        match self {
            Self::Brace => '}',
            Self::Bracket => ']',
            Self::Parenthesis => ')',
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct OpeningDelimiter {
    pub position: ByteOffset,
    pub kind: DelimiterKind,
}

#[derive(Debug, Clone, Copy, serde::Serialize, serde::Deserialize)]
pub struct IndentContext {
    pub cursor: ByteOffset,
    pub code: bool,
    pub opener: Option<OpeningDelimiter>,
}

fn opaque(kind: &str) -> bool {
    kind.contains("comment")
        || kind.contains("string")
        || kind.contains("char_literal")
        || kind.contains("character_literal")
        || matches!(kind, "regex" | "regex_pattern" | "heredoc_body")
}

/// Recovery can retain the lexical pieces of a literal without its enclosing
/// completed syntax node. These fragments do not establish a closed extent.
fn literal_fragment(kind: &str) -> bool {
    (kind.contains("string")
        && (kind.ends_with("_start")
            || kind.ends_with("_end")
            || kind.ends_with("_close")
            || kind.contains("content")
            || kind.contains("fragment")))
        || matches!(kind, "template_chars" | "heredoc_body")
}

fn advance(cursor: &mut tree_sitter::TreeCursor<'_>) -> bool {
    loop {
        if cursor.goto_next_sibling() {
            return true;
        }
        if !cursor.goto_parent() {
            return false;
        }
    }
}

/// Recovery can hide a literal's opening token. For example, Rust's raw
/// string start is an invisible external token and its content scanner emits
/// no token at EOF without a terminator. Examine only uncovered/error bytes;
/// complete opaque syntax nodes remain authoritative and are skipped.
fn malformed_literal(
    node: tree_sitter::Node<'_>,
    rope: &Rope,
    cancelled: &impl Fn() -> bool,
) -> Result<bool, HighlightError> {
    if node.kind() != "ERROR" {
        return Ok(false);
    }
    let uncertain = |start: usize, end: usize| -> Result<bool, HighlightError> {
        for (index, byte) in rope.byte_slice(start..end).bytes().enumerate() {
            if index % 1024 == 0 && cancelled() {
                return Err(HighlightError::Cancelled);
            }
            if matches!(byte, b'"' | b'\'' | b'`') {
                return Ok(true);
            }
        }
        Ok(false)
    };
    let mut cursor = node.walk();
    let mut covered = node.start_byte();
    loop {
        if cancelled() {
            return Err(HighlightError::Cancelled);
        }
        let candidate = cursor.node();
        if candidate.start_byte() > covered && uncertain(covered, candidate.start_byte())? {
            return Ok(true);
        }
        if literal_fragment(candidate.kind()) {
            return Ok(true);
        }
        if opaque(candidate.kind()) && !candidate.has_error() {
            covered = covered.max(candidate.end_byte());
        } else if cursor.goto_first_child() {
            continue;
        } else {
            if candidate.end_byte() > covered
                && uncertain(covered.max(candidate.start_byte()), candidate.end_byte())?
            {
                return Ok(true);
            }
            if candidate.kind() == "/*" {
                return Ok(true);
            }
            covered = covered.max(candidate.end_byte());
        }
        if !advance(&mut cursor) {
            return uncertain(covered, node.end_byte());
        }
    }
}

impl Highlighter {
    /// Resolve sorted source-byte cursors in one tree walk. Opaque nodes are
    /// skipped wholesale, including interpolation: indentation never rewrites
    /// literal content merely because it happens to contain a brace.
    pub fn indentation_contexts(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        positions: &[ByteOffset],
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<IndentContext>, HighlightError> {
        self.ensure_tree(rope, revision, &cancelled)?;
        let tree = self.tree.as_ref().ok_or(HighlightError::Parse)?;
        let mut walk = tree.walk();
        let mut stack = Vec::new();
        let mut result = Vec::with_capacity(positions.len());
        let mut next = 0;
        while next < positions.len() {
            if cancelled() {
                return Err(HighlightError::Cancelled);
            }
            let node = walk.node();
            while next < positions.len() && positions[next].get() <= node.start_byte() {
                result.push(IndentContext {
                    cursor: positions[next],
                    code: true,
                    opener: stack.last().copied(),
                });
                next += 1;
            }
            if next == positions.len() {
                break;
            }
            if malformed_literal(node, rope, &cancelled)? {
                // Recovery may parse the unterminated literal's remaining
                // bytes as sibling code nodes. Without a recognized closing
                // token their lexical extent is unknown, including at EOF.
                result.extend(positions[next..].iter().map(|&cursor| IndentContext {
                    cursor,
                    code: false,
                    opener: None,
                }));
                return Ok(result);
            }
            let hidden = opaque(node.kind());
            if !hidden && walk.goto_first_child() {
                continue;
            }
            if !hidden && node.end_byte() == node.start_byte() + 1 && !node.is_missing() {
                let byte = rope.byte(node.start_byte());
                match byte {
                    b'{' | b'[' | b'(' => {
                        if let Some(kind) = DelimiterKind::opening(byte) {
                            stack.push(OpeningDelimiter {
                                position: ByteOffset::new(node.start_byte()),
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
                            // An unmatched/misnested closer is not a safe anchor.
                            stack.clear();
                        }
                    }
                    _ => {}
                }
            }
            while next < positions.len() && positions[next].get() <= node.end_byte() {
                let at_end = positions[next].get() == node.end_byte();
                let line_comment = node.kind().contains("comment")
                    && rope
                        .byte_slice(node.start_byte()..node.end_byte())
                        .bytes()
                        .take(2)
                        .eq(b"//".iter().copied());
                let hash_comment = node.kind().contains("comment")
                    && rope.get_byte(node.start_byte()) == Some(b'#');
                let unfinished_comment = node.kind().contains("comment")
                    && !line_comment
                    && !hash_comment
                    && (node.end_byte() < 2
                        || rope
                            .byte_slice(node.end_byte() - 2..node.end_byte())
                            .bytes()
                            .ne(b"*/".iter().copied()));
                let content = node.kind().contains("content") || node.kind().contains("body");
                result.push(IndentContext {
                    cursor: positions[next],
                    code: !hidden
                        || (at_end
                            && !node.has_error()
                            && !line_comment
                            && !hash_comment
                            && !unfinished_comment
                            && !content),
                    opener: stack.last().copied(),
                });
                next += 1;
            }
            if !advance(&mut walk) {
                break;
            }
        }
        for &cursor in &positions[next..] {
            result.push(IndentContext {
                cursor,
                code: true,
                opener: stack.last().copied(),
            });
        }
        Ok(result)
    }
}
