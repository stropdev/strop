//! Structural indentation reads the document's one syntax tree, from the
//! cursor outward (#309): the path from the root to the cursor, and on that
//! path each node's own bracket tokens before the cursor. No text copies,
//! highlight-query execution, or language-independent string/comment lexer.
use crate::{languages::LanguageId, HighlightError, Highlighter};
use ropey::Rope;
use strop_core::id::{BufferRevision, ByteOffset};
use tree_sitter::Node;

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

/// Work one input-path lookup may do, in nodes and 1 KiB byte runs
/// examined. Past it the key goes to the tree's owner, which looks with no
/// bound.
pub const INPUT_BUDGET: usize = 4096;

/// Children of one node examined before the cursor: this many from its
/// start, where its opening bracket is, and this many just before the
/// cursor. The run between (a file's items, a block's statements) is
/// complete syntax whose brackets close inside it.
const WINDOW: usize = 32;

/// The tree the analysis owner keeps for one document, at the revision it
/// describes. Cloning is a reference count; a lookup on it never parses.
#[derive(Clone)]
pub struct SyntaxTree {
    revision: BufferRevision,
    language: LanguageId,
    tree: tree_sitter::Tree,
}

impl std::fmt::Debug for SyntaxTree {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SyntaxTree")
            .field("revision", &self.revision)
            .field("language", &self.language)
            .finish_non_exhaustive()
    }
}

impl SyntaxTree {
    pub fn revision(&self) -> BufferRevision {
        self.revision
    }
    pub fn language(&self) -> LanguageId {
        self.language
    }

    /// Indent contexts for sorted positions within `budget`, or `None`
    /// past it. `rope` must be the text at [`Self::revision`].
    pub fn indentation_contexts(
        &self,
        rope: &Rope,
        positions: &[ByteOffset],
        budget: usize,
    ) -> Option<Vec<IndentContext>> {
        contexts(&self.tree, rope, positions, Some(budget), &|| false).ok()
    }
}

impl Highlighter {
    /// The kept tree, when it describes the last text parsed.
    pub fn syntax_tree(&self) -> Option<SyntaxTree> {
        let tree = self.tree.as_ref()?;
        (self.source_hash == Some(self.tree_revision)).then(|| SyntaxTree {
            revision: self.tree_revision,
            language: self.language,
            tree: tree.clone(),
        })
    }

    /// The owner's lookup: parse up to `revision` (incrementally against
    /// the kept tree), then look with no work bound. Text past
    /// [`crate::PARSE_LIMIT`] is not parsed: every position is outside
    /// code, so it indents from the previous line.
    pub fn indentation_contexts(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        positions: &[ByteOffset],
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<IndentContext>, HighlightError> {
        if rope.len_bytes() > crate::PARSE_LIMIT {
            self.tree = None;
            self.source_hash = None;
            return Ok(positions
                .iter()
                .map(|&cursor| IndentContext {
                    cursor,
                    code: false,
                    opener: None,
                })
                .collect());
        }
        self.ensure_tree(rope, revision, &cancelled)?;
        let tree = self.tree.as_ref().ok_or(HighlightError::Parse)?;
        contexts(tree, rope, positions, None, &cancelled).map_err(|stop| match stop {
            Stop::Cancelled => HighlightError::Cancelled,
            // an unbounded probe has no budget to run out of
            Stop::Budget => HighlightError::Parse,
        })
    }
}

enum Stop {
    Budget,
    Cancelled,
}

struct Probe<'a> {
    rope: &'a Rope,
    budget: Option<usize>,
    cancelled: &'a dyn Fn() -> bool,
}

impl Probe<'_> {
    fn tick(&mut self) -> Result<(), Stop> {
        if let Some(budget) = &mut self.budget {
            *budget = budget.checked_sub(1).ok_or(Stop::Budget)?;
        }
        if (self.cancelled)() {
            return Err(Stop::Cancelled);
        }
        Ok(())
    }

    /// A one-byte bracket token outside literals and comments.
    fn bracket(&self, node: Node<'_>, stack: &mut Vec<OpeningDelimiter>) {
        if node.end_byte() != node.start_byte() + 1 || node.is_missing() {
            return;
        }
        let byte = self.rope.byte(node.start_byte());
        if let Some(kind) = DelimiterKind::opening(byte) {
            stack.push(OpeningDelimiter {
                position: ByteOffset::new(node.start_byte()),
                kind,
            });
        } else if matches!(byte, b'}' | b']' | b')') {
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
    }

    /// Recovery can hide a literal's opening token. For example, Rust's raw
    /// string start is an invisible external token and its content scanner
    /// emits no token at EOF without a terminator. Examine only
    /// uncovered/error bytes; complete opaque syntax nodes remain
    /// authoritative and are skipped.
    fn malformed_literal(&mut self, node: Node<'_>) -> Result<bool, Stop> {
        if node.kind() != "ERROR" {
            return Ok(false);
        }
        let mut cursor = node.walk();
        let mut covered = node.start_byte();
        loop {
            self.tick()?;
            let candidate = cursor.node();
            if candidate.start_byte() > covered
                && self.uncertain(covered, candidate.start_byte())?
            {
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
                    && self.uncertain(covered.max(candidate.start_byte()), candidate.end_byte())?
                {
                    return Ok(true);
                }
                if candidate.kind() == "/*" {
                    return Ok(true);
                }
                covered = covered.max(candidate.end_byte());
            }
            if !next_in_subtree(&mut cursor) {
                return self.uncertain(covered, node.end_byte());
            }
        }
    }

    /// Whether uncovered bytes hold a quote that may open a literal.
    fn uncertain(&mut self, start: usize, end: usize) -> Result<bool, Stop> {
        for (index, byte) in self.rope.byte_slice(start..end).bytes().enumerate() {
            if index % 1024 == 0 {
                self.tick()?;
            }
            if matches!(byte, b'"' | b'\'' | b'`') {
                return Ok(true);
            }
        }
        Ok(false)
    }

    /// Every bracket of a subtree that recovery left incomplete, in order.
    /// `Ok(true)`: it holds a malformed literal.
    fn recovered(
        &mut self,
        node: Node<'_>,
        stack: &mut Vec<OpeningDelimiter>,
    ) -> Result<bool, Stop> {
        let mut cursor = node.walk();
        loop {
            self.tick()?;
            let current = cursor.node();
            if self.malformed_literal(current)? {
                return Ok(true);
            }
            let hidden = opaque(current.kind());
            if !hidden && cursor.goto_first_child() {
                continue;
            }
            if !hidden {
                self.bracket(current, stack);
            }
            if !next_in_subtree(&mut cursor) {
                return Ok(false);
            }
        }
    }

    /// The children of `parent` before index `end`, in order. Complete
    /// subtrees close their own brackets and are passed over; leaves are
    /// bracket tokens; subtrees with errors are read in full.
    fn preceding(
        &mut self,
        parent: Node<'_>,
        end: usize,
        stack: &mut Vec<OpeningDelimiter>,
    ) -> Result<bool, Stop> {
        let head = if end > 2 * WINDOW { WINDOW } else { end };
        let mut cursor = parent.walk();
        let mut more = cursor.goto_first_child();
        let mut index = 0;
        while more && index < head {
            if self.sibling(cursor.node(), stack)? {
                return Ok(true);
            }
            more = cursor.goto_next_sibling();
            index += 1;
        }
        let mut index = end.saturating_sub(WINDOW).max(head);
        while index < end {
            let Some(child) = parent.child(index) else {
                break;
            };
            if self.sibling(child, stack)? {
                return Ok(true);
            }
            index += 1;
        }
        Ok(false)
    }

    fn sibling(&mut self, node: Node<'_>, stack: &mut Vec<OpeningDelimiter>) -> Result<bool, Stop> {
        self.tick()?;
        if opaque(node.kind()) {
            return Ok(false);
        }
        if node.has_error() {
            return self.recovered(node, stack);
        }
        if node.child_count() == 0 {
            self.bracket(node, stack);
        }
        Ok(false)
    }

    /// One position: descend from the root to the node that holds it.
    fn context(&mut self, root: Node<'_>, at: usize) -> Result<IndentContext, Stop> {
        let cursor = ByteOffset::new(at);
        let malformed = IndentContext {
            cursor,
            code: false,
            opener: None,
        };
        let mut stack = Vec::new();
        if at <= root.start_byte() {
            return Ok(IndentContext {
                cursor,
                code: true,
                opener: None,
            });
        }
        let mut walk = root.walk();
        loop {
            self.tick()?;
            let node = walk.node();
            if self.malformed_literal(node)? {
                // Recovery may parse the unterminated literal's remaining
                // bytes as sibling code nodes. Without a recognized closing
                // token their lexical extent is unknown, including at EOF.
                return Ok(malformed);
            }
            let hidden = opaque(node.kind());
            if hidden || node.child_count() == 0 {
                if !hidden {
                    self.bracket(node, &mut stack);
                }
                return Ok(IndentContext {
                    cursor,
                    code: !hidden || self.closed_before(node, at),
                    opener: stack.last().copied(),
                });
            }
            let child = self.first_reaching(&mut walk, at)?;
            if self.preceding(node, child.unwrap_or(node.child_count()), &mut stack)? {
                return Ok(malformed);
            }
            if child.is_none() || walk.node().start_byte() >= at {
                return Ok(IndentContext {
                    cursor,
                    code: true,
                    opener: stack.last().copied(),
                });
            }
        }
    }

    /// Move to the first child that ends at or after `at` (which is past
    /// the node's start), returning its index. tree-sitter's lookup gives
    /// up when a hidden node ends just past its goal on a hidden token (Go's
    /// statement newlines): retry one byte on, then walk the children.
    fn first_reaching(
        &mut self,
        walk: &mut tree_sitter::TreeCursor<'_>,
        at: usize,
    ) -> Result<Option<usize>, Stop> {
        for goal in [at - 1, at] {
            if let Some(index) = walk.goto_first_child_for_byte(goal) {
                return Ok(Some(index));
            }
        }
        if !walk.goto_first_child() {
            return Ok(None);
        }
        let mut index = 0;
        loop {
            self.tick()?;
            if walk.node().end_byte() >= at {
                return Ok(Some(index));
            }
            if !walk.goto_next_sibling() {
                walk.goto_parent();
                return Ok(None);
            }
            index += 1;
        }
    }

    /// A cursor at the very end of a complete literal or block comment is
    /// back in code; at the end of a line comment, an unterminated comment
    /// or a literal's content it is not.
    fn closed_before(&self, node: Node<'_>, at: usize) -> bool {
        let comment = node.kind().contains("comment");
        let line_comment = comment
            && self
                .rope
                .byte_slice(node.start_byte()..node.end_byte())
                .bytes()
                .take(2)
                .eq(b"//".iter().copied());
        let hash_comment = comment && self.rope.get_byte(node.start_byte()) == Some(b'#');
        let unfinished_comment = comment
            && !line_comment
            && !hash_comment
            && (node.end_byte() < 2
                || self
                    .rope
                    .byte_slice(node.end_byte() - 2..node.end_byte())
                    .bytes()
                    .ne(b"*/".iter().copied()));
        let content = node.kind().contains("content") || node.kind().contains("body");
        at == node.end_byte()
            && !node.has_error()
            && !line_comment
            && !hash_comment
            && !unfinished_comment
            && !content
    }
}

/// Pre-order step that never leaves the cursor's own subtree.
fn next_in_subtree(cursor: &mut tree_sitter::TreeCursor<'_>) -> bool {
    loop {
        if cursor.goto_next_sibling() {
            return true;
        }
        if !cursor.goto_parent() {
            return false;
        }
    }
}

/// Opaque nodes are skipped wholesale, including interpolation:
/// indentation never rewrites literal content merely because it happens to
/// contain a brace.
fn contexts(
    tree: &tree_sitter::Tree,
    rope: &Rope,
    positions: &[ByteOffset],
    budget: Option<usize>,
    cancelled: &dyn Fn() -> bool,
) -> Result<Vec<IndentContext>, Stop> {
    let mut probe = Probe {
        rope,
        budget,
        cancelled,
    };
    let root = tree.root_node();
    let mut result = Vec::with_capacity(positions.len());
    for position in positions {
        result.push(probe.context(root, position.get())?);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
