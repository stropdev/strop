//! The query language end to end in Search, and Ctrl-Space.

use super::super::*;
use strop_core::Buffer;

/// 0051 §3: the qualifier language end to end — language filter +
/// literal content, rows install, accept opens the hit.
#[test]
fn grep_query_language_end_to_end() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn send() {}\nlet x = send;\n").unwrap();
    std::fs::write(dir.path().join("b.py"), "send = 1\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.cwd = dir.path().to_path_buf();
    e.open_picker(Kind::Search);
    e.feed_text("language:rust send");
    e.wait_picker();
    let p = &e.picker.as_ref().unwrap().picker;
    assert_eq!(p.items.len(), 2, "rust-only hits: {:?}", p.items.len());
    assert_eq!(p.rows.len(), 2, "rows install for grep hits");
    assert!(
        p.items.iter().all(|i| !format!("{i:?}").contains("b.py")),
        "no python hit leaks through"
    );
    e.feed(strop_core::frontend_input::Key::Enter);
    assert!(!e.picker_open(), "Enter accepts the first hit");
}

/// An invalid query never broadens the search (0051 §4): no worker
/// launches, the diagnostic shows, and previous state stays.
#[test]
fn invalid_query_never_launches() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.rs"), "fn send() {}\n").unwrap();
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.cwd = dir.path().to_path_buf();
    e.open_picker(Kind::Search);
    e.feed_text("langauge:rust send");
    let p = &e.picker.as_ref().unwrap().picker;
    assert!(
        p.error
            .as_ref()
            .is_some_and(|e| e.contains("unknown filter")),
        "the correction shows: {:?}",
        p.error
    );
    assert_eq!(p.items.len(), 0, "nothing searched");
}

/// 0051 R02: ctrl-space offers manual suggestions from the parse
/// position; Enter replaces the exact token span.
#[test]
fn ctrl_space_suggests_and_enter_completes() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_picker(Kind::Search);
    e.feed_text("lang");
    e.feed(strop_core::frontend_input::Key::CtrlSpace);
    let glue = e.picker.as_ref().unwrap();
    let list = glue.suggestions.as_ref().expect("suggestions open");
    assert_eq!(list.items.len(), 1);
    assert_eq!(list.items[0].insert, "language:");
    e.feed(strop_core::frontend_input::Key::Enter);
    assert_eq!(
        e.picker.as_ref().unwrap().picker.input.text(),
        "language:",
        "the span replaced"
    );
    // Esc in the suggestion list dismisses IT, not the picker
    e.feed(strop_core::frontend_input::Key::CtrlSpace);
    e.feed(strop_core::frontend_input::Key::Esc);
    assert!(e.picker_open(), "esc dismissed only the list");
    assert!(e.picker.as_ref().unwrap().suggestions.is_none());
}

/// The replacement With field and non-query fields keep literal
/// semantics — no suggestions there.
#[test]
fn suggestions_stay_out_of_literal_fields() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    e.open_search(true);
    e.feed_text("foo");
    e.feed(strop_core::frontend_input::Key::Tab); // With field
    e.feed_text("lang");
    e.feed(strop_core::frontend_input::Key::CtrlSpace);
    assert!(
        e.picker.as_ref().unwrap().suggestions.is_none(),
        "no suggestions in the With field"
    );
}
