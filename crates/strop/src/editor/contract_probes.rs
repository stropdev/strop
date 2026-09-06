//! Contract probes (review 4, 0023): the reviewer's reproductions,
//! adopted verbatim as the acceptance suite — each names a guarantee.
//! Probes that pinned a bug have their fix's regression coverage here.
//! No probes are removed when fixed; they stay green.

use super::*;
use strop_core::Buffer;

fn syntax_editor() -> Editor {
    let mut e = Editor::new(Buffer::from_text("fn demo() {\n    let x = 1;\n}\n"));
    e.cur_mut().highlighter = strop_syntax::Highlighter::for_path("audit.rs");
    e
}

fn spans(e: &mut Editor) -> Vec<strop_syntax::Span> {
    let rope = e.buf().rope.clone();
    let rev = e.buf().epoch;
    e.cur_mut()
        .highlighter
        .as_mut()
        .unwrap()
        .highlight(&rope, rev, 0, rope.len_bytes())
}

fn fresh_spans(e: &Editor) -> Vec<strop_syntax::Span> {
    let mut h = strop_syntax::Highlighter::for_path("audit.rs").unwrap();
    h.highlight(&e.buf().rope, e.buf().epoch, 0, e.buf().len_bytes())
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
fn review_input_edit_preserves_start_column() {
    let b = Buffer::from_text("abcdXef\n");
    let op = strop_core::history::Edit {
        at: 4,
        text: "X".into(),
        kind: strop_core::history::EditKind::Insert,
    };
    let edit = b.input_edit_of(&op);
    assert_eq!(
        edit.new_end_point,
        (0, 5),
        "single-line insertion ends at start.column + inserted bytes"
    );
}

#[test]
fn review_split_from_empty_scratch_retains_live_panes() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("file.txt");
    std::fs::write(&path, "file\n").unwrap();
    let mut e = Editor::new(Buffer::from_text(""));
    e.feed_text(&format!(":vs {}<cr><c-w>h", path.display()));
    assert!(e.panes.iter().all(|p| e.docs.get(p.doc).is_some()));
}

#[test]
fn review_git_reply_belongs_to_request_document() {
    let mut e = Editor::new(Buffer::from_text("first\n"));
    let old_epoch = e.buf().epoch;
    let second = e
        .docs
        .insert(Document::scratch(Buffer::from_text("second\n")));
    e.switch_to(second);
    e.hunks_in_flight = true;
    let hunk = strop_git::Hunk::build(
        1,
        1,
        1,
        1,
        vec![strop_git::DiffLine {
            has_newline: true,
            origin: strop_git::LineOrigin::Addition,
            old_lineno: None,
            new_lineno: Some(1),
            text: b"first-file change".to_vec(),
        }],
    );
    let first = e.docs.iter().next().map(|(id, _)| id).unwrap();
    e.handle_git_job(GitJob::Hunks {
        doc: first,
        epoch: old_epoch,
        unstaged: vec![hunk],
        staged: vec![],
    });
    assert!(
        e.hunks.is_empty(),
        "a result from the first file must not populate the second file"
    );
}

#[test]
fn review_clipboard_reply_preserves_destination() {
    let mut e = Editor::new(Buffer::from_text("first\n"));
    let first = e.current();
    e.clip_paste_pending = Some((false, first));
    let second = e
        .docs
        .insert(Document::scratch(Buffer::from_text("second\n")));
    e.switch_to(second);
    e.handle_clipboard(Some("CLIP".into()));
    assert_eq!(
        e.buf().rope.to_string(),
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
    b.insert(0, "new ");
    b.save(false).unwrap();
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
    let before = e.buf().rope.to_string();
    e.feed(Key::Char('c'));
    e.feed(Key::Char('i'));
    assert!(
        e.preview().is_none(),
        "no fake preview for ci — the object isn't chosen yet"
    );
    e.feed(Key::Char('['));
    assert_ne!(
        e.buf().rope.to_string(),
        before,
        "ci[ executes at the completing key"
    );
    // the composition window previews (d/foo shows its target as you type)
    let mut e = Editor::new(Buffer::from_text("hello world foo\n"));
    e.feed_text("d/wo");
    assert!(
        e.preview().is_some(),
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
    assert_eq!(e.buf().rope.to_string(), "one\none\ntwo\n");
}

#[test]
fn review_render_handles_more_panes_than_cells() {
    let mut e = Editor::new(Buffer::from_text("x\n"));
    for _ in 0..5 {
        e.feed_text(":vs<cr>");
    }
    let _ = crate::headless::frame_string(&mut e, 4, 4);
}

#[test]
fn review_tab_glyph_and_caret_use_same_layout() {
    let mut e = Editor::new(Buffer::from_text("\tX\n"));
    e.feed_text("l");
    let backend = ratatui::backend::TestBackend::new(40, 8);
    let mut terminal = ratatui::Terminal::new(backend).unwrap();
    terminal.draw(|f| crate::render::render(&mut e, f)).unwrap();
    let x_col = (0..40u16)
        .find(|x| terminal.backend().buffer()[(*x, 0)].symbol() == "X")
        .unwrap() as usize;
    // the contract: the glyph and the caret read the same layout,
    // driven by the editor's tab width
    let caret = 5 + e
        .buf()
        .cell_col_with_tab(e.head(), e.config.tab_size as u16) as usize;
    assert_eq!(x_col, caret, "rendered X and caret must agree after a tab");
}

#[test]
fn review_stale_replace_range_is_utf8_safe() {
    let mut e = Editor::new(Buffer::from_text("界foo\n"));
    let doc = e.current();
    let (_, applied, stale) = e.replace_in_buffer_pub(doc, &[(1, 2, 3, "afoo".into())], "bar");
    assert_eq!((applied, stale), (0, 1));
}

#[test]
fn review_operator_find_accepts_digit_as_target() {
    let mut e = Editor::new(Buffer::from_text("ab2cd\n"));
    e.feed_text("df2");
    assert_eq!(
        e.buf().rope.to_string(),
        "cd\n",
        "a digit after f is a target character"
    );
}

#[test]
fn review_search_preview_starts_at_utf8_boundary() {
    let mut e = Editor::new(Buffer::from_text("界foo\n"));
    e.feed_text("d/f");
    let _ = e.preview();
}
