//! strop-syntax: tree-sitter highlighting. Parsers statically linked
//! (0002 §2.2 — never dlopen'd grammars); queries are data (0001 §5.11),
//! embedded defaults now, runtime overrides when config lands (0005).
//!
//! The whole pipeline is rope-backed: parsing walks rope chunks, and
//! query predicates (`#eq?`/`#match?`/…) read node text through a
//! [`tree_sitter::TextProvider`] over the same chunks — no full-text
//! `String` is materialized on any input or render path, and language
//! detection never touches the filesystem.

use streaming_iterator::StreamingIterator;
mod guides;
pub mod languages;
pub use guides::{GuideFrame, IndentGuides};
mod injections;
mod spans;
pub use spans::Emphasis;
use spans::{CaptureStyle, LayeredSpan};

use ropey::Rope;
use strop_core::id::BufferRevision;
use tree_sitter::{Parser, Query, QueryCursor, TextProvider};

/// Semantic classes the renderer maps to palette colors. Kept small and
/// stable; the query capture names map onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, serde::Serialize, serde::Deserialize)]
pub enum Class {
    Keyword,
    Function,
    Type,
    String,
    Comment,
    Number,
    Operator,
    Punctuation,
    Constant,
    Variable,
    Attribute,
    Heading,
    Link,
    Code,
    Quote,
    List,
    Tag,
}

impl Class {
    fn from_capture(name: &str) -> Self {
        if name.starts_with("constant.numeric.") {
            return Class::Number;
        }
        if let Some(markup) = name.strip_prefix("markup.") {
            return match markup.split('.').next() {
                Some("heading") => Class::Heading,
                Some("link") => Class::Link,
                Some("raw") => Class::Code,
                Some("quote") => Class::Quote,
                Some("list") => Class::List,
                _ => Class::Variable,
            };
        }
        let head = name.split('.').next().unwrap_or(name);
        match head {
            "keyword" => Class::Keyword,
            "function" | "constructor" => Class::Function,
            "type" | "namespace" | "label" => Class::Type,
            "string" | "character" => Class::String,
            "comment" => Class::Comment,
            "number" | "float" => Class::Number,
            "operator" => Class::Operator,
            "punctuation" => Class::Punctuation,
            "constant" | "boolean" => Class::Constant,
            "attribute" | "property" => Class::Attribute,
            "tag" => Class::Tag,
            _ => Class::Variable,
        }
    }
}

/// A colored span, in byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: Class,
    pub emphasis: Emphasis,
}

/// Highlighting failed out loud: no hidden full-text reparse fallback, no
/// swallowed error, no panic — the caller decides what the failure means
/// for its surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightError {
    /// tree-sitter refused the chunked input (encoding breach or an
    /// internal limit). Cached spans and the kept tree stay untouched, so
    /// the next call retries from the same state.
    Parse,
    /// Superseded analysis is not a parser failure.
    Cancelled,
    /// Recursive injected language structure exceeded the owned parser bound.
    InjectionDepth,
}

impl std::fmt::Display for HighlightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HighlightError::Parse => f.write_str("tree-sitter produced no parse tree"),
            HighlightError::Cancelled => f.write_str("syntax analysis superseded"),
            HighlightError::InjectionDepth => {
                f.write_str("syntax injection nesting exceeds eight levels")
            }
        }
    }
}

impl std::error::Error for HighlightError {}

/// Query text source over the rope itself: each node's text is yielded as
/// the rope's own chunk slices, so a node inside one chunk is compared
/// with zero copy. Only a node straddling a chunk boundary is assembled —
/// by tree-sitter, into its reusable buffers, bounded by node size.
struct RopeText<'a> {
    rope: &'a Rope,
}

impl<'a> TextProvider<&'a [u8]> for RopeText<'a> {
    type I = RopeSlices<'a>;

    fn text(&mut self, node: tree_sitter::Node<'_>) -> Self::I {
        RopeSlices {
            rope: self.rope,
            start: node.start_byte(),
            end: node.end_byte(),
        }
    }
}

/// The chunk slices covering `[start, end)`, in document order. An empty
/// range yields nothing — an empty node's text is empty, exactly as with
/// a plain byte-slice provider.
struct RopeSlices<'a> {
    rope: &'a Rope,
    start: usize,
    end: usize,
}

impl<'a> Iterator for RopeSlices<'a> {
    type Item = &'a [u8];

    fn next(&mut self) -> Option<Self::Item> {
        if self.start >= self.end {
            return None;
        }
        let (chunk, chunk_start, ..) = self.rope.chunk_at_byte(self.start);
        let head = &chunk[self.start - chunk_start..];
        // stop at the chunk's end or the node's end, whichever is first;
        // both are char boundaries (rope chunks and tree-sitter node
        // ranges always are), so the slice is valid UTF-8 as-is
        let take = head.len().min(self.end - self.start);
        let slice = &head.as_bytes()[..take];
        self.start += take;
        Some(slice)
    }
}

/// One language's parser + highlight query. The tree tracks the buffer's
/// journal (0022 §1): edits apply as a cheap pointer walk at commit time
/// and reparsing is incremental against the kept tree — a full parse is
/// the cold-start path, not the rule.
pub struct Highlighter {
    parser: Parser,
    query: Query,
    /// Capture index → class, resolved once at construction.
    classes: Vec<CaptureStyle>,
    source_hash: Option<BufferRevision>,
    spans: Vec<Span>,
    /// The parse tree covering `tree_revision`.
    tree: Option<tree_sitter::Tree>,
    tree_revision: BufferRevision,
    span_window: Option<(usize, usize)>,
    injection_query: Option<Query>,
    injection_depth: usize,
    children: Vec<injections::InjectedHighlighter>,
}

impl Highlighter {
    pub fn invalidate(&mut self) {
        self.tree = None;
        self.source_hash = None;
        self.span_window = None;
        self.children.clear();
    }

    /// Feed the pre-edit journal to the kept tree (0022 §1): a cheap
    /// pointer walk at commit time; the reparse stays lazy. The edits
    /// arrive exactly as `strop-core` published them — tuple points
    /// converted to tree-sitter points once, here — so the editor hands
    /// over `&[change.edit]` with `change.revision` untouched.
    pub fn apply_edits(&mut self, edits: &[strop_core::InputEdit], revision: BufferRevision) {
        if revision == self.tree_revision {
            return;
        }
        if let Some(tree) = &mut self.tree {
            for edit in edits {
                tree.edit(&tree_sitter::InputEdit {
                    start_byte: edit.start_byte,
                    old_end_byte: edit.old_end_byte,
                    new_end_byte: edit.new_end_byte,
                    start_position: tree_sitter::Point {
                        row: edit.start_point.0,
                        column: edit.start_point.1,
                    },
                    old_end_position: tree_sitter::Point {
                        row: edit.old_end_point.0,
                        column: edit.old_end_point.1,
                    },
                    new_end_position: tree_sitter::Point {
                        row: edit.new_end_point.0,
                        column: edit.new_end_point.1,
                    },
                });
            }
        }
        // with no kept tree the next parse builds it — the revision
        // still advances so reparse-once stays the rule, not per frame
        self.tree_revision = revision;
        for child in &mut self.children {
            child.apply_edits(edits, revision);
        }
    }

    /// Pure constructor: the path plus the rope that backs the document.
    /// When basename and extension both miss, the rope's first line —
    /// bounded to 256 bytes, assembled chunk-wise — is the shebang
    /// fallback. Nothing here reads the filesystem: UI dispatch never
    /// blocks on disk.
    pub fn for_path(path: &std::path::Path, rope: &Rope) -> Option<Self> {
        let spec = languages::detect(path, Some(&first_line_bounded(rope)))?;
        Self::from_spec(spec)
    }

    fn from_spec(spec: languages::LanguageSpec) -> Option<Self> {
        let mut parser = Parser::new();
        parser.set_language(&spec.language).ok()?;
        let query = Query::new(&spec.language, spec.highlights).ok()?;
        let injection_query = if spec.injections.is_empty() {
            None
        } else {
            Some(Query::new(&spec.language, spec.injections).ok()?)
        };
        let classes = query
            .capture_names()
            .iter()
            .map(|name| CaptureStyle {
                class: Class::from_capture(name),
                emphasis: Emphasis::from_capture(name),
            })
            .collect();
        Some(Self {
            parser,
            query,
            classes,
            source_hash: None,
            spans: Vec::new(),
            tree: None,
            tree_revision: BufferRevision::new(0),
            span_window: None,
            injection_query,
            injection_depth: 0,
            children: Vec::new(),
        })
    }

    /// Highlight spans intersecting `[first_byte, last_byte)` of the rope.
    /// Reparses only when the text changed. `revision` is the document's
    /// edit counter (0020 §5: the len+first+last key under-invalidated
    /// same-length middle edits deterministically). A parse that cannot
    /// complete is a typed [`HighlightError`] — never a hidden reparse
    /// fallback, never a panic.
    pub fn highlight(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        first_byte: usize,
        last_byte: usize,
    ) -> Result<Vec<Span>, HighlightError> {
        self.highlight_while(rope, revision, first_byte, last_byte, || false)
    }

    pub fn highlight_while(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        first_byte: usize,
        last_byte: usize,
        cancelled: impl Fn() -> bool,
    ) -> Result<Vec<Span>, HighlightError> {
        self.highlight_cancellable(rope, revision, first_byte, last_byte, &cancelled)
    }

    fn highlight_cancellable(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        first_byte: usize,
        last_byte: usize,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<Vec<Span>, HighlightError> {
        if cancelled() {
            return Err(HighlightError::Cancelled);
        }
        self.ensure_tree(rope, revision, cancelled)?;
        let window = (
            first_byte.min(rope.len_bytes()),
            last_byte.min(rope.len_bytes()),
        );
        if self.span_window != Some(window) {
            let tree = self.tree.as_ref().ok_or(HighlightError::Parse)?;
            let mut cursor = QueryCursor::new();
            cursor.set_byte_range(window.0..window.1);
            let mut progress = |_: &tree_sitter::QueryCursorState| cancelled();
            let mut captures = Vec::new();
            let mut matches = cursor.matches_with_options(
                &self.query,
                tree.root_node(),
                RopeText { rope },
                tree_sitter::QueryCursorOptions::new().progress_callback(&mut progress),
            );
            while let Some(m) = { StreamingIterator::next(&mut matches) } {
                if cancelled() {
                    return Err(HighlightError::Cancelled);
                }
                for cap in m.captures {
                    let node = cap.node;
                    if node.end_byte() <= window.0 || node.start_byte() >= window.1 {
                        continue;
                    }
                    let style = self.classes[cap.index as usize];
                    captures.push(LayeredSpan {
                        span: Span {
                            start: node.start_byte(),
                            end: node.end_byte(),
                            class: style.class,
                            emphasis: style.emphasis,
                        },
                        injected: false,
                    });
                }
            }
            if cancelled() {
                return Err(HighlightError::Cancelled);
            }
            drop(matches);
            captures.extend(self.injection_spans(rope, revision, window.0, window.1, cancelled)?);
            self.spans = spans::flatten(captures, window.0, window.1);
            self.span_window = Some(window);
        }
        Ok(self.spans.clone())
    }

    /// Parse (incrementally against the kept tree) until `revision` is
    /// covered. Cancellation and parse failure stay typed; the lexical
    /// cache drops with the tree it was captured from.
    fn ensure_tree(
        &mut self,
        rope: &Rope,
        revision: BufferRevision,
        cancelled: &dyn Fn() -> bool,
    ) -> Result<(), HighlightError> {
        if Some(revision) == self.source_hash {
            return Ok(());
        }
        // A skipped edit journal must never make an unchanged old tree
        // masquerade as the new rope. Exact journals retain incremental parse.
        if self.tree_revision != revision {
            self.tree = None;
        }
        let mut progress = |_: &tree_sitter::ParseState| cancelled();
        let tree = self.parser.parse_with_options(
            &mut |byte: usize, _| {
                if byte >= rope.len_bytes() {
                    return "";
                }
                let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                &chunk[byte - start..]
            },
            self.tree.as_ref(),
            Some(tree_sitter::ParseOptions::new().progress_callback(&mut progress)),
        );
        let Some(tree) = tree else {
            self.parser.reset();
            return Err(if cancelled() {
                HighlightError::Cancelled
            } else {
                HighlightError::Parse
            });
        };
        self.tree = Some(tree);
        self.tree_revision = revision;
        self.source_hash = Some(revision);
        self.span_window = None;
        Ok(())
    }
}

/// Largest char-boundary-aligned length of `head` that is at most `want`.
/// A byte cap can land inside a multibyte character; walking back at most
/// three bytes keeps the slice valid UTF-8.
fn cut_at_boundary(head: &str, want: usize) -> usize {
    let mut take = want.min(head.len());
    while take > 0 && !head.is_char_boundary(take) {
        take -= 1;
    }
    take
}

/// The rope's first line, capped at 256 bytes so a minified no-newline
/// blob can't turn shebang detection into a full-text copy. Assembled
/// chunk-wise, so a first line spanning rope chunks comes out whole
/// (up to the cap).
fn first_line_bounded(rope: &Rope) -> String {
    const CAP: usize = 256;
    // only a shebang can match — skip the copy for the common case
    if rope.len_bytes() == 0 || rope.byte(0) != b'#' {
        return String::new();
    }
    let limit = rope.len_bytes().min(CAP);
    let mut line = String::new();
    let mut byte = 0;
    while byte < limit {
        let (chunk, start, ..) = rope.chunk_at_byte(byte);
        let head = &chunk[byte - start..];
        let stop = head.find('\n').unwrap_or(head.len());
        let take = cut_at_boundary(head, stop.min(limit - byte));
        if take == 0 {
            break; // the cap cut inside a multibyte char
        }
        line.push_str(&head[..take]);
        if take == stop {
            break; // consumed through the newline (or the whole chunk had none)
        }
        byte += take;
    }
    line
}

#[cfg(test)]
mod tests;
