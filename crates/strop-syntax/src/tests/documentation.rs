use super::{apply_and_highlight, capture_at, clean_spans};
use crate::{Class, Highlighter, Span};
use ropey::Rope;
use strop_core::{id::BufferRevision, Buffer};

#[test]
fn ordinary_comments_and_go_line_comments_remain_host_comments() {
    for (path, source) in [
        (
            "doc.rs",
            "// ```javascript\n// const ordinary = 42;\n// ```\nfn host() {}\n",
        ),
        (
            "doc.js",
            "/*\n```javascript\nconst ordinary = 42;\n```\n*/\n",
        ),
        (
            "doc.go",
            "package doc\n// ```javascript\n// const ordinary = 42;\n// ```\nfunc host() {}\n",
        ),
        (
            "doc.go",
            "package doc\n/*\n```javascript\nconst ordinary = 42;\n```\n*/\n\nfunc host() {}\n",
        ),
    ] {
        let spans = clean_spans(std::path::Path::new(path), &Rope::from_str(source));
        assert_eq!(
            capture_at(&spans, source, "const ordinary").class,
            Class::Comment,
            "{path}"
        );
    }
}

#[test]
fn documentation_prefixes_preserve_crlf_and_unicode_byte_offsets() {
    let source =
        "// λ\r\n/// ```javascript\r\n/// const documented = \"é\";\r\n/// ```\r\nfn host() {}\r\n";
    let spans = clean_spans(std::path::Path::new("doc.rs"), &Rope::from_str(source));
    assert_eq!(
        capture_at(&spans, source, "const documented").class,
        Class::Keyword
    );
    assert_eq!(capture_at(&spans, source, "\"é\"").class, Class::String);
    let python =
        "# introductory comment\n\"\"\"```javascript\nconst documented = 42;\n```\n\"\"\"\n";
    let spans = clean_spans(std::path::Path::new("doc.py"), &Rope::from_str(python));
    assert_eq!(
        capture_at(&spans, python, "const documented").class,
        Class::Keyword
    );
}

#[test]
fn markdown_fences_resolve_non_rust_languages_and_unknown_overrides_shebang() {
    let source = "```python\ndef example():\n    return 42\n```\n```javascript\nconst example = 42;\n```\n```unknown-language\n#!/bin/bash\nif true; then echo yes; fi\n```\n";
    let spans = clean_spans(std::path::Path::new("guide.md"), &Rope::from_str(source));
    assert_eq!(
        capture_at(&spans, source, "def example").class,
        Class::Keyword
    );
    assert_eq!(
        capture_at(&spans, source, "const example").class,
        Class::Keyword
    );
    assert_eq!(capture_at(&spans, source, "if true").class, Class::Code);
}

#[test]
fn documentation_hosts_inject_registered_languages_without_styling_delimiters() {
    for (path, source, prefix, suffix) in [
        ("doc.rs", "/// ```python\n/// def example():\n///     return 42\n/// ```\nfn host() {}\n", "///", None),
        ("doc.rs", "//! ```python\n//! def example():\n//!     return 42\n//! ```\n", "//!", None),
        ("doc.rs", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nfn host() {}\n", "/**", Some("*/")),
        ("doc.rs", "/*!\n * ```python\n * def example():\n *     return 42\n * ```\n */\n", "/*!", Some("*/")),
        ("doc.js", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nfunction host() {}\n", "/**", Some("*/")),
        ("doc.ts", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nfunction host() {}\n", "/**", Some("*/")),
        ("doc.tsx", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nfunction host() {}\n", "/**", Some("*/")),
        ("doc.c", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nvoid host(void) {}\n", "/**", Some("*/")),
        ("doc.cpp", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nvoid host() {}\n", "/**", Some("*/")),
        ("Doc.java", "/**\n * ```python\n * def example():\n *     return 42\n * ```\n */\nclass Host {}\n", "/**", Some("*/")),
        ("doc.go", "package doc\n/*\n```python\ndef example():\n    return 42\n```\n*/\nfunc host() {}\n", "/*", Some("*/")),
    ] {
        let spans = clean_spans(std::path::Path::new(path), &Rope::from_str(source));
        assert_eq!(capture_at(&spans, source, "def example").class, Class::Keyword, "{path}");
        assert_eq!(capture_at(&spans, source, prefix).class, Class::Comment, "{path}");
        if let Some(suffix) = suffix {
            assert_eq!(capture_at(&spans, source, suffix).class, Class::Comment, "{path}");
        }
        if source.contains(" * def") {
            assert_eq!(capture_at(&spans, source, "* def").class, Class::Comment, "{path}");
        }
    }
}

#[test]
fn python_docstrings_are_structural_not_arbitrary_strings() {
    for source in [
        "\"\"\"Résumé.\n```javascript\nconst documented = 42;\n```\n\"\"\"\n",
        "class Example:\n    r\"\"\"Résumé.\n    ```javascript\n    const documented = 42;\n    ```\n    \"\"\"\n",
        "def example():\n    \"\"\"Résumé.\n    ```javascript\n    const documented = 42;\n    ```\n    \"\"\"\n",
    ] {
        let spans = clean_spans(std::path::Path::new("doc.py"), &Rope::from_str(source));
        assert_eq!(capture_at(&spans, source, "const documented").class, Class::Keyword);
        assert_eq!(capture_at(&spans, source, "\"\"\"").class, Class::String);
    }
    for source in [
        "example = \"\"\"```javascript\nconst ordinary = 42;\n```\n\"\"\"\n",
        "def example():\n    pass\n    \"\"\"```javascript\n    const ordinary = 42;\n    ```\n    \"\"\"\n",
        "b\"\"\"```javascript\nconst ordinary = 42;\n```\n\"\"\"\n",
        "f\"\"\"```javascript\nconst ordinary = 42;\n```\n\"\"\"\n",
    ] {
        let spans = clean_spans(std::path::Path::new("doc.py"), &Rope::from_str(source));
        assert_eq!(capture_at(&spans, source, "const ordinary").class, Class::String);
    }
}

#[test]
fn rustdoc_default_flags_and_explicit_languages_have_distinct_precedence() {
    for label in [
        "",
        "rust",
        "rust,ignore",
        "no_run",
        "compile_fail",
        "edition2021",
    ] {
        let source = format!("/// ```{label}\n/// fn documented() {{}}\n/// ```\nfn host() {{}}\n");
        let spans = clean_spans(std::path::Path::new("doc.rs"), &Rope::from_str(&source));
        assert_eq!(
            capture_at(&spans, &source, "fn documented").class,
            Class::Keyword,
            "{label}"
        );
    }
    let source = "/// ```unknown-language\n/// fn documented() {}\n/// ```\nfn host() {}\n";
    let spans = clean_spans(std::path::Path::new("doc.rs"), &Rope::from_str(source));
    assert_eq!(
        capture_at(&spans, source, "fn documented").class,
        Class::Code
    );
}

#[test]
fn independent_documentation_groups_do_not_share_unclosed_fences() {
    for separator in [
        "fn first() {}\n",
        "\n",
        "// ordinary comment\n",
        "//! inner documentation\n",
    ] {
        let source = format!(
            "/// ```python\n/// def unclosed():\n{separator}/// # Independent\nfn second() {{}}\n"
        );
        let spans = clean_spans(std::path::Path::new("doc.rs"), &Rope::from_str(&source));
        assert_eq!(
            capture_at(&spans, &source, "Independent").class,
            Class::Heading,
            "{separator:?}"
        );
    }
    let source = "/**\n * ```python\n * def unclosed():\n */\nfunction first() {}\n/**\n * # Independent\n */\nfunction second() {}\n";
    let spans = clean_spans(std::path::Path::new("doc.js"), &Rope::from_str(source));
    assert_eq!(
        capture_at(&spans, source, "Independent").class,
        Class::Heading
    );
}

#[test]
fn nested_documentation_ranges_preserve_offsets_and_viewport_context() {
    let source = "// Unicode outside: λ\n/// ```html\n/// <script>\n/// const nested = 42;\n/// </script>\n/// ```\nfn host() {}\n";
    let rope = Rope::from_str(source);
    let mut highlighter = Highlighter::for_path(std::path::Path::new("doc.rs"), &rope).unwrap();
    let first = source.find("const nested").unwrap();
    let last = first + "const nested = 42;".len();
    // Start cold in the middle of a group: neither opening fence is visible.
    let window = highlighter
        .highlight(&rope, BufferRevision::new(0), first, last)
        .unwrap();
    assert_eq!(
        capture_at(&window, source, "const nested").class,
        Class::Keyword
    );
    assert_eq!(capture_at(&window, source, "42").class, Class::Number);
    assert!(window
        .iter()
        .all(|span| first <= span.start && span.end <= last));
    let full = highlighter
        .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
        .unwrap();
    let clipped: Vec<_> = full
        .iter()
        .filter_map(|span| {
            let start = span.start.max(first);
            let end = span.end.min(last);
            (start < end).then_some(Span {
                start,
                end,
                ..*span
            })
        })
        .collect();
    assert_eq!(window, clipped);
    for (offset, _) in source.match_indices("///") {
        assert_eq!(
            full.iter()
                .find(|span| span.start <= offset && offset < span.end)
                .unwrap()
                .class,
            Class::Comment
        );
    }
}

#[test]
fn incremental_documentation_edits_match_fresh_including_changed_fence_language() {
    let path = std::path::Path::new("doc.rs");
    let source = "/// ```javascript\n/// const documented = 42;\n/// ```\nfn host() {}\n";
    let mut buffer = Buffer::from_text(source);
    let mut highlighter = Highlighter::for_path(path, buffer.text()).unwrap();
    highlighter
        .highlight(buffer.text(), buffer.revision(), 0, buffer.len_bytes())
        .unwrap();
    let start = source.find("javascript").unwrap();
    let changed = apply_and_highlight(
        &mut buffer,
        &mut highlighter,
        (start, start + 10),
        "unknown",
    );
    assert_eq!(changed, clean_spans(path, buffer.text()));
    assert_eq!(
        capture_at(&changed, &buffer.text().to_string(), "const documented").class,
        Class::Code
    );
    let restored = apply_and_highlight(
        &mut buffer,
        &mut highlighter,
        (start, start + 7),
        "javascript",
    );
    assert_eq!(restored, clean_spans(path, buffer.text()));
    assert_eq!(
        capture_at(&restored, &buffer.text().to_string(), "const documented").class,
        Class::Keyword
    );
    let shifted = apply_and_highlight(
        &mut buffer,
        &mut highlighter,
        (0, 0),
        "// λ shifts all nested byte offsets\n",
    );
    assert_eq!(shifted, clean_spans(path, buffer.text()));
    let current = buffer.text().to_string();
    let prefix = current.find("/// const").unwrap();
    let split = apply_and_highlight(&mut buffer, &mut highlighter, (prefix, prefix + 1), "");
    assert_eq!(split, clean_spans(path, buffer.text()));
}
