//! strop-syntax: tree-sitter highlighting. Parsers statically linked
//! (0002 §2.2 — never dlopen'd grammars); queries are data (0001 §5.11),
//! embedded defaults now, runtime overrides when config lands (0005).

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
pub mod languages;

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
        let spec = languages::detect(path, None).or_else(|| {
            // basename/extension both missed: one bounded read of the
            // first line, and the shebang decides (languages::detect)
            let line = first_line(path)?;
            languages::detect(path, Some(&line))
        })?;
        Self::from_spec(spec)
    }

    fn from_spec(spec: languages::LanguageSpec) -> Option<Self> {
        let mut parser = Parser::new();
        parser.set_language(&spec.language).ok()?;
        let query = Query::new(&spec.language, spec.highlights).ok()?;
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

/// First line of a file, capped at 256 bytes so a minified no-newline
/// blob can't turn a probe into a full read. `None` on any IO/decoding
/// hiccup — shebang detection is a best-effort fallback, never an error.
fn first_line(path: &str) -> Option<String> {
    use std::io::{BufRead, BufReader, Read};
    let mut line = String::new();
    BufReader::new(std::fs::File::open(path).ok()?)
        .take(256)
        .read_line(&mut line)
        .ok()?;
    Some(line)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn classes_for(path: &str, src: &str) -> Vec<Class> {
        let mut hl = Highlighter::for_path(path).expect("language");
        let rope = ropey::Rope::from_str(src);
        hl.highlight(&rope, 0, src.len())
            .iter()
            .map(|s| s.class)
            .collect()
    }

    #[test]
    fn rust_keywords_and_strings() {
        let classes = classes_for("x.rs", "fn main() { let s = \"hi\"; }\n");
        assert!(classes.contains(&Class::Keyword), "{classes:?}");
        assert!(classes.contains(&Class::String), "{classes:?}");
    }

    #[test]
    fn cpp_highlights_with_cxx_scanner() {
        // the 0002 §5 gate: C++ grammar's scanner is C++ — a broken
        // static-libstdc++ link fails here, per-PR, not at a user's file.
        let classes = classes_for("x.cpp", "auto edge = hone(blade);\n");
        assert!(!classes.is_empty(), "cpp grammar produced no spans");
        assert!(classes.contains(&Class::Type), "{classes:?}"); // auto → @type.builtin
    }

    #[test]
    fn python_and_go_and_ts() {
        assert!(classes_for("x.py", "def f(x):\n    return x\n").contains(&Class::Keyword));
        assert!(classes_for("x.go", "package main\nfunc main() {}\n").contains(&Class::Keyword));
        assert!(!classes_for("x.ts", "const x: number = 1;\n").is_empty());
        assert!(!classes_for("x.json", "{\"a\": 1}\n").is_empty());
        assert!(!classes_for("x.sh", "#!/bin/sh\necho hi\n").is_empty());
    }

    #[test]
    fn fish_lua_and_sql() {
        // for_path compiles each vendored Helix query against its
        // grammar — node drift upstream surfaces here as a None.
        assert!(!classes_for("x.fish", "set -l name rust\n").is_empty());
        assert!(classes_for("x.lua", "local x = 1\n").contains(&Class::Keyword));
        assert!(classes_for("x.sql", "SELECT * FROM users;\n").contains(&Class::Keyword));
    }

    #[test]
    fn shebang_script_file_resolves() {
        // extensionless file on disk: for_path must read its first
        // line once and hand it to the shebang fallback
        let path =
            std::env::temp_dir().join(format!("strop-syntax-shebang-{}", std::process::id()));
        std::fs::write(&path, "#!/usr/bin/env bash\necho hi\n").unwrap();
        let resolved = Highlighter::for_path(path.to_str().unwrap());
        std::fs::remove_file(&path).ok();
        let mut hl = resolved.expect("bash via shebang");
        let rope = ropey::Rope::from_str("#!/usr/bin/env bash\necho hi\n");
        assert!(!hl.highlight(&rope, 0, rope.len_bytes()).is_empty());
    }
}
