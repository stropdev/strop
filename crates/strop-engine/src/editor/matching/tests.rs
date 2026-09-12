//! Delimiter matching through the pure Vim authority, owned worker delivery
//! and source-backed collection projection.

use super::*;
use crate::editor::Mode;
use std::path::PathBuf;
use strop_core::worker::WorkerId;

fn scan(text: &str, caret: usize, insert: bool) -> Option<(usize, usize)> {
    let buf = Buffer::from_text(text);
    match_delimiters(&buf, caret, insert, || false).expect("uncancelled scan")
}

fn rust_editor(text: &str) -> Editor {
    let mut e = Editor::new(Buffer::from_text(text));
    e.buf_mut().path = Some(PathBuf::from("x.rs"));
    e
}

/// Ask, pump the worker, ask again: the second call reads the cache.
fn settled(e: &mut Editor, caret: usize, insert: bool) -> [Option<usize>; 2] {
    let doc = e.current();
    let _ = e.pair_highlight(doc, caret, insert);
    e.wait_analysis();
    e.pair_highlight(doc, caret, insert)
}

// ---- unit: the scan itself -------------------------------------------------

#[test]
fn pairs_on_open_and_close() {
    assert_eq!(scan("(a)", 0, false), Some((0, 2)));
    assert_eq!(scan("(a)", 2, false), Some((0, 2)));
    assert_eq!(scan("a[b]c", 1, false), Some((1, 3)));
    assert_eq!(scan("a{b}c", 3, false), Some((1, 3)));
}

#[test]
fn nesting_counts_the_same_kind_only() {
    //        0123456
    assert_eq!(scan("(a(b)c)", 0, false), Some((0, 6)));
    assert_eq!(scan("(a(b)c)", 2, false), Some((2, 4)));
    assert_eq!(scan("(a(b)c)", 4, false), Some((2, 4)));
    // mismatched kinds cross freely, like vim's %
    //        0123
    assert_eq!(scan("([)]", 0, false), Some((0, 2)));
    assert_eq!(scan("([)]", 1, false), Some((1, 3)));
}

#[test]
fn incomplete_code_pairs_nothing() {
    assert_eq!(scan("fn main() {", 10, false), None);
    assert_eq!(scan("}", 0, false), None);
    assert_eq!(scan("((()", 0, false), None);
}

#[test]
fn angle_brackets_are_never_probed() {
    assert_eq!(scan("a<b>", 1, false), None);
    assert_eq!(scan("a<b>", 3, false), None);
    let text = "std::vector<int> v;";
    let lt = text.find('<').expect("fixture");
    assert_eq!(scan(text, lt, false), None);
}

#[test]
fn string_bytes_are_skipped_in_both_directions() {
    //        012345
    let text = r#"f("(")"#;
    assert_eq!(scan(text, 1, false), Some((1, 5)));
    // a close with the mate behind a string scans backward over it
    //        012345
    let text = r#"(")")"#;
    assert_eq!(scan(text, 4, false), Some((0, 4)));
}

#[test]
fn an_unmatched_quoted_probe_cannot_escape_its_string() {
    //        012345
    let text = r#"f("(")"#;
    assert_eq!(scan(text, 3, false), None);
    // Insert mode probes the same unmatched delimiter inside the string.
    assert_eq!(scan(text, 4, true), None);
}

#[test]
fn insert_mode_probes_the_just_typed_byte() {
    // caret past the end (as insert leaves it) probes nothing normally
    assert_eq!(scan("()", 2, false), None);
    assert_eq!(scan("()", 2, true), Some((0, 1)));
    // the caret byte still wins when it is itself a delimiter: '('
    // pairs forward here, while the byte before (']') is unmatched
    assert_eq!(scan("]()", 1, true), Some((1, 2)));
}

#[test]
fn a_cancelled_scan_reports_cancellation() {
    let text = "(".repeat(4096);
    let buf = Buffer::from_text(&text);
    let result = match_delimiters(&buf, 0, false, || true);
    assert!(result.is_err(), "cancellation is not a false unmatched");
}

// ---- end to end through the analysis worker --------------------------------

#[test]
fn rust_pairs_skip_quoted_brackets_and_balance_comment_brackets_like_vim() {
    let text = "fn main() {\n    let s = \"(\";\n    let c = '(';\n    // balanced { } here\n    /* balanced { } here */\n    println!(\"{}\", s);\n}\n";
    let mut e = rust_editor(text);
    // the function body braces pair across every lexical trap
    let open = text.find('{').expect("fixture");
    let close = text.rfind('}').expect("fixture");
    assert_eq!(settled(&mut e, open, false), [Some(open), Some(close)]);
    assert_eq!(settled(&mut e, close, false), [Some(open), Some(close)]);
    // println!'s parens pair
    let p_open = text.find("println!").expect("fixture") + "println!".len();
    let p_close = text.find(", s)").expect("fixture") + 3;
    assert_eq!(
        settled(&mut e, p_open, false),
        [Some(p_open), Some(p_close)]
    );
}

#[test]
fn overlay_and_percent_agree_inside_comments_and_balanced_strings() {
    for text in ["/* { x } */", "let s = \"{ x }\";"] {
        let mut editor = rust_editor(text);
        let open = text.find('{').unwrap();
        let close = text.find('}').unwrap();
        assert_eq!(settled(&mut editor, open, false), [Some(open), Some(close)]);
        editor.set_head(open);
        editor.feed_text("%");
        assert_eq!(editor.head(), close);
    }
}

#[test]
fn escaped_quotes_do_not_end_a_balanced_string() {
    // incomplete code whose only `}` sits BEHIND an escaped quote
    // (buffer text: "\"}" — open, escaped quote, brace, close): an
    // escape-deaf lexer ends the string at the escape and pairs it
    let text = "fn main() {\n    let t = \"\\\"}\";\n";
    let mut e = rust_editor(text);
    let open = text.find('{').expect("fixture");
    assert_eq!(settled(&mut e, open, false), [None, None]);
    let in_string = text.find('}').expect("fixture");
    assert_eq!(settled(&mut e, in_string, false), [None, None]);
}

#[test]
fn cpp_angle_brackets_paint_nothing_and_own_no_job() {
    let mut e = Editor::new(Buffer::from_text("std::vector<int> v;\n"));
    e.buf_mut().path = Some(PathBuf::from("x.cpp"));
    let lt = 12;
    assert_eq!(settled(&mut e, lt, false), [None, None]);
    assert!(
        !e.analysis.pending(),
        "an angle-bracket caret never owns a job"
    );
}

#[test]
fn unmatched_and_unknown_states_cache_a_negative() {
    let mut e = rust_editor("fn main() {\n");
    let open = 10;
    assert_eq!(settled(&mut e, open, false), [None, None]);
    assert!(
        !e.analysis.pending(),
        "the negative result is cached, not resubmitted"
    );
}

// ---- stale results ----------------------------------------------------------

#[test]
fn a_superseded_delivery_is_dropped() {
    let mut e = rust_editor("fn main() {}\n");
    let doc = e.current();
    let caret = 10;
    let _ = e.pair_highlight(doc, caret, false); // owns the job
    let target = AnalysisTarget::Document(doc);
    let mut ticket = match e.analysis.pair.pending.get(&target) {
        Some(pending) => pending.ticket.clone(),
        None => panic!("job owned"),
    };
    // the caret moves before the answer lands: the job is superseded
    let _ = e.pair_highlight(doc, 11, false);
    ticket.request = WorkerId::new(ticket.request.get() + 100);
    e.handle_match(Completion {
        ticket,
        outcome: Outcome::Success(Some(PairMatch {
            first: 10,
            second: 12,
        })),
    });
    assert!(
        e.analysis.pair.cache.get(&target).is_none_or(Vec::is_empty),
        "a superseded delivery caches nothing"
    );
}

#[test]
fn a_delivery_against_an_old_revision_is_dropped() {
    let mut e = rust_editor("fn main() {}\n");
    let doc = e.current();
    let target = AnalysisTarget::Document(doc);
    let revision = e.buf().revision();
    let key = MatchKey {
        target: target.clone(),
        revision,
        caret: 10,
        insert: false,
    };
    let ticket = Ticket {
        request: WorkerId::new(900),
        key: key.clone(),
    };
    e.analysis.pair.pending.insert(
        target.clone(),
        PendingMatch {
            ticket: ticket.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        },
    );
    // the buffer moves on before the answer lands
    e.feed_text("x");
    e.analysis.pair.pending.insert(
        target.clone(),
        PendingMatch {
            ticket: ticket.clone(),
            cancel: Arc::new(AtomicBool::new(false)),
        },
    );
    e.handle_match(Completion {
        ticket,
        outcome: Outcome::Success(Some(PairMatch {
            first: 10,
            second: 12,
        })),
    });
    assert!(
        e.analysis.pair.cache.get(&target).is_none_or(Vec::is_empty),
        "a stale-revision delivery caches nothing"
    );
}

#[test]
fn a_moved_caret_never_sees_the_old_carets_result() {
    let mut e = rust_editor("(a) (b)\n");
    let first = settled(&mut e, 0, false);
    assert_eq!(first, [Some(0), Some(2)]);
    // a different delimiter is a different key: the cached pair for
    // the first paren must not leak into the second one's frame
    let doc = e.current();
    let interim = e.pair_highlight(doc, 4, false);
    assert_eq!(interim, [None, None], "exact-key lookup only");
    assert_eq!(settled(&mut e, 4, false), [Some(4), Some(6)]);
}

// ---- collections ------------------------------------------------------------

/// Two source files with bracket pairs; the grep hits excerpt the
/// bracket lines only (adjacent hit lines merge, nothing else does).
fn collection_fixture() -> (Editor, tempfile::TempDir) {
    let dir = tempfile::tempdir().expect("tempdir");
    let a = dir.path().join("a.txt");
    let b = dir.path().join("b.txt");
    std::fs::write(&a, "fn main() {\n    body\n}\n").expect("write");
    std::fs::write(
        &b,
        "if (x) {\n    body\n    more\n    hidden\n    still_hidden\n}\n",
    )
    .expect("write");
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&a).expect("open a");
    e.open_fixture(&b).expect("open b");
    e.open_picker(strop_picker::Kind::Search);
    let item = |path: &std::path::Path, line: usize, text: &str| strop_picker::Item {
        badge: None,
        text: text.to_string(),
        payload: strop_picker::Payload::Grep {
            path: path.to_path_buf(),
            line,
            col: 1,
            match_len: 2,
            line_text: text.into(),
        },
    };
    let items = vec![
        item(&a, 1, "fn main() {"), // source line 0
        item(&a, 3, "}"),           // source line 2
        item(&b, 1, "if (x) {"),    // source line 0
    ];
    e.picker_items_fixture(items);
    e.feed(crate::editor::Key::CtrlO);
    (e, dir)
}

#[test]
fn collection_pairing_maps_across_excerpts_of_the_same_source() {
    let (mut e, _dir) = collection_fixture();
    // view: title, card top, body("fn main() {"), gap, body("}"), …
    let probe_view = e.buf().line_start(2) + 10; // the '{'
    let mate_view = e.buf().line_start(4); // the '}'
    assert_eq!(
        settled(&mut e, probe_view, false),
        [Some(probe_view), Some(mate_view)],
        "the pair maps across two excerpts of one source"
    );
}

#[test]
fn collection_pairing_never_maps_into_gaps_or_other_sources() {
    let (mut e, _dir) = collection_fixture();
    // b.txt's `if (x) {` pairs with a `}` that no excerpt shows; the
    // ONLY other visible `}` lives in a.txt's card — mapping it would
    // pair across sources
    let b_card_top = 6; // after a's card (top+2 bodies+gap+bottom)
    let probe_view = e.buf().line_start(b_card_top + 1) + 7; // b's '{'
    let served = settled(&mut e, probe_view, false);
    assert_eq!(
        served,
        [Some(probe_view), None],
        "the unexcerpted mate maps nowhere — never another file's row"
    );
}

#[test]
fn collection_chrome_paints_nothing() {
    let (mut e, _dir) = collection_fixture();
    // the gap row between a.txt's excerpts has no source at all
    let gap = e.buf().line_start(3);
    assert_eq!(settled(&mut e, gap, false), [None, None]);
    assert!(!e.analysis.pending(), "chrome never owns a job");
}

#[test]
fn mode_and_view_changes_are_key_changes() {
    // the Insert flag is part of the key: the same caret in Normal and
    // Insert are different jobs, so a mode flip cannot serve stale
    let text = "()\n";
    let mut e = rust_editor(text);
    assert_eq!(settled(&mut e, 1, false), [Some(0), Some(1)]);
    e.mode = Mode::Insert;
    let doc = e.current();
    let interim = e.pair_highlight(doc, 1, true);
    assert_eq!(interim, [None, None], "mode is part of the key");
    assert_eq!(settled(&mut e, 1, true), [Some(0), Some(1)]);
    e.mode = Mode::Normal;
}
