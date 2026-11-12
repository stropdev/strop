//! The collect flow (0097): where `Ctrl-O` puts you, how the hits are
//! stepped, what `Enter` opens, and what the collection says about itself.
use super::*;
use strop_core::frontend_input::Key;
use strop_core::id::DocumentId;

const A: &str = "one\nfn hit() { hit(); }\nthree\nfour\nfive\nsix\nseven\neight\nnine\nhit ten\n";
const B: &str = "x\nb hit\n";

/// Search has found `query` in a project of two open files. For `hit`
/// that is four hits: two on one line of `a.txt`, one further down it,
/// one in `b.txt`. Nothing is collected yet.
fn searching(root: &std::path::Path, query: &str) -> (Editor, DocumentId, DocumentId) {
    let a = root.join("a.txt");
    let b = root.join("b.txt");
    std::fs::write(&a, A).unwrap();
    std::fs::write(&b, B).unwrap();
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), root.to_path_buf());
    let a_id = e.open_fixture(&a).unwrap();
    let b_id = e.open_fixture(&b).unwrap();
    search(&mut e, query);
    (e, a_id, b_id)
}

/// `Space /`, the field emptied, `query` typed, the search finished.
fn search(e: &mut Editor, query: &str) {
    e.open_search(false);
    e.feed(Key::CtrlU);
    e.paste_bracketed(query);
    e.wait_picker();
}

/// The text from the cursor to the end of its line.
fn under_cursor(e: &Editor) -> String {
    let end = e.buf().line_end(e.buf().line_of(e.head()));
    e.buf().text().byte_slice(e.head()..end).to_string()
}

fn collections(e: &Editor) -> Vec<String> {
    let mut names: Vec<String> = e
        .collections
        .keys()
        .map(|id| e.doc(*id).buf.name.clone().unwrap())
        .collect();
    names.sort();
    names
}

#[test]
fn ctrl_o_lands_on_the_hit_under_the_bar() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _, _) = searching(dir.path(), "hit");
    e.feed(Key::Down);
    e.feed(Key::Down);
    e.feed(Key::CtrlO);
    assert_eq!(under_cursor(&e), "hit ten", "the third row's matched text");
    assert_eq!(
        e.message,
        "4 hits in 2 files — ]q next, Enter opens the file, :w writes"
    );
    // that hit is the current one: from a file, `]q` goes to the one after
    e.feed(Key::CtrlO);
    e.feed_text("]q");
    assert_eq!(e.message, "hit 4 of 4: b.txt:2");
    assert_eq!(under_cursor(&e), "hit");
}

#[test]
fn the_collection_says_what_was_searched_and_counts_in_words() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _, _) = searching(dir.path(), "hit");
    e.feed(Key::CtrlO);
    assert_eq!(e.buf().name.as_deref(), Some("collection: search hit"));
    let text = current_text(&e);
    let rows: Vec<&str> = text.lines().collect();
    assert_eq!(rows[0], "search hit — 4 hits in 2 files");
    assert_eq!(rows[1], "");
    assert_eq!(
        rows[2], "a.txt",
        "a heading holds the path; the count is drawn"
    );
    assert!(rows.contains(&"3 lines not shown"), "{text}");
    assert!(rows.contains(&"b.txt"), "{text}");

    // one hit, one file, and a list with no query of its own
    let mut e = Editor::new_in(Buffer::from_text("scratch\n"), dir.path().to_path_buf());
    e.open_fixture(&dir.path().join("b.txt")).unwrap();
    e.locations_fixture(&[(dir.path().join("b.txt"), 1)], vec!["b hit".into()]);
    e.feed(Key::CtrlO);
    assert_eq!(e.buf().name.as_deref(), Some("collection: locations"));
    assert!(
        current_text(&e).starts_with("locations — 1 hit in 1 file\n\nb.txt\n"),
        "{}",
        current_text(&e)
    );
    assert!(e.message.starts_with("1 hit in 1 file — "), "{}", e.message);
    assert_eq!(under_cursor(&e), "b hit");
}

#[test]
fn bracket_q_steps_every_hit_from_the_cursor_and_stays_in_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _, _) = searching(dir.path(), "hit");
    e.feed(Key::CtrlO);
    let list = e.current();
    assert_eq!(under_cursor(&e), "hit() { hit(); }");
    // two hits in one excerpt line are two steps
    e.feed_text("]q");
    assert_eq!(under_cursor(&e), "hit(); }");
    assert_eq!(e.message, "hit 2 of 4: a.txt:2");
    e.feed_text("]q]q");
    assert_eq!(under_cursor(&e), "hit");
    assert_eq!(e.message, "hit 4 of 4: b.txt:2");
    let last = e.head();
    e.feed_text("]q");
    assert_eq!((e.message.as_str(), e.head()), ("last hit", last));
    // from wherever the cursor is, not from the hit last stepped to
    e.feed_text("gg]q");
    assert_eq!(e.message, "hit 1 of 4: a.txt:2");
    e.feed_text("/ten<cr>[q");
    assert_eq!(e.message, "hit 3 of 4: a.txt:10");
    e.feed_text("2[q");
    assert_eq!(e.message, "hit 1 of 4: a.txt:2");
    let first = e.head();
    e.feed_text("[q");
    assert_eq!((e.message.as_str(), e.head()), ("first hit", first));
    assert_eq!(e.current(), list, "never left the collection");
}

#[test]
fn enter_opens_the_file_at_the_cursor_and_ctrl_o_returns() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, a, b) = searching(dir.path(), "hit");
    e.feed(Key::CtrlO);
    let list = e.current();

    // on a hit: the same byte in the file
    e.feed_text("]q");
    e.feed(Key::Enter);
    assert_eq!(e.current(), a);
    assert_eq!(
        (e.buf().line_of(e.head()), under_cursor(&e).as_str()),
        (1, "hit(); }")
    );
    // it is the current hit: `]q` in the file goes on from it
    e.feed_text("]q");
    assert_eq!(e.message, "hit 3 of 4: a.txt:10");
    e.feed(Key::CtrlO);
    e.feed(Key::CtrlO);
    assert_eq!(e.current(), list, "Ctrl-O returns to the collection");

    // on a context line: that line, and the hit above it becomes current
    e.feed_text("/three<cr>");
    e.feed(Key::Enter);
    assert_eq!(e.current(), a);
    assert_eq!(under_cursor(&e), "three");
    e.feed_text("]q");
    assert_eq!(e.message, "hit 3 of 4: a.txt:10");
    e.feed_text(":copen<cr>");

    // on a card's top border: the file, at the card's first line
    e.feed_text("/b.txt<cr>");
    e.feed(Key::Enter);
    assert_eq!(e.current(), b);
    assert_eq!(e.buf().line_of(e.head()), 0);
    e.feed(Key::CtrlO);

    // on a row that shows no file, Enter is the motion it always was
    e.feed_text("gg");
    e.feed(Key::Enter);
    assert_eq!(e.current(), list);
    assert_eq!(e.buf().line_of(e.head()), 1);
}

#[test]
fn collecting_the_same_search_again_refreshes_its_collection() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, a, _) = searching(dir.path(), "hit");
    e.feed(Key::CtrlO);
    let first = e.current();
    type_keys(&mut e, "cwmiss<esc>");
    assert!(e.doc(a).buf.text().to_string().contains("fn miss()"));
    e.feed(Key::CtrlO);

    // the same search, from a file: the same buffer, the hits of now
    search(&mut e, "hit");
    e.feed(Key::Down);
    e.feed(Key::CtrlO);
    assert_eq!(e.current(), first);
    assert_eq!(collections(&e), ["collection: search hit"]);
    assert!(current_text(&e).starts_with("search hit — 3 hits in 2 files\n"));
    assert_eq!(under_cursor(&e), "hit ten", "on the picked hit");
    // its history started over: `u` has nothing of the old collection
    type_keys(&mut e, "u");
    assert!(e.doc(a).buf.text().to_string().contains("fn miss()"));

    // another search is another collection
    e.feed(Key::CtrlO);
    search(&mut e, "ten");
    e.feed(Key::CtrlO);
    assert_ne!(e.current(), first);
    assert_eq!(
        collections(&e),
        ["collection: search hit", "collection: search ten"]
    );
    // and the hit keys act on the one in view, then on the one last in view
    e.switch_to(first);
    e.feed_text("gg]q");
    e.feed(Key::Enter);
    e.feed_text(":copen<cr>");
    assert_eq!(e.current(), first);
}

#[test]
fn nothing_to_collect_says_why() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _, _) = searching(dir.path(), "hit");
    for _ in 0..2 {
        e.feed(Key::CtrlD);
        e.feed(Key::Down);
        e.feed(Key::Down);
        e.feed(Key::Down);
    }
    e.feed(Key::CtrlO);
    assert!(e.picker_open());
    assert_eq!(e.message, "nothing to collect — every hit is excluded");

    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Search);
    e.feed(Key::CtrlO);
    assert_eq!(e.message, "nothing to collect — the list is empty");
    e.open_picker(Kind::Buffers);
    e.feed(Key::CtrlO);
    assert_eq!(
        e.message,
        "Ctrl-O collects search results, references or diagnostics"
    );
}

/// 0097 §3.3: one key opens the file and one pair steps the hits.
#[test]
fn the_superseded_keys_are_gone() {
    let dir = tempfile::tempdir().unwrap();
    let (mut e, _, _) = searching(dir.path(), "hit");
    e.feed(Key::CtrlO);
    let list = e.current();
    let at = e.head();
    for keys in ["g ", "]e", "[e", ":collection source<cr>"] {
        e.feed_text(keys);
        assert_eq!((e.current(), e.head()), (list, at), "{keys}");
    }
}
