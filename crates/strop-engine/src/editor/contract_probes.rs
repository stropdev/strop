//! Consumer-facing regressions from the earlier review. Geometry and worker
//! transition oracles live beside their respective implementations.

use super::*;
use strop_core::Buffer;

fn syntax_editor() -> Editor {
    let mut e = Editor::new(Buffer::from_text("fn demo() {\n    let x = 1;\n}\n"));
    e.buf_mut().path = Some(std::path::PathBuf::from("audit.rs"));
    e
}

fn spans(e: &mut Editor) -> Vec<strop_syntax::Span> {
    e.analysis_fixture().spans.clone()
}

fn fresh_spans(e: &Editor) -> Vec<strop_syntax::Span> {
    let mut h =
        strop_syntax::Highlighter::for_path(std::path::Path::new("audit.rs"), e.buf().text())
            .unwrap();
    h.highlight(e.buf().text(), e.buf().revision(), 0, e.buf().len_bytes())
        .unwrap()
}

#[test]
fn review_highlight_updates_during_insert() {
    let mut e = syntax_editor();
    spans(&mut e);
    e.feed_text("i//");
    assert_eq!(
        spans(&mut e),
        fresh_spans(&e),
        "a visible insert-mode edit must invalidate syntax"
    );
}

#[test]
fn review_highlight_updates_after_undo() {
    let mut e = syntax_editor();
    spans(&mut e);
    e.feed_text("i//<esc>");
    spans(&mut e);
    e.feed_text("u");
    assert_eq!(
        spans(&mut e),
        fresh_spans(&e),
        "undo must invalidate syntax"
    );
}

#[test]
fn review_split_from_empty_scratch_retains_live_panes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "file\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.feed_text(&format!(":vs {}<cr>", path.display()));
    e.wait_io().unwrap();
    e.feed_text("<c-w>h");
    assert_eq!(e.panes.len(), 2);
    assert!(e.panes.iter().all(|p| e.docs.get(p.doc).is_some()));
}

#[test]
fn review_clipboard_reply_preserves_destination() {
    let mut e = Editor::new(Buffer::from_text("first\n"));
    let first = e.current();
    let ticket = strop_core::worker::Ticket {
        request: e.worker_ids.allocate().unwrap(),
        key: ClipboardKey { document: first },
    };
    e.clip_paste_pending = Some((false, ticket.clone()));
    let second = e
        .docs
        .insert(Document::scratch(Buffer::from_text("second\n")));
    e.switch_to(second);
    e.handle_clipboard(strop_core::worker::Completion {
        ticket,
        outcome: strop_core::worker::Outcome::Success("CLIP".into()),
    });
    assert_eq!(
        e.buf().text().to_string(),
        "second\n",
        "paste must retain its initiating document"
    );
}

#[test]
fn review_unicode_filename_matches_itself() {
    assert!(
        strop_picker::fuzzy_score("日本", "src/日本.rs").is_some(),
        "Unicode filenames must be searchable"
    );
}

#[test]
fn review_unicode_filename_reports_char_columns() {
    let (_, cols) = strop_picker::fuzzy_score("m", "é/main.rs").unwrap();
    assert_eq!(
        cols,
        vec![2],
        "matched columns must be character indexes, not UTF-8 byte indexes"
    );
}

#[cfg(unix)]
#[test]
fn review_save_preserves_symlink() {
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().join("target.txt");
    let link = dir.path().join("link.txt");
    std::fs::write(&target, "old\n").unwrap();
    std::os::unix::fs::symlink(&target, &link).unwrap();
    let mut b = Buffer::open(&link).unwrap();
    b.edit().insert(0, "new ").unwrap();
    let receipt = b.prepare_save(None, false).unwrap().execute().unwrap();
    assert!(b.accept_save(receipt));
    assert!(
        std::fs::symlink_metadata(&link)
            .unwrap()
            .file_type()
            .is_symlink(),
        "saving a symlink must preserve the link"
    );
    assert_eq!(std::fs::read_to_string(&target).unwrap(), "new old\n");
}

#[test]
fn review_preview_contract_is_honest() {
    // 0023's narrowed contract: instant operator+object chords (ci[)
    // execute at the completing key — there is no inspection window,
    // and the preview claims none. Composition windows preview.
    let mut e = Editor::new(Buffer::from_text("f[hello]\n"));
    e.feed_text("ll");
    let before = e.buf().text().to_string();
    e.feed(Key::Char('c'));
    e.feed(Key::Char('i'));
    assert!(
        e.preview().unwrap().is_none(),
        "no fake preview for ci — the object isn't chosen yet"
    );
    e.feed(Key::Char('['));
    assert_ne!(
        e.buf().text().to_string(),
        before,
        "ci[ executes at the completing key"
    );
    // the composition window previews (d/foo shows its target as you type)
    let mut e = Editor::new(Buffer::from_text("hello world foo\n"));
    e.feed_text("d/wo");
    assert!(
        e.preview().unwrap().is_some(),
        "the search composition previews its target"
    );
}

#[test]
fn review_marks_follow_undo() {
    let mut e = Editor::new(Buffer::from_text("one\nTARGET\n"));
    e.feed_text("jma");
    e.feed_text("ggOnew<esc>");
    assert_eq!(e.buf().line_of(e.marks[&'a'].1), 2);
    e.feed_text("u");
    assert_eq!(
        e.marks[&'a'].1, 4,
        "undo must map the mark back with its text"
    );
}

#[test]
fn review_marks_map_in_two_documents_with_equal_history_depth() {
    let mut e = Editor::new(Buffer::from_text("a\nTARGET\n"));
    e.feed_text("jma");
    e.feed_text("ggOnew<esc>");
    let second = e
        .docs
        .insert(Document::scratch(Buffer::from_text("b\nTARGET\n")));
    e.switch_to(second);
    e.set_head(0);
    e.feed_text("jmb");
    e.feed_text("ggOnew<esc>");
    assert_eq!(
        e.buf().line_of(e.marks[&'b'].1),
        2,
        "anchor watermark must be per document"
    );
}

#[test]
fn review_ranged_yank_updates_unnamed_register() {
    let mut e = Editor::new(Buffer::from_text("one\ntwo\n"));
    e.feed_text(":1y<cr>p");
    assert_eq!(e.buf().text().to_string(), "one\none\ntwo\n");
}

#[test]
fn review_stale_replace_range_is_utf8_safe() {
    let mut e = Editor::new(Buffer::from_text("界foo\n"));
    let doc = e.current();
    let hit = crate::editor::picker::ReplacementHit {
        line: 1,
        col: 2,
        match_len: 3,
        text: "afoo".into(),
    };
    let (_, applied, stale) = e.replace_in_buffer_pub(doc, &[hit], "bar");
    assert_eq!((applied, stale), (0, 1));
}

#[test]
fn review_operator_find_accepts_digit_as_target() {
    let mut e = Editor::new(Buffer::from_text("ab2cd\n"));
    e.feed_text("df2");
    assert_eq!(
        e.buf().text().to_string(),
        "cd\n",
        "a digit after f is a target character"
    );
}
