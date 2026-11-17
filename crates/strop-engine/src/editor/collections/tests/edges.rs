//! An excerpt is whole source lines (0108 B4): its end is column 1 of the
//! line below, and an edit made there in the source belongs to that line.
use super::write::assert_view_is_its_sources;
use super::*;
use strop_core::id::DocumentId;

/// A collection of one excerpt near the top of a 30-line file, so lines
/// below the excerpt exist and are not shown.
fn one_excerpt() -> (Editor, DocumentId, DocumentId) {
    let dir = tempfile::tempdir().unwrap().keep();
    let path = dir.join("a.txt");
    let text: String = (0..30).map(|line| format!("line {line}\n")).collect();
    std::fs::write(&path, text).unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.clone());
    let source = e.open_fixture(&path).unwrap();
    e.open_picker(Kind::Search);
    e.picker_items_fixture(vec![Item {
        badge: None,
        text: "a.txt:2".into(),
        facts: Default::default(),
        payload: Payload::Grep {
            location: strop_workspace::ResourceLocation::local(path.clone()),
            line: 2,
            col: 1,
            match_len: 4,
            line_text: "line 1".into(),
        },
    }]);
    e.feed(strop_core::frontend_input::Key::CtrlO);
    let view = e.current();
    assert!(e.collections.contains_key(&view));
    (e, view, source)
}

fn span(e: &Editor, view: DocumentId) -> (usize, usize) {
    let excerpt = &e.collections[&view].excerpts[0];
    (excerpt.start, excerpt.end)
}

/// Make `keys` in the source with the caret on the excerpt's end.
fn below(e: &mut Editor, source: DocumentId, end: usize, keys: &str) {
    e.switch_to(source);
    e.set_head(end);
    type_keys(e, keys);
}

#[test]
fn typing_on_the_line_below_an_excerpt_leaves_it_whole() {
    let (mut e, view, source) = one_excerpt();
    let (start, end) = span(&e, view);
    below(&mut e, source, end, "ix<esc>");
    assert_eq!(e.doc(source).buf.byte_at(end), Some(b'x'));
    assert_eq!(
        span(&e, view),
        (start, end),
        "the line below is not the excerpt's"
    );
    assert_view_is_its_sources(&e, view);
}

#[test]
fn replacing_the_start_of_the_line_below_leaves_the_excerpt_whole() {
    let (mut e, view, source) = one_excerpt();
    let (start, end) = span(&e, view);
    below(&mut e, source, end, "rZ");
    assert_eq!(e.doc(source).buf.byte_at(end), Some(b'Z'));
    assert_eq!(span(&e, view), (start, end), "no partial line is pulled in");
    assert_view_is_its_sources(&e, view);
}

#[test]
fn a_line_opened_through_the_view_at_an_excerpts_end_joins_it() {
    let (mut e, view, source) = one_excerpt();
    let (start, end) = span(&e, view);
    let last = e.doc(source).buf.line_of(end - 1);
    let last_text = e.doc(source).buf.line_text(last);
    let at = current_text(&e).find(last_text.trim_end()).unwrap();
    e.set_head(at);
    type_keys(&mut e, "onew<esc>");
    assert_eq!(span(&e, view), (start, end + "new\n".len()));
    assert_view_is_its_sources(&e, view);
}

/// 0108 B5: a mirrored insert the view refuses (a cursor on chrome) moves
/// nothing; the cursor whose insert applied moves past its own text only.
#[test]
fn a_refused_mirrored_insert_moves_no_cursor() {
    let (mut e, view, _) = one_excerpt();
    let body = current_text(&e).find("line 1").unwrap();
    e.set_head(body);
    e.feed_text("Q");
    e.set_head(0); // the title row: chrome
    let before = current_text(&e);
    type_keys(&mut e, "ix");
    let mut heads = e.sels().heads();
    heads.sort_unstable();
    assert_eq!(heads, vec![0, body + 1]);
    assert_eq!(
        current_text(&e).len(),
        before.len() + 1,
        "only the body took the insert"
    );
    assert_view_is_its_sources(&e, view);
}
