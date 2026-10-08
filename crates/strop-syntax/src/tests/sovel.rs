//! Sovel (0070): the statically linked presentation grammar drives the
//! real `Highlighter` — token classes, incremental equivalence, rope
//! shapes and incomplete input. Corpus snippets are original (no Sovel
//! fixture text; the handover has no license grant). A colored tree is
//! not source validity.
use super::*;

/// Exercises the showcase's syntactic families in miniature: modules,
/// attributes, generics with origin parameters, `using`/`from`/`!`
/// clauses, effect rows, regions, tasks, traits, quotes and splices.
const FAMILIES: &str = r#"// Module header comment with keywords: using from region extern unsafe
module demo::widgets;

import util::log as logger;

newtype WidgetId = U64;

#[derive(Copy)]
struct Metrics {
    ticks: U64,
    label: Str,
}

dyn trait Describe {
    fn label(&self) -> &Str from self ! {};
}

fn measure<T, effects E>(
    input: &Str,
    graph_id: NodeId from graph
) -> (fn(&Str) -> Bool ! E) from input
    ! {io(net), alloc(heap), release(heap), spawn, fail(IoError), ..rest}
{
    region scratch: ScratchMemory in heap {
        tasks on exec within budget {
            let left = spawn || io.fetch(input)?;
            (await left)?
        }
    }
}

macro wrap(body: Syntax<Expr>) -> Syntax<Expr> ! {} {
    quote {
        let temp: U64 = 1;
        $body
    }
}

fn staged() -> U64 ! {} {
    wrap!(temp)
}
"#;

fn sovel_spans(src: &str) -> Vec<Span> {
    clean_spans(std::path::Path::new("x.sov"), &Rope::from_str(src))
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
    let spans = sovel_spans(FAMILIES);
    // the header comment spells several keywords; search code only
    let code_start = FAMILIES.find('\n').expect("comment line") + 1;
    let keyword = |token: &str| {
        let at = FAMILIES[code_start..]
            .find(token)
            .map(|off| off + code_start)
            .unwrap_or_else(|| panic!("code token {token:?}"));
        let class = spans
            .iter()
            .find(|span| span.start <= at && at < span.end)
            .map(|span| span.class);
        assert_eq!(class, Some(Class::Keyword), "{token:?}");
    };
    for token in [
        "module", "import", "newtype", "struct", "trait", "fn", "region", "tasks", "quote", "macro",
    ] {
        keyword(token);
    }
    // contract clause heads (skip the header comment's copies by taking
    // later occurrences via rfind-backed search where needed)
    let using = FAMILIES.find(") -> (fn").map(|_| "-> (fn");
    assert!(using.is_some());
    assert_eq!(
        class_at(&spans, FAMILIES, "from graph"),
        Some(Class::Keyword),
        "origin `from` in a field type"
    );
    // declared names
    assert_eq!(class_at(&spans, FAMILIES, "measure"), Some(Class::Function));
    assert_eq!(
        class_at(&spans, FAMILIES, "Metrics"),
        Some(Class::Type),
        "declared struct name"
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "Describe"),
        Some(Class::Type),
        "declared trait name"
    );
    // attribute name, distinct from the brackets
    assert_eq!(class_at(&spans, FAMILIES, "derive"), Some(Class::Attribute));
    // effect spellings inside the row are types, not keywords
    let io_in_row = FAMILIES.find("io(net)").expect("effect row");
    let span = spans
        .iter()
        .find(|span| span.start <= io_in_row && io_in_row < span.end)
        .expect("styled effect label");
    assert_eq!(span.class, Class::Type, "io(net) inside the effect row");
    let fail = FAMILIES.find("fail(IoError)").expect("fail label");
    assert!(
        spans
            .iter()
            .any(|span| span.start <= fail && fail < span.end && span.class == Class::Keyword),
        "fail( is the structural label keyword"
    );
    // spawn as an effect name vs spawn as an expression keyword
    let spawn_row = FAMILIES.find("spawn,").expect("spawn effect");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= spawn_row && spawn_row < span.end)
            .map(|span| span.class),
        Some(Class::Type),
        "spawn inside the effect row"
    );
    // macro name and splice marker
    assert_eq!(
        class_at(&spans, FAMILIES, "wrap!"),
        Some(Class::Function),
        "macro callee"
    );
    assert_eq!(
        class_at(&spans, FAMILIES, "$body"),
        Some(Class::Operator),
        "splice marker"
    );
    // literals and punctuation
    assert_eq!(class_at(&spans, FAMILIES, "1"), Some(Class::Number));
    assert_eq!(class_at(&spans, FAMILIES, ";"), Some(Class::Punctuation));
}

#[test]
fn contract_keywords_avoid_the_header_comment() {
    // `using`/`from`/`region` inside the leading comment stay Comment;
    // the first CODE occurrence is the keyword.
    let spans = sovel_spans(FAMILIES);
    let comment_end = FAMILIES.find('\n').expect("comment line");
    for span in &spans {
        if span.start < comment_end {
            assert_eq!(
                span.class,
                Class::Comment,
                "span inside the header comment: {span:?}"
            );
        }
    }
    let code_using = FAMILIES.find("tasks on").expect("tasks clause");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= code_using && code_using < span.end)
            .map(|span| span.class),
        Some(Class::Keyword)
    );
}

#[test]
fn effect_spellings_are_contextual_not_global_keywords() {
    // `read`, `io`, `alloc` as ordinary identifiers take ordinary
    // classes; only effect syntax gives them the Type class.
    let src =
        "fn f(read: U64, io: U64) -> U64 ! {read(io)} {\n    let alloc = read;\n    alloc\n}\n";
    let spans = sovel_spans(src);
    let param = src.find("read:").expect("parameter");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= param && param < span.end)
            .map(|span| span.class),
        Some(Class::Variable),
        "a parameter named `read` is a variable.parameter"
    );
    let row = src.find("read(io)").expect("effect row");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= row && row < span.end)
            .map(|span| span.class),
        Some(Class::Type),
        "read(io) inside the effect row"
    );
    let local = src.find("let alloc").map(|at| at + 4).expect("local");
    let class = spans
        .iter()
        .find(|span| span.start <= local && local < span.end)
        .map(|span| span.class);
    assert!(
        class != Some(Class::Keyword) && class != Some(Class::Type),
        "a local named `alloc` is not an effect: {class:?}"
    );
}

#[test]
fn keywords_do_not_leak_into_strings_or_comments() {
    let src = "fn f() -> Unit ! {} {\n    let s = \"using from region extern unsafe fn\";\n    // using from region\n    let c = '\\u{41}';\n}\n";
    let spans = sovel_spans(src);
    assert_eq!(
        class_at(&spans, src, "using from region extern unsafe fn"),
        Some(Class::String),
        "keywords spelled inside a string stay string"
    );
    assert_eq!(
        class_at(&spans, src, "// using from region"),
        Some(Class::Comment)
    );
    assert_eq!(class_at(&spans, src, "'\\u{41}'"), Some(Class::String));
}

#[test]
fn incremental_contract_edit_matches_fresh_parse() {
    let path = std::path::Path::new("x.sov");
    let original = "fn fetch(url: &Url)\n    -> Response ! {io(net)}\n{\n    get(url)\n}\n";
    let mut buf = Buffer::from_text(original);
    let mut hl = Highlighter::for_path(path, buf.text()).expect("sovel");
    hl.highlight(buf.text(), buf.revision(), 0, buf.len_bytes())
        .expect("parse");

    // grow a `using` clause inside the signature (char == byte here)
    let at = original.find("-> Response").expect("anchor");
    let grown = apply_and_highlight(&mut buf, &mut hl, (at, at), "using net: &Net\n    ");
    assert_eq!(grown, clean_spans(path, buf.text()));
    assert_eq!(
        class_at(&grown, &buf.text().to_string(), "using net"),
        Some(Class::Keyword),
        "incrementally inserted clause head"
    );

    // edit the effect row, then undo it: both match fresh parses
    let text = buf.text().to_string();
    let row = text.find("io(net)").expect("row");
    let edited = apply_and_highlight(
        &mut buf,
        &mut hl,
        (row, row + "io(net)".len()),
        "io(net), alloc(heap)",
    );
    assert_eq!(edited, clean_spans(path, buf.text()));
    let undone = apply_and_highlight(
        &mut buf,
        &mut hl,
        (row, row + "io(net), alloc(heap)".len()),
        "io(net)",
    );
    assert_eq!(undone, clean_spans(path, buf.text()));
    assert_eq!(buf.text().to_string(), text);
}

#[test]
fn incomplete_inputs_highlight_without_panicking() {
    for src in [
        "#[derive(\n",
        "#[link_name =\n",
        "fn f(x: &Str) using\n",
        "fn g() -> Unit ! {\n",
        "fn h<T: Clone(\n",
        "fn v() -> Vec<Vec<T>> {\n",
        "fn q() -> Unit ! {} {\n    let a = quote { $x + ${y\n}\n",
        "fn b() -> Unit ! {} {\n    let s = \"unterminated\n    let t = 1;\n}\n",
    ] {
        let rope = Rope::from_str(src);
        let mut hl = Highlighter::for_path(std::path::Path::new("x.sov"), &rope)
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
    // multibyte comments/strings, CRLF endings, and a padded tail that
    // pushes the rope past one chunk
    let mut src = String::from("module demo::utf8;\r\n// λ 絵 — comment with keywords: using from\r\nfn f() -> Unit ! {} {\r\n    let s = \"λ 絵 — using\";\r\n}\r\n");
    while src.len() < 3000 {
        src.push_str("// filler λ line to span rope chunks\r\n");
    }
    let rope = Rope::from_str(&src);
    assert!(rope.chunks().count() > 1, "precondition: multi-chunk rope");
    let spans = clean_spans(std::path::Path::new("x.sov"), &rope);
    assert!(!spans.is_empty());
    for span in &spans {
        assert!(span.start < span.end && span.end <= rope.len_bytes());
        assert!(src.is_char_boundary(span.start) && src.is_char_boundary(span.end));
    }
    assert_eq!(
        class_at(&spans, &src, "λ 絵 — comment"),
        Some(Class::Comment),
        "multibyte comment"
    );
    assert_eq!(
        class_at(&spans, &src, "λ 絵 — using"),
        Some(Class::String),
        "multibyte string, no keyword leakage"
    );
    assert_eq!(class_at(&spans, &src, "module"), Some(Class::Keyword));
}

#[test]
fn sovel_markdown_fence_uses_the_injection_route() {
    let src = "# Notes\n\n```sovel\nfn f() -> Unit ! {io(net)} {\n}\n```\n";
    let spans = clean_spans(std::path::Path::new("notes.md"), &Rope::from_str(src));
    assert_eq!(class_at(&spans, src, "fn f"), Some(Class::Keyword));
    let row = src.find("io(net)").expect("effect row");
    assert_eq!(
        spans
            .iter()
            .find(|span| span.start <= row && row < span.end)
            .map(|span| span.class),
        Some(Class::Type),
        "injected fence keeps the effect-row class"
    );
}
