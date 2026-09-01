//! strop-syntax: tree-sitter highlighting. Parsers statically linked
//! (0002 §2.2 — never dlopen'd grammars); queries are data (0001 §5.11),
//! embedded defaults now, runtime overrides when config lands (0005).

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
use tree_sitter::{Parser, Query, QueryCursor};

/// Semantic classes the renderer maps to palette colors. Kept small and
/// stable; the query capture names map onto these.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
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
}

impl Class {
    fn from_capture(name: &str) -> Self {
        let head = name.split('.').next().unwrap_or(name);
        match head {
            "keyword" => Class::Keyword,
            "function" | "constructor" => Class::Function,
            "type" => Class::Type,
            "string" | "character" => Class::String,
            "comment" => Class::Comment,
            "number" | "float" => Class::Number,
            "operator" => Class::Operator,
            "punctuation" => Class::Punctuation,
            "constant" | "boolean" => Class::Constant,
            "attribute" | "property" => Class::Attribute,
            _ => Class::Variable,
        }
    }
}

/// A colored span, in byte offsets.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Span {
    pub start: usize,
    pub end: usize,
    pub class: Class,
}

/// One language's parser + highlight query. Reparses on demand; the
/// incremental edit-diff feed (0001 pillar 4) lands when the core reports
/// edits — prototype correctness first, per-frame cost is invisible at
/// demo file sizes.
pub struct Highlighter {
    parser: Parser,
    query: Query,
    /// Capture index → class, resolved once at construction.
    classes: Vec<Class>,
    source_hash: u64,
    spans: Vec<Span>,
}

impl Highlighter {
    pub fn for_path(path: &str) -> Option<Self> {
        if !path.ends_with(".rs") {
            return None;
        }
        let mut parser = Parser::new();
        parser
            .set_language(&tree_sitter_rust::LANGUAGE.into())
            .ok()?;
        let query = Query::new(
            &tree_sitter_rust::LANGUAGE.into(),
            tree_sitter_rust::HIGHLIGHTS_QUERY,
        )
        .ok()?;
        let classes = query
            .capture_names()
            .iter()
            .map(|n| Class::from_capture(n))
            .collect();
        Some(Self {
            parser,
            query,
            classes,
            source_hash: 0,
            spans: Vec::new(),
        })
    }

    /// Highlight spans intersecting `[first_byte, last_byte)` of the rope.
    /// Reparses only when the text changed. Owned: callers hold buffer
    /// borrows, so the visible-window clone (small) keeps lifetimes flat.
    pub fn highlight(
        &mut self,
        rope: &ropey::Rope,
        first_byte: usize,
        last_byte: usize,
    ) -> Vec<Span> {
        let mut hasher = std::hash::DefaultHasher::new();
        std::hash::Hash::hash(&rope.len_bytes(), &mut hasher);
        // cheap change detector: length + first/last bytes; sufficient for
        // the prototype, replaced by real edit-diff tracking later
        if let (Some(first), Some(last)) = (
            rope.get_byte(0),
            rope.len_bytes()
                .checked_sub(1)
                .and_then(|i| rope.get_byte(i)),
        ) {
            std::hash::Hash::hash(&(first, last), &mut hasher);
        }
        let hash = std::hash::Hasher::finish(&hasher);
        if hash != self.source_hash {
            let text = rope.to_string(); // prototype: whole-buffer; chunk callback when hot
            let Some(tree) = self.parser.parse(&text, None) else {
                return Vec::new();
            };
            let mut cursor = QueryCursor::new();
            let mut by_byte: HashMap<usize, (usize, Class)> = HashMap::new();
            let mut matches = cursor.matches(&self.query, tree.root_node(), text.as_bytes());
            while let Some(m) = { StreamingIterator::next(&mut matches) } {
                for cap in m.captures {
                    let node = cap.node;
                    let class = self.classes[cap.index as usize];
                    // most specific wins: smallest containing span
                    let entry = by_byte
                        .entry(node.start_byte())
                        .or_insert((node.end_byte(), class));
                    if node.end_byte() - node.start_byte() <= entry.0 - node.start_byte() {
                        *entry = (node.end_byte(), class);
                    }
                }
            }
            let mut spans: Vec<Span> = by_byte
                .into_iter()
                .map(|(start, (end, class))| Span { start, end, class })
                .collect();
            spans.sort_by_key(|s| (s.start, s.end));
            self.spans = spans;
            self.source_hash = hash;
        }
        // return only visible spans; spans are sorted, binary search the window
        let lo = self.spans.partition_point(|s| s.end <= first_byte);
        let hi = self.spans.partition_point(|s| s.start < last_byte);
        self.spans[lo..hi.max(lo)].to_vec()
    }
}
