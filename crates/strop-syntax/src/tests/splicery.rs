//! Splicery (0083): the statically linked presentation grammar (with
//! its heredoc scanner) drives the real `Highlighter` — token classes,
//! heredoc/rx byte domains, incremental equivalence and incomplete
//! input. Corpus snippets are original Strop-side test inputs; a
//! colored tree is not a valid recipe.
use super::*;

/// Exercises the /1 families in miniature: edit with raw profile,
/// every selector shape, every payload piece, iteration with rx
/// captures and a heredoc whose body quotes comment and keyword
/// spellings.
const FAMILIES: &str = r#"# header comment with verbs: edit create rename remove each in expect
edit "sdk/src/client.rs" as bytes {
  @call := one text "transport.send(request)"
  @zone := one line "[zone]" within @call
  @span := range [8, 34)
  replace @span with ["trace_send(", keep(@call), indent("\t", <<'TAIL'
# hash and keywords inside the body: each in expect
"quoted" line
TAIL
)]
  each @r in line_matches rx"^route[ \t]+(?<verb>[A-Z]+)$" expect 3 {
    replace @r with [keep(@r.verb), hex"0a"]
  }
}
"#;

fn splicery_spans(src: &str) -> Vec<Span> {
    clean_spans(std::path::Path::new("x.spl"), &Rope::from_str(src))
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
    let spans = splicery_spans(FAMILIES);
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
        "edit",
        "as",
        "bytes",
        "one",
        "within",
        "range",
        "replace",
        "with",
        "keep",
        " in line_matches",
        " each @r",
        "line_matches",
        " expect 3",
    ] {
        keyword(token);
    }
    // `text` is a keyword only inside the one-selector
    assert_eq!(class_at(&spans, FAMILIES, "one text"), Some(Class::Keyword));
    // handles: binder is a parameter, reference is a variable, group
    // suffix is an attribute; the marker itself is an operator
    let binder = in_code("call :=");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= binder && binder < span.end)
            .map(|span| span.class),
        Some(Class::Variable),
        "binding handle name"
    );
    let keep_ref = in_code("keep(@call)");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= keep_ref + 6 && keep_ref + 6 < span.end)
            .map(|span| span.class),
        Some(Class::Variable),
        "reference handle"
    );
    assert_eq!(class_at(&spans, FAMILIES, "verb)"), Some(Class::Attribute));
    assert_eq!(
        class_at(&spans, FAMILIES, "@call :="),
        Some(Class::Operator)
    );
    assert_eq!(class_at(&spans, FAMILIES, ":= one"), Some(Class::Operator));
    // literals: string / rx / hex share the String class via distinct
    // capture heads; nats are numbers
    assert_eq!(
        class_at(&spans, FAMILIES, "\"transport.send(request)\""),
        Some(Class::String)
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "rx\"^route"),
        Some(Class::String),
        "rx body is one raw string token"
    );
    assert_eq!(class_at(&spans, FAMILIES, "hex\"0a\""), Some(Class::String));
    assert_eq!(class_at(&spans, FAMILIES, "8,"), Some(Class::Number));
    assert_eq!(
        class_at(&spans, FAMILIES, "# header comment"),
        Some(Class::Comment)
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "<<'TAIL'"),
        Some(Class::Punctuation)
    );
}

#[test]
fn heredoc_body_is_raw_string_and_does_not_leak() {
    let src = "edit \"d\" {\n  @a := one text \"x\"\n  replace @a with <<'POLICY'\n# hash and verbs: each in expect keep\n\"quoted\" text line\nPOLICY\n}\n";
    let spans = splicery_spans(src);
    // body is one raw string span: comment/keyword spellings stay string
    assert_eq!(
        class_at(&spans, src, "# hash and verbs"),
        Some(Class::String),
        "heredoc body is not a comment"
    );
    assert_eq!(
        class_at(&spans, src, "\"quoted\" text line"),
        Some(Class::String)
    );
    // opener and closing tag line are punctuation, body excluded
    let body = src.find("# hash and verbs").expect("body");
    let body_span = spans
        .iter()
        .find(|span| span.start <= body && body < span.end)
        .expect("body span");
    assert!(body_span.end <= src.find("POLICY\n").expect("closer"));
    assert_eq!(
        class_at(&spans, src, "<<'POLICY'"),
        Some(Class::Punctuation)
    );
    // empty body is valid: opener immediately followed by the closer
    let empty = "create \"e\" with <<'EMPTY'\nEMPTY\n";
    let spans = splicery_spans(empty);
    assert_eq!(
        class_at(&spans, empty, "<<'EMPTY'"),
        Some(Class::Punctuation)
    );
    assert_eq!(class_at(&spans, empty, "EMPTY\n"), Some(Class::Punctuation));
    // a lookalike with leading whitespace is content, not a closer
    let lookalike = "create \"f\" with <<'TAG'\n  TAG indented\nTAG\n";
    let spans = splicery_spans(lookalike);
    assert_eq!(
        class_at(&spans, lookalike, "  TAG indented"),
        Some(Class::String),
        "indented lookalike stays body"
    );
}

#[test]
fn contextual_spellings_stay_names() {
    // `text`/`line`/`like`/`within`/`expect` as handle names are
    // variables; only the fixed productions make them keywords.
    let src = "edit \"d\" {\n  @text := one line \"x\"\n  @like := one text \"y\"\n  replace @text with keep(@like)\n}\n";
    let spans = splicery_spans(src);
    assert_eq!(class_at(&spans, src, "text :="), Some(Class::Variable));
    assert_eq!(class_at(&spans, src, "like :="), Some(Class::Variable));
    assert_eq!(class_at(&spans, src, "one line"), Some(Class::Keyword));
    assert_eq!(class_at(&spans, src, "one text"), Some(Class::Keyword));
}

#[test]
fn incremental_edits_inside_heredoc_and_rx_match_fresh_parse() {
    let path = std::path::Path::new("x.spl");
    let original = "edit \"d\" {\n  @r := one text \"x\"\n  replace @r with <<'BODY'\nfirst line\nBODY\n  each @m in line_matches rx\"^a\" expect 1 {\n    replace @m with keep(@m)\n  }\n}\n";
    let mut buf = Buffer::from_text(original);
    let mut hl = Highlighter::for_path(path, buf.text()).expect("splicery");
    hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");

    // edit inside the heredoc body (ASCII-only fixture)
    let at = original.find("first line").expect("body");
    let grown = apply_and_highlight(
        &mut buf,
        &mut hl,
        (at, at + "first line".len()),
        "edited # line",
    );
    assert_eq!(grown, clean_spans(path, buf.text()));
    assert_eq!(
        class_at(&grown, &buf.text().to_string(), "edited # line"),
        Some(Class::String),
        "edited heredoc body stays one raw string"
    );

    // edit inside the rx body, then undo: both match fresh parses
    let text = buf.text().to_string();
    let rx = text.find("rx\"^a\"").expect("rx");
    let edited = apply_and_highlight(&mut buf, &mut hl, (rx + 3, rx + 5), "^ab+");
    assert_eq!(edited, clean_spans(path, buf.text()));
    let undone = apply_and_highlight(&mut buf, &mut hl, (rx + 3, rx + 7), "^a");
    assert_eq!(undone, clean_spans(path, buf.text()));
    assert_eq!(buf.text().to_string(), text);
}

#[test]
fn incomplete_inputs_highlight_without_panicking() {
    for src in [
        "edit \"half.txt\" {\n  @a := one text \"open\"\n",
        "edit \"half.txt\" {\n  @a := one text\n",
        "edit \"half.txt\" {\n  @a := range [1,\n}\n",
        "edit \"half.txt\" {\n  @a := one text \"no close\n  replace @a with \"x\"\n}\n",
        "edit \"half.txt\" {\n  replace @a with <<'EOF'\nstill typing\n",
        "edit \"half.txt\" {\n  @a := one text \"x\"\n  replace @a with [keep(@a),\n}\n",
        "@a := one text \"x\"\nreplace @a with keep(@a\n",
    ] {
        let rope = Rope::from_str(src);
        let mut hl = Highlighter::for_path(std::path::Path::new("x.spl"), &rope)
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
        "edit \"ü/λ.txt\" {\r\n  # λ 絵 — comment with verbs: each in expect\r\n  @a := one text \"λ 絵 — keep\"\r\n  replace @a with <<'BODY'\r\nλ 絵 body line\r\nBODY\r\n}\r\n",
    );
    while src.len() < 3000 {
        src.push_str("# filler λ line to span rope chunks\r\n");
    }
    let rope = Rope::from_str(&src);
    assert!(rope.chunks().count() > 1, "precondition: multi-chunk rope");
    let spans = clean_spans(std::path::Path::new("x.spl"), &rope);
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
        class_at(&spans, &src, "λ 絵 — keep"),
        Some(Class::String),
        "multibyte string, no verb leakage"
    );
    assert_eq!(
        class_at(&spans, &src, "λ 絵 body line"),
        Some(Class::String),
        "CRLF heredoc body stays string across chunks"
    );
}

#[test]
fn splicery_markdown_fence_uses_the_injection_route() {
    let src = "# Notes\n\n```splicery\nedit \"f\" {\n  @a := one text \"x\"\n}\n```\n";
    let spans = clean_spans(std::path::Path::new("notes.md"), &Rope::from_str(src));
    assert_eq!(class_at(&spans, src, "edit \"f\""), Some(Class::Keyword));
    let handle = src.find("@a").expect("handle");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= handle && handle < span.end)
            .map(|span| span.class),
        Some(Class::Operator),
        "injected fence keeps the @ marker operator class"
    );
}

#[test]
fn splicery_line_comment_toggles() {
    let buffer = Buffer::from_text("@a := one text \"x\"\nreplace @a with \"y\"\n");
    let edits = crate::comments::toggle(
        Some(std::path::Path::new("recipe.spl")),
        &buffer,
        Range::linewise(0, buffer.len_bytes()),
    )
    .unwrap();
    let texts: Vec<_> = edits.iter().map(|edit| edit.text.as_str()).collect();
    assert_eq!(texts, ["# ", "# "], "splicery uses # line comments");
}
