//! Weft (0083): the statically linked presentation grammar drives the
//! real `Highlighter` — token classes, incremental equivalence, rope
//! shapes and incomplete input. Corpus snippets are original Strop-side
//! test inputs; the showcase card is vendored from the MIT-licensed
//! Weft repository with attribution. A colored tree is not source
//! validity.
use super::*;

/// Exercises every revision-3 family in miniature: header, evidence
/// bindings, set algebra with all three operators, spreads, trailing
/// commas, quantifiers, membership/subset comparisons, predicate calls
/// and rule uses.
const FAMILIES: &str = r#"// review header keywords in a comment: context at profile target proposal
review showcase {
    context "realm/checkout" at "release-42";
    profile "weft.symbolic/1";
    target plan = proposal "plan-18";

    asset contract = citation "contract-4";
    shared := {1, 2, 3,};
    wider := {"two", ...shared};
    need contract;
    assume ready: accepts_cursor("sdk-v1");
    check algebra: shared | {9} & wider - {2} == {1, 3};
    check bounded: forall x in shared { x in shared and not (x in wider) or true };
    check cited: cursor_met(plan) by contract, contract;
}
"#;

fn weft_spans(src: &str) -> Vec<Span> {
    clean_spans(std::path::Path::new("x.weft"), &Rope::from_str(src))
}

fn class_at(spans: &[Span], source: &str, token: &str) -> Option<Class> {
    let byte = source
        .find(token)
        .unwrap_or_else(|| panic!("fixture token {token:?}"));
    spans
        .iter()
        .find(|span| span.start <= byte && byte < span.end)
        .map(|span| span.class)
}

#[test]
fn showcase_families_are_distinguished() {
    let spans = weft_spans(FAMILIES);
    // the header comment spells every keyword; code occurrences only
    let code_start = FAMILIES.find('\n').expect("comment line") + 1;
    let in_code = |token: &str| {
        let at = FAMILIES[code_start..]
            .find(token)
            .map(|off| off + code_start)
            .unwrap_or_else(|| panic!("code token {token:?}"));
        // anchored tokens may start with spaces; style lives on the word
        at + (token.len() - token.trim_start().len())
    };
    let keyword = |token: &str| {
        let at = in_code(token);
        let class = spans
            .iter()
            .find(|span| span.start <= at && at < span.end)
            .map(|span| span.class);
        assert_eq!(class, Some(Class::Keyword), "{token:?}");
    };
    for token in [
        "review",
        "context",
        " at \"release",
        "profile",
        "target",
        "proposal",
        "asset",
        "citation",
        "need contract",
        "assume",
        "check algebra",
    ] {
        keyword(token);
    }
    // connectives and the dual-role `in` are keyword-operators; tokens
    // are anchored so `or` does not match inside `cursor`
    let keyword_operator = |token: &str| {
        let at = in_code(token);
        let class = spans
            .iter()
            .find(|span| span.start <= at && at < span.end)
            .map(|span| span.class);
        assert_eq!(class, Some(Class::Keyword), "keyword.operator {token:?}");
    };
    for token in ["and", " or ", " not ", "forall", " in "] {
        keyword_operator(token);
    }
    // set algebra and comparison operators
    for token in ["|", "&", " - {", "==", ":=", "=", "..."] {
        let at = in_code(token);
        let class = spans
            .iter()
            .find(|span| span.start <= at && at < span.end)
            .map(|span| span.class);
        assert_eq!(class, Some(Class::Operator), "operator {token:?}");
    }
    // literals, comments, names by role
    assert_eq!(
        class_at(&spans, FAMILIES, "// review header"),
        Some(Class::Comment)
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "\"realm/checkout\""),
        Some(Class::String)
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "\"sdk-v1\""),
        Some(Class::String)
    );
    assert_eq!(class_at(&spans, FAMILIES, "9}"), Some(Class::Number));
    assert_eq!(class_at(&spans, FAMILIES, "plan-18"), Some(Class::String));
    assert_eq!(
        class_at(&spans, FAMILIES, "shared :="),
        Some(Class::Variable)
    );
    assert_eq!(class_at(&spans, FAMILIES, "ready:"), Some(Class::Variable));
    assert_eq!(
        class_at(&spans, FAMILIES, "bounded:"),
        Some(Class::Variable)
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "accepts_cursor("),
        Some(Class::Function)
    );
    assert_eq!(class_at(&spans, FAMILIES, "plan ="), Some(Class::Variable));
}

#[test]
fn keywords_do_not_leak_into_strings_or_comments() {
    let src = "review r {\n    context \"review check forall in\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    // check assume forall true\n    s := {\"not and or\"};\n}\n";
    let spans = weft_spans(src);
    assert_eq!(
        class_at(&spans, src, "review check forall in"),
        Some(Class::String),
        "keywords inside a string stay string"
    );
    assert_eq!(
        class_at(&spans, src, "// check assume forall true"),
        Some(Class::Comment)
    );
    assert_eq!(class_at(&spans, src, "not and or"), Some(Class::String));
}

#[test]
fn comparison_never_chains_and_recovers_locally() {
    // `a == b == c` is rejected upstream; the presentation tree keeps
    // one operator per comparison, so the third operand errors without
    // swallowing the file.
    let src = "review r {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    check chain: a == b == c;\n    need ok;\n}\n";
    let spans = weft_spans(src);
    assert_eq!(class_at(&spans, src, "need"), Some(Class::Keyword));
    assert_eq!(class_at(&spans, src, "== b"), Some(Class::Operator));
}

#[test]
fn incremental_review_edit_matches_fresh_parse() {
    let path = std::path::Path::new("x.weft");
    let original = "review r {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    check member: 2 in intersection;\n}\n";
    let mut buf = Buffer::from_text(original);
    let mut hl = Highlighter::for_path(path, buf.text()).expect("weft");
    hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");

    // grow the check expression into a quantifier (ASCII-only fixture)
    let at = original.find("2 in intersection").expect("anchor");
    let grown = apply_and_highlight(
        &mut buf,
        &mut hl,
        (at, at + "2 in intersection".len()),
        "forall x in union { x in shared }",
    );
    assert_eq!(grown, clean_spans(path, buf.text()));
    assert_eq!(
        class_at(&grown, &buf.text().to_string(), "forall x"),
        Some(Class::Keyword),
        "incrementally inserted quantifier"
    );

    // edit the string header, then undo it: both match fresh parses
    let text = buf.text().to_string();
    let profile = text.find("\"p\"").expect("profile");
    let edited = apply_and_highlight(
        &mut buf,
        &mut hl,
        (profile, profile + 3),
        "\"weft.symbolic/1\"",
    );
    assert_eq!(edited, clean_spans(path, buf.text()));
    let undone = apply_and_highlight(
        &mut buf,
        &mut hl,
        (profile, profile + "\"weft.symbolic/1\"".len()),
        "\"p\"",
    );
    assert_eq!(undone, clean_spans(path, buf.text()));
    assert_eq!(buf.text().to_string(), text);
}

#[test]
fn incomplete_inputs_highlight_without_panicking() {
    for src in [
        "review half {\n    context \"c\" at \"s\";\n",
        "review half {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    check dangling: a ==\n",
        "review half {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    assume s: \"unterminated\n    need ok;\n",
        "review half {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    s := {\n",
    ] {
        let rope = Rope::from_str(src);
        let mut hl = Highlighter::for_path(std::path::Path::new("x.weft"), &rope)
            .unwrap_or_else(|| panic!("language for {src:?}"));
        let spans = hl
            .highlight(&rope, BufferRevision::new(0), 0, rope.len_bytes())
            .unwrap_or_else(|error| panic!("highlight failed for {src:?}: {error}"));
        for span in &spans {
            assert!(span.end <= rope.len_bytes(), "in bounds: {span:?}");
            assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end));
        }
    }
}

#[test]
fn utf8_crlf_multichunk_spans_are_valid() {
    let mut src = String::from(
        "review r {\r\n    context \"λ 絵 realm\" at \"snap\\n\";\r\n    profile \"p\";\r\n    target t = proposal \"x\";\r\n    // λ 絵 — comment with keywords: review check in\r\n    s := {\"λ 絵 — forall\"};\r\n}\r\n",
    );
    while src.len() < 3000 {
        src.push_str("// filler λ line to span rope chunks\r\n");
    }
    let rope = Rope::from_str(&src);
    assert!(rope.chunks().count() > 1, "precondition: multi-chunk rope");
    let spans = clean_spans(std::path::Path::new("x.weft"), &rope);
    assert!(!spans.is_empty());
    for span in &spans {
        assert!(span.start < span.end && span.end <= rope.len_bytes());
        assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end));
    }
    assert_eq!(
        class_at(&spans, &src, "λ 絵 — comment"),
        Some(Class::Comment)
    );
    assert_eq!(
        class_at(&spans, &src, "λ 絵 — forall"),
        Some(Class::String),
        "multibyte string, no keyword leakage"
    );
    assert_eq!(class_at(&spans, &src, "review r"), Some(Class::Keyword));
}

#[test]
fn weft_markdown_fence_uses_the_injection_route() {
    // A complete review: the grammar's source_file is one review, so a
    // bare fragment would stay the host's raw-code styling.
    let src = "# Notes\n\n```weft\nreview notes {\n    context \"c\" at \"s\";\n    profile \"p\";\n    target t = proposal \"x\";\n    check ready: forall x in s { x in s }\n}\n```\n";
    let spans = clean_spans(std::path::Path::new("notes.md"), &Rope::from_str(src));
    assert_eq!(class_at(&spans, src, "review notes"), Some(Class::Keyword));
    let body = src.rfind(" in s }").expect("quantifier body");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= body + 1 && body + 1 < span.end)
            .map(|span| span.class),
        Some(Class::Keyword),
        "injected fence keeps the keyword-operator class"
    );
}

#[test]
fn weft_line_comment_toggles() {
    let buffer = Buffer::from_text("need contract;\nneed other;\n");
    let edits = crate::comments::toggle(
        Some(std::path::Path::new("review.weft")),
        &buffer,
        Range::linewise(0, buffer.len_bytes()),
    )
    .unwrap();
    let texts: Vec<_> = edits.iter().map(|edit| edit.text.as_str()).collect();
    assert_eq!(texts, ["// ", "// "], "weft uses // line comments");
}
