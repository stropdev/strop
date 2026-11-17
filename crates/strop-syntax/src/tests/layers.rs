//! 0111: injected layers and budgets — depth, combined injections, kept
//! child trees, journal contiguity, long lines.
use super::{apply_and_highlight, capture_at, clean_spans};
use crate::{Class, Highlighter, Span, LINE_BUDGET};
use ropey::Rope;
use std::path::Path;
use strop_core::{id::BufferRevision, Buffer, Range};

fn windowed(path: &str, source: &str, first: usize, last: usize) -> Vec<Span> {
    let rope = Rope::from_str(source);
    let mut hl = Highlighter::for_path(Path::new(path), &rope).expect("language");
    hl.highlight(&rope, BufferRevision::new(0), first, last)
        .expect("parse")
}

/// `levels` Markdown fences nested inside each other around a line of
/// prose (fence lengths shrink inwards), then an ordinary Rust block.
fn nested(levels: usize) -> String {
    let mut source = String::new();
    for level in 0..levels {
        source.push_str(&"`".repeat(3 + levels - level));
        source.push_str("markdown\n");
    }
    source.push_str("prose *here*\n");
    for level in (0..levels).rev() {
        source.push_str(&"`".repeat(3 + levels - level));
        source.push('\n');
    }
    source.push_str("\n```rust\nfn main() {}\n```\n");
    source
}

#[test]
fn nesting_past_the_depth_cap_keeps_the_rest_of_the_window_coloured() {
    for levels in [7, 8, 9, 12] {
        let source = nested(levels);
        let spans = clean_spans(Path::new("nested.md"), &Rope::from_str(&source));
        assert_eq!(
            capture_at(&spans, &source, "fn").class,
            Class::Keyword,
            "{levels} levels: the trailing Rust block keeps its colour"
        );
    }
}

#[test]
fn a_combined_html_closer_keeps_its_colour_when_its_opener_is_off_screen() {
    for (path, source, from, closer) in [
        ("page.md", "<div>\n\npara\n\n</div>\n", "para", "div>"),
        (
            "page.php",
            "<html>\n<?php echo 1; ?>\n<div>\n<b>t</b>\n</div>\n</html>\n",
            "<b>",
            "html>",
        ),
    ] {
        let from = source.find(from).expect("window start");
        let closer = source.rfind(closer).expect("closing tag");
        let spans = windowed(path, source, from, source.len());
        let tag = spans
            .iter()
            .find(|span| span.start <= closer && closer < span.end)
            .unwrap_or_else(|| panic!("{path}: the closing tag is coloured: {spans:?}"));
        assert_eq!(tag.class, Class::Tag, "{path}");
    }
}

#[test]
fn typing_inside_a_fence_keeps_the_injected_tree() {
    let body: String = (0..200).map(|i| format!("fn f{i}() {{}}\n")).collect();
    let source = format!("# T\n\n```rust\n{body}```\n");
    let mut buf = Buffer::from_text(&source);
    let mut hl = Highlighter::for_path(Path::new("a.md"), buf.text()).expect("markdown");
    hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");
    let first_item = |hl: &Highlighter| {
        hl.injected_tree(crate::LanguageId::Rust)
            .and_then(|tree| tree.root_node().named_child(0))
            .map(|node| node.id())
    };
    let before = first_item(&hl).expect("the fence has a rust layer");
    // type at the end of the fence: its range grows, its start stays put
    let at = source.rfind("```").expect("closing fence");
    let spans = apply_and_highlight(&mut buf, &mut hl, (at, at), "fn g() {}\n");
    assert_eq!(
        first_item(&hl),
        Some(before),
        "the first item far from the edit is reused, not reparsed"
    );
    assert_eq!(spans, clean_spans(Path::new("a.md"), buf.text()));
}

#[test]
fn a_journal_gap_discards_the_kept_tree() {
    let path = Path::new("x.rs");
    let mut buf = Buffer::from_text("fn a() { let s = 1; }\nfn b() {}\n");
    let mut hl = Highlighter::for_path(path, buf.text()).expect("rust");
    hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");
    buf.edit()
        .replace(Range::charwise(0, 0), "/* ")
        .expect("first edit");
    let len = buf.len_bytes();
    buf.edit()
        .replace(Range::charwise(len, len), "*/\n")
        .expect("second edit");
    let changes = buf.changes().to_vec();
    assert_eq!(changes.len(), 2);
    // the first batch is lost; only the second reaches the highlighter
    hl.apply_edits(&[changes[1].edit], changes[1].revision);
    let got = hl
        .highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");
    assert_eq!(got, clean_spans(path, buf.text()));
    assert!(got.iter().all(|span| span.class == Class::Comment));
}

#[test]
fn a_long_line_is_highlighted_only_up_to_the_budget() {
    let items: Vec<String> = (0..60_000).map(|i| format!("{{k:{i}}}")).collect();
    let source = format!("var a = [{}];\nlet b = \"tail\";\n", items.join(","));
    let line1 = source.find("let b").expect("second line");
    let spans = clean_spans(Path::new("big.js"), &Rope::from_str(&source));
    assert!(spans.iter().any(|span| span.end <= LINE_BUDGET));
    assert!(
        spans
            .iter()
            .all(|span| span.end <= LINE_BUDGET || span.start >= line1),
        "nothing past the budget on the long line is coloured"
    );
    assert_eq!(capture_at(&spans, &source, "\"tail\"").class, Class::String);
}
