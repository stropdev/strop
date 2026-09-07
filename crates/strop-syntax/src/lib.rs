//! strop-syntax: tree-sitter highlighting. Parsers statically linked
//! (0002 §2.2 — never dlopen'd grammars); queries are data (0001 §5.11),
//! embedded defaults now, runtime overrides when config lands (0005).
//!
//! The whole pipeline is rope-backed: parsing walks rope chunks, and
//! query predicates (`#eq?`/`#match?`/…) read node text through a
//! [`tree_sitter::TextProvider`] over the same chunks — no full-text
//! `String` is materialized on any input or render path, and language
//! detection never touches the filesystem.

use std::collections::HashMap;

use streaming_iterator::StreamingIterator;
pub mod languages;

use ropey::Rope;
use strop_core::id::BufferRevision;
use tree_sitter::{Parser, Query, QueryCursor, TextProvider};

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

/// Highlighting failed out loud: no hidden full-text reparse fallback, no
/// swallowed error, no panic — the caller decides what the failure means
/// for its surface.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HighlightError {
    /// tree-sitter refused the chunked input (encoding breach or an
    /// internal limit). Cached spans and the kept tree stay untouched, so
    /// the next call retries from the same state.
    Parse,
}

impl std::fmt::Display for HighlightError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            HighlightError::Parse => f.write_str("tree-sitter produced no parse tree"),
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
    classes: Vec<Class>,
    source_hash: BufferRevision,
    spans: Vec<Span>,
    /// The parse tree covering `tree_revision`.
    tree: Option<tree_sitter::Tree>,
    tree_revision: BufferRevision,
}

impl Highlighter {
    /// Drop the kept tree (0023: a mutation path that can't produce
    /// exact edit coordinates invalidates rather than lying).
    pub fn invalidate(&mut self) {
        self.tree = None;
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
        let classes = query
            .capture_names()
            .iter()
            .map(|n| Class::from_capture(n))
            .collect();
        Some(Self {
            parser,
            query,
            classes,
            source_hash: BufferRevision::from(u64::MAX), // never a real revision
            spans: Vec::new(),
            tree: None,
            tree_revision: BufferRevision::new(0),
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
        if revision != self.source_hash {
            // 0022 §1: parse from rope chunks against the kept tree —
            // no String materialization, no from-scratch parse unless
            // there is no tree to reuse
            let tree = self
                .parser
                .parse_with_options(
                    &mut |byte: usize, _| {
                        // random-access chunks (0022 fix): tree-sitter
                        // re-requests earlier bytes on error recovery —
                        // a forward-only chunk iterator underflowed there
                        // and panicked (or fed garbage in release builds)
                        if byte >= rope.len_bytes() {
                            return "";
                        }
                        let (chunk, start, _, _) = rope.chunk_at_byte(byte);
                        &chunk[byte - start..]
                    },
                    self.tree.as_ref(),
                    None,
                )
                .ok_or(HighlightError::Parse)?;
            self.tree = Some(tree.clone());
            self.tree_revision = revision;
            // predicates read node text as rope chunk slices: zero-copy
            // inside a chunk, assembled by tree-sitter only across a
            // chunk boundary
            let mut cursor = QueryCursor::new();
            let mut by_byte: HashMap<usize, (usize, Class)> = HashMap::new();
            let mut matches = cursor.matches(&self.query, tree.root_node(), RopeText { rope });
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
            self.source_hash = revision;
        }
        // return only visible spans; spans are sorted, binary search the window
        let lo = self.spans.partition_point(|s| s.end <= first_byte);
        let hi = self.spans.partition_point(|s| s.start < last_byte);
        Ok(self.spans[lo..hi.max(lo)].to_vec())
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
mod tests {
    use super::*;
    use strop_core::{Buffer, Range};

    fn clean_spans(path: &std::path::Path, rope: &Rope) -> Vec<Span> {
        let mut hl = Highlighter::for_path(path, rope).expect("language");
        hl.highlight(rope, BufferRevision::new(0), 0, rope.len_bytes())
            .expect("parse")
    }

    fn classes_for(path: &std::path::Path, src: &str) -> Vec<Class> {
        clean_spans(path, &Rope::from_str(src))
            .iter()
            .map(|s| s.class)
            .collect()
    }

    /// Drive the highlighter exactly as the editor does: core applies the
    /// replacement, publishes the pre-edit journal, the highlighter walks
    /// it onto the kept tree, and the next highlight reparses
    /// incrementally. `(start, end)` are pre-edit byte offsets.
    fn apply_and_highlight(
        buf: &mut Buffer,
        hl: &mut Highlighter,
        (start, end): (usize, usize),
        text: &str,
    ) -> Vec<Span> {
        buf.edit()
            .replace(Range::charwise(start, end), text)
            .expect("edit");
        for change in buf.changes() {
            hl.apply_edits(std::slice::from_ref(&change.edit), change.revision);
        }
        buf.clear_changes();
        hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
            .expect("parse")
    }

    #[test]
    fn rust_keywords_and_strings() {
        let classes = classes_for(
            std::path::Path::new("x.rs"),
            "fn main() { let s = \"hi\"; }\n",
        );
        assert!(classes.contains(&Class::Keyword), "{classes:?}");
        assert!(classes.contains(&Class::String), "{classes:?}");
    }

    #[test]
    fn cpp_highlights_with_cxx_scanner() {
        // the 0002 §5 gate: C++ grammar's scanner is C++ — a broken
        // static-libstdc++ link fails here, per-PR, not at a user's file.
        let classes = classes_for(std::path::Path::new("x.cpp"), "auto edge = hone(blade);\n");
        assert!(!classes.is_empty(), "cpp grammar produced no spans");
        assert!(classes.contains(&Class::Type), "{classes:?}"); // auto → @type.builtin
    }

    #[test]
    fn python_and_go_and_ts() {
        assert!(
            classes_for(std::path::Path::new("x.py"), "def f(x):\n    return x\n")
                .contains(&Class::Keyword)
        );
        assert!(classes_for(
            std::path::Path::new("x.go"),
            "package main\nfunc main() {}\n"
        )
        .contains(&Class::Keyword));
        assert!(!classes_for(std::path::Path::new("x.ts"), "const x: number = 1;\n").is_empty());
        assert!(!classes_for(std::path::Path::new("x.json"), "{\"a\": 1}\n").is_empty());
        assert!(!classes_for(std::path::Path::new("x.sh"), "#!/bin/sh\necho hi\n").is_empty());
    }

    #[test]
    fn fish_lua_and_sql() {
        // for_path compiles each vendored Helix query against its
        // grammar — node drift upstream surfaces here as a None.
        assert!(!classes_for(std::path::Path::new("x.fish"), "set -l name rust\n").is_empty());
        assert!(
            classes_for(std::path::Path::new("x.lua"), "local x = 1\n").contains(&Class::Keyword)
        );
        assert!(
            classes_for(std::path::Path::new("x.sql"), "SELECT * FROM users;\n")
                .contains(&Class::Keyword)
        );
    }

    #[test]
    fn for_path_is_pure_over_path_and_rope() {
        // extensionless path that does not exist on disk: detection must
        // come from the rope's shebang, not from a filesystem probe
        let src = "#!/usr/bin/env bash\necho hi\n";
        let rope = Rope::from_str(src);
        let mut hl =
            Highlighter::for_path(std::path::Path::new("strop-syntax-purity-probe"), &rope)
                .expect("bash via rope shebang");
        assert!(!hl
            .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
            .expect("parse")
            .is_empty());
    }

    #[test]
    fn shebang_detection_is_bounded_at_256_bytes() {
        // interpreter name running past the cap cannot resolve
        let over = format!("#!/bin/{}\nls\n", "b".repeat(300));
        let rope = Rope::from_str(&over);
        assert!(Highlighter::for_path(std::path::Path::new("probe"), &rope).is_none());
        // same shape, name inside the cap
        let under = Rope::from_str("#!/bin/bash\nls\n");
        assert!(Highlighter::for_path(std::path::Path::new("probe"), &under).is_some());
        // a multibyte run crossing the cap must not panic the cut
        let wide = format!("#!/bin/{}\nls\n", "é".repeat(200));
        let rope = Rope::from_str(&wide);
        assert!(Highlighter::for_path(std::path::Path::new("probe"), &rope).is_none());
    }

    #[test]
    fn first_line_assembly_walks_rope_chunks() {
        // a first line long enough to span ropey's internal chunk layout
        let mut line = String::from("#!/usr/bin/env bash");
        line.push_str(&" # padding ".repeat(600));
        let rope = Rope::from_str(&format!("{line}\nls\n"));
        assert!(
            rope.chunks().count() > 1,
            "precondition: multi-chunk first line"
        );
        let got = first_line_bounded(&rope);
        assert!(got.starts_with("#!/usr/bin/env bash"));
        assert!(line.starts_with(got.as_str()), "bounded prefix of the line");
        assert_eq!(got.len(), 256, "ASCII content fills the cap exactly");
    }

    #[test]
    fn window_and_cache_semantics_hold() {
        let rope = Rope::from_str("fn main() { let s = \"hi\"; let n = 7; }\n");
        let path = std::path::Path::new("x.rs");
        let mut hl = Highlighter::for_path(path, &rope).unwrap();
        let full = hl
            .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
            .unwrap();
        assert!(!full.is_empty());
        // a mid-document window returns exactly the intersecting spans
        let anchor = full
            .iter()
            .find(|s| s.class == Class::String)
            .expect("a string span");
        let (w0, w1) = (anchor.start - 1, anchor.end + 1);
        let window = hl.highlight(&rope, BufferRevision::new(0), w0, w1).unwrap();
        let expect: Vec<Span> = full
            .iter()
            .copied()
            .filter(|s| s.end > w0 && s.start < w1)
            .collect();
        assert_eq!(window, expect);
        // same revision: served from cache, identical to a cold ask
        let again = hl
            .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
            .unwrap();
        assert_eq!(again, full);
    }

    #[test]
    fn incremental_replacement_matches_clean_parse() {
        // a multibyte replacement on a mid-line range: the journal's byte
        // coordinates must land the incrementally edited tree on exactly
        // the spans a fresh parse of the same text produces
        let path = std::path::Path::new("x.rs");
        let mut buf = Buffer::from_text("fn main() { let s = \"hi\"; let t = 2; }\n");
        let mut hl = Highlighter::for_path(path, buf.text()).unwrap();
        let warm = hl
            .highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
            .unwrap();
        assert!(warm.iter().any(|s| s.class == Class::String));

        let start = buf.text().to_string().find("\"hi\"").unwrap();
        let got = apply_and_highlight(&mut buf, &mut hl, (start, start + 4), "\"wörld → 🌍\"");
        assert!(got.iter().any(|s| s.class == Class::String));
        assert_eq!(got, clean_spans(path, buf.text()));
    }

    #[test]
    fn incremental_multiline_edit_matches_clean_parse() {
        // insert a multiline string, then replace it across line
        // boundaries — row/column points in the journal must survive both
        let path = std::path::Path::new("x.rs");
        let mut buf = Buffer::from_text("fn a() {}\nfn b() {}\n");
        let mut hl = Highlighter::for_path(path, buf.text()).unwrap();
        hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
            .unwrap();

        let got = apply_and_highlight(&mut buf, &mut hl, (7, 7), " let s = \"one\ntwo 🌍\";");
        assert_eq!(got, clean_spans(path, buf.text()));

        let text = buf.text().to_string();
        let start = text.find("\"one\ntwo 🌍\"").unwrap();
        let got = apply_and_highlight(
            &mut buf,
            &mut hl,
            (start, start + "\"one\ntwo 🌍\"".len()),
            "\"x\"",
        );
        assert_eq!(got, clean_spans(path, buf.text()));
    }

    #[test]
    fn incremental_undo_roundtrip_matches_clean_parse() {
        // forward edit, then reverse it through the same journal bridge:
        // the tree must end exactly where a clean parse of the restored
        // text is — undo is just another replacement to it
        let path = std::path::Path::new("x.rs");
        let original = "fn main() { let x = 1; }\n";
        let mut buf = Buffer::from_text(original);
        let mut hl = Highlighter::for_path(path, buf.text()).unwrap();
        hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
            .unwrap();

        let start = original.find("1").unwrap();
        let forward = apply_and_highlight(&mut buf, &mut hl, (start, start + 1), "0x1f 🌍");
        assert_eq!(forward, clean_spans(path, buf.text()));

        let back = apply_and_highlight(&mut buf, &mut hl, (start, start + "0x1f 🌍".len()), "1");
        assert_eq!(back, clean_spans(path, buf.text()));
        assert_eq!(buf.text().to_string(), original);
    }

    #[test]
    fn predicates_evaluate_across_rope_chunk_boundaries() {
        // `#match? @constant "^[A-Z][A-Z\d_]*$"` must see the WHOLE
        // identifier even when the rope splits it across chunks. A ~5KB
        // tail holds ropey's chunk layout steady (boundaries near
        // 984-byte multiples for multi-KB ropes) while the padding
        // comment slides the identifier across one full period; only a
        // provable straddle is asserted on.
        const CAPS: &str = "A_VERY_LONG_CAPS_IDENTIFIER_STRADDLING_ROPE_CHUNKS";
        let tail = "fn tail() { let filler = \"padding to a multi-chunk rope\"; }\n".repeat(90);
        let mut found = false;
        for pad in (0..1050usize).step_by(11) {
            let src = format!(
                "fn main() {{\n    //{}\n    let {} = 1;\n    let {}_use = {};\n}}\n{}",
                "x".repeat(pad),
                CAPS,
                CAPS.to_lowercase(),
                CAPS,
                tail,
            );
            let rope = Rope::from_str(&src);
            let start = src.find(CAPS).unwrap();
            let end = start + CAPS.len();
            if !spans_chunks(&rope, start, end) {
                continue;
            }
            let spans = clean_spans(std::path::Path::new("x.rs"), &rope);
            let class = spans.iter().find(|s| s.start == start).map(|s| s.class);
            assert_eq!(
                class,
                Some(Class::Constant),
                "pad {pad}: predicate must see the whole identifier across chunks"
            );
            found = true;
            break;
        }
        assert!(
            found,
            "the sweep never produced a chunk-straddling identifier"
        );
    }

    /// Does the byte range `[start, end)` strictly contain a rope chunk
    /// boundary?
    fn spans_chunks(rope: &Rope, start: usize, end: usize) -> bool {
        let mut offset = 0;
        for chunk in rope.chunks() {
            if offset > start && offset < end {
                return true;
            }
            offset += chunk.len();
        }
        false
    }

    #[test]
    fn incremental_on_multichunk_rope_matches_clean_parse() {
        // large file: the journal edit lands mid-rope, far from byte 0,
        // and the incremental tree must still match a cold parse
        let path = std::path::Path::new("x.rs");
        let mut big = String::from("fn top() {}\n");
        for i in 0..80 {
            big.push_str(&format!("fn f{i}() {{ let s{i} = \"{i}\"; }}\n"));
        }
        big.push_str("fn bottom() {}\n");
        let mut buf = Buffer::from_text(&big);
        let mut hl = Highlighter::for_path(path, buf.text()).unwrap();
        assert!(
            buf.text().chunks().count() > 1,
            "precondition: multi-chunk rope"
        );
        hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
            .unwrap();

        let mid = big.len() / 2;
        let line_start = big[..mid].rfind('\n').map(|i| i + 1).unwrap_or(0);
        let got = apply_and_highlight(
            &mut buf,
            &mut hl,
            (line_start, line_start),
            "let inserted_mid_rope = \"x\";\n",
        );
        assert_eq!(got, clean_spans(path, buf.text()));
    }

    #[test]
    fn highlight_survives_backtracking_requests() {
        // 0022 fix: tree-sitter re-requests earlier bytes on error
        // recovery in large template-heavy files — the forward-only
        // chunk iterator underflowed and panicked (the optional crash)
        let mut big = String::from("namespace std {\n");
        for i in 0..400 {
            big.push_str(&format!(
                "template <typename T{i}> struct O{i} {{ T{i} v; O{i} f() {{ return O{i}{{}}; }} }};\n"
            ));
        }
        big.push_str("}\n");
        let rope = ropey::Rope::from_str(&big);
        let mut hl = Highlighter::for_path(std::path::Path::new("x.hpp"), &rope).unwrap();
        let spans = hl
            .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
            .unwrap();
        assert!(!spans.is_empty(), "the big file highlights");
        // an edit shifts everything — the chunk callback sees arbitrary
        // byte asks and must not panic either
        let edited = big.replacen("namespace", "namespace extra_long_name_here", 1);
        let rope2 = ropey::Rope::from_str(&edited);
        let spans2 = hl
            .highlight(&rope2, BufferRevision::new(1), 0, rope2.len_bytes())
            .unwrap();
        assert!(!spans2.is_empty());
    }
}
