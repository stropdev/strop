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
    assert!(classes_for(std::path::Path::new("x.lua"), "local x = 1\n").contains(&Class::Keyword));
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
    let mut hl = Highlighter::for_path(std::path::Path::new("strop-syntax-purity-probe"), &rope)
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

fn capture_at<'a>(spans: &'a [Span], source: &str, token: &str) -> &'a Span {
    let byte = source
        .find(token)
        .unwrap_or_else(|| panic!("fixture token {token:?}"));
    spans
        .iter()
        .find(|span| span.start <= byte && byte < span.end)
        .unwrap_or_else(|| panic!("no style for {token:?} at {byte}: {spans:?}"))
}

#[test]
fn cmake_commands_variables_and_conditions_have_semantic_styles() {
    let source = "cmake_minimum_required(VERSION 3.20)\nset(PROJECT_NAME \"strop\")\nif(PROJECT_NAME)\nmessage(STATUS \"ok\")\nendif()\n";
    let spans = clean_spans(
        std::path::Path::new("CMakeLists.txt"),
        &Rope::from_str(source),
    );
    assert_eq!(
        capture_at(&spans, source, "cmake_minimum_required").class,
        Class::Function
    );
    assert_eq!(
        capture_at(&spans, source, "PROJECT_NAME").class,
        Class::Variable
    );
    assert_eq!(capture_at(&spans, source, "\"strop\"").class, Class::String);
    assert_eq!(capture_at(&spans, source, "if(").class, Class::Keyword);
}

#[test]
fn markdown_inline_styles_and_fenced_rust_keep_their_byte_domains() {
    let source = "# Heading\n\n**bold** and *italic* [link](https://example.invalid) `code`.\n\n```rust\nfn demo() {}\n```\n";
    let spans = clean_spans(std::path::Path::new("README.md"), &Rope::from_str(source));
    assert_eq!(capture_at(&spans, source, "Heading").class, Class::Heading);
    assert!(capture_at(&spans, source, "bold").emphasis.bold);
    assert!(capture_at(&spans, source, "italic").emphasis.italic);
    let link = capture_at(&spans, source, "https://");
    assert_eq!(link.class, Class::Link);
    assert!(link.emphasis.underline);
    assert_eq!(capture_at(&spans, source, "code").class, Class::Code);
    assert_eq!(capture_at(&spans, source, "fn demo").class, Class::Keyword);
}

#[test]
fn editing_a_fence_language_retires_old_injected_spans() {
    let path = std::path::Path::new("guide.markdown");
    let source = "```rust\nfn demo() {}\n```\n";
    let mut buffer = Buffer::from_text(source);
    let mut highlighter = Highlighter::for_path(path, buffer.text()).unwrap();
    let before = highlighter
        .highlight(buffer.text(), buffer.revision(), 0, buffer.len_bytes())
        .unwrap();
    assert_eq!(capture_at(&before, source, "fn demo").class, Class::Keyword);
    let after = apply_and_highlight(&mut buffer, &mut highlighter, (3, 7), "text");
    let changed = buffer.text().to_string();
    assert_eq!(capture_at(&after, &changed, "fn demo").class, Class::Code);
    assert_eq!(after, clean_spans(path, buffer.text()));
}

#[test]
fn additional_grammars_highlight_representative_language_tokens() {
    for (path, source, token, class) in [
        (
            "Demo.java",
            "class Demo { int value = 1; }\n",
            "class",
            Class::Keyword,
        ),
        (
            "Demo.cs",
            "class Demo { public int Value = 1; }\n",
            "class",
            Class::Keyword,
        ),
        (
            "demo.rb",
            "def demo\n  puts \"ok\"\nend\n",
            "def",
            Class::Keyword,
        ),
        (
            "demo.php",
            "<?php function demo() { return \"ok\"; }\n",
            "function",
            Class::Keyword,
        ),
        (
            "Cargo.toml",
            "[package]\nname = \"strop\"\n",
            "\"strop\"",
            Class::String,
        ),
        (
            "config.yaml",
            "name: \"strop\"\n",
            "\"strop\"",
            Class::String,
        ),
        (
            "index.html",
            "<div class=\"note\">hello</div>\n",
            "div",
            Class::Tag,
        ),
        (
            "style.css",
            "body { content: \"ok\"; }\n",
            "\"ok\"",
            Class::String,
        ),
    ] {
        let spans = clean_spans(std::path::Path::new(path), &Rope::from_str(source));
        assert_eq!(capture_at(&spans, source, token).class, class, "{path}");
    }
}

#[test]
fn html_embedded_script_and_style_use_the_shared_registry() {
    let source = "<script>const answer = 42;</script><style>body { content: \"yes\"; }</style>\n";
    let spans = clean_spans(std::path::Path::new("index.html"), &Rope::from_str(source));
    assert_eq!(capture_at(&spans, source, "const").class, Class::Keyword);
    assert_eq!(capture_at(&spans, source, "42").class, Class::Number);
    assert_eq!(capture_at(&spans, source, "\"yes\"").class, Class::String);
}
