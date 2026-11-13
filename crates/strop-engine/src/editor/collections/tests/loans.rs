//! Collections close what they opened (0101 §5.3): the files a collect
//! loaded in the background go when the collection does, unless the user
//! looked at them, edited them, or they were open already.
use super::*;
use crate::editor::events::AppEvent;
use crate::editor::trace::drive::Action;
use std::path::{Path, PathBuf};
use strop_core::frontend_input::{notation, Input, Key};

/// Turns of the event loop, one per key, each with its after-event work
/// (where a loaned file on screen is taken), as the terminal runs them.
fn keys(e: &mut Editor, text: &str) {
    for key in notation::parse(text) {
        e.apply_recorded(Action::Event(AppEvent::Input(Input::Key(key))))
            .unwrap();
    }
}

fn turn(e: &mut Editor, key: Key) {
    e.apply_recorded(Action::Event(AppEvent::EditorKey(key)))
        .unwrap();
}

/// Three files containing `needle`, none of them open.
fn project(root: &Path) -> Vec<PathBuf> {
    ["one.txt", "two.txt", "three.txt"]
        .iter()
        .map(|name| {
            let path = root.join(name);
            std::fs::write(&path, format!("top\nneedle {name}\nbottom\n")).unwrap();
            path
        })
        .collect()
}

/// `Space /` for `query`, the search finished, then `Ctrl-O` and the
/// background loads landed.
fn collect(e: &mut Editor, query: &str) {
    e.open_search(false);
    e.feed(Key::CtrlU);
    e.paste_bracketed(query);
    e.wait_picker();
    turn(e, Key::CtrlO);
    e.wait_io().unwrap();
    // the build landed: one turn of after-event work, as the loop runs it
    crate::editor::collections::loans::take_shown(e);
}

/// The file names `Space b` would list.
fn open_files(e: &Editor) -> Vec<String> {
    let mut names: Vec<String> = e
        .docs
        .iter()
        .filter_map(|(_, document)| document.buf.path.as_ref())
        .filter_map(|path| path.file_name())
        .map(|name| name.to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

fn editor(root: &Path) -> Editor {
    Editor::new_in(Buffer::from_text("origin\nline two\n"), root.to_path_buf())
}

#[test]
fn files_never_visited_close_with_the_collection() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut e = editor(dir.path());
    let origin = e.current();
    keys(&mut e, "j");
    let pressed_at = e.head();
    collect(&mut e, "needle");
    assert!(e.collections.contains_key(&e.current()));
    assert_eq!(open_files(&e), ["one.txt", "three.txt", "two.txt"]);
    keys(&mut e, ":q<cr>");
    assert!(open_files(&e).is_empty(), "{:?}", open_files(&e));
    assert!(e.collections.is_empty());
    assert_eq!(e.current(), origin);
    assert_eq!(e.head(), pressed_at, "back where Ctrl-O was pressed");
    assert!(!e.should_quit);
}

#[test]
fn a_visited_file_stays_and_the_others_close() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    keys(&mut e, "<cr>");
    let visited = e.buf().path.clone().unwrap();
    keys(&mut e, "<c-o>");
    assert!(e.collections.contains_key(&e.current()));
    keys(&mut e, ":q<cr>");
    let name = visited.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(open_files(&e), [name]);
}

#[test]
fn an_edited_and_saved_file_stays_for_its_undo_history() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    let at = current_text(&e).find("needle one.txt").unwrap();
    e.set_head(at);
    keys(&mut e, "cwpin<esc>");
    keys(&mut e, ":w<cr>");
    e.wait_io().unwrap();
    assert!(std::fs::read_to_string(&paths[0])
        .unwrap()
        .contains("pin one.txt"));
    keys(&mut e, ":q<cr>");
    assert_eq!(open_files(&e), ["one.txt"]);
}

#[test]
fn a_dirty_file_is_never_closed_and_the_close_says_so() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    let at = current_text(&e).find("needle two.txt").unwrap();
    e.set_head(at);
    keys(&mut e, "cwpin<esc>");
    keys(&mut e, ":q<cr>");
    assert_eq!(open_files(&e), ["two.txt"]);
    let (_, kept) = e
        .docs
        .iter()
        .find(|(_, d)| d.buf.path.as_deref() == Some(paths[1].as_path()))
        .unwrap();
    assert!(kept.buf.is_dirty(), "still dirty, still open");
    assert_eq!(
        e.message,
        "1 file the collection opened has unsaved changes and stays open"
    );
}

#[test]
fn a_file_open_before_the_collect_is_never_closed() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    let origin = e.current();
    e.open_fixture(&paths[2]).unwrap();
    e.switch_to(origin);
    collect(&mut e, "needle");
    keys(&mut e, ":q<cr>");
    assert_eq!(open_files(&e), ["three.txt"]);
}

#[test]
fn a_shared_source_closes_with_the_last_collection_holding_it() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    let first = e.current();
    collect(&mut e, "needle one");
    let second = e.current();
    assert_ne!(first, second);
    keys(&mut e, ":q<cr>");
    assert!(!e.collections.contains_key(&second));
    assert!(e.collections.contains_key(&first));
    assert_eq!(open_files(&e), ["one.txt", "three.txt", "two.txt"]);
    e.switch_to(first);
    keys(&mut e, ":q<cr>");
    assert!(open_files(&e).is_empty(), "{:?}", open_files(&e));
}

#[test]
fn a_refresh_closes_the_sources_the_new_list_does_not_use() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    let collection = e.current();
    // the same search again, after two files stopped matching on disk
    std::fs::write(&paths[1], "nothing here\n").unwrap();
    std::fs::write(&paths[2], "nothing here\n").unwrap();
    collect(&mut e, "needle");
    assert_eq!(e.current(), collection, "a refresh keeps its buffer");
    assert_eq!(open_files(&e), ["one.txt"]);
}

#[test]
fn an_abandoned_build_closes_what_it_had_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    let other = dir.path().join("other.txt");
    std::fs::write(&other, "elsewhere\n").unwrap();
    e.open_search(false);
    e.feed(Key::CtrlU);
    e.paste_bracketed("needle");
    e.wait_picker();
    turn(&mut e, Key::CtrlO);
    // one of the three loads lands, then a second collect replaces the build
    let event =
        e.io.rx
            .as_ref()
            .unwrap()
            .recv_timeout(std::time::Duration::from_secs(10))
            .unwrap();
    e.handle_io(event);
    assert_eq!(open_files(&e).len(), 1, "one source loaded");
    collect(&mut e, "elsewhere");
    assert_eq!(open_files(&e), ["other.txt"], "{paths:?}");
}

#[test]
fn every_hit_changed_closes_what_the_build_loaded() {
    let dir = tempfile::tempdir().unwrap();
    let paths = project(dir.path());
    let mut e = editor(dir.path());
    e.open_search(false);
    e.feed(Key::CtrlU);
    e.paste_bracketed("needle");
    e.wait_picker();
    for path in &paths {
        std::fs::write(path, "changed\n").unwrap();
    }
    turn(&mut e, Key::CtrlO);
    e.wait_io().unwrap();
    assert!(e.message.starts_with("nothing collected"), "{}", e.message);
    assert!(open_files(&e).is_empty(), "{:?}", open_files(&e));
}

#[test]
fn closing_the_only_buffer_lands_on_a_file_that_stays() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    // a pristine scratch is replaced by the collection
    let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_path_buf());
    collect(&mut e, "needle");
    assert_eq!(e.docs.len(), 4, "the collection and its three files");
    keys(&mut e, ":q<cr>");
    assert!(!e.should_quit, "the editor does not quit");
    let landed = e.buf().path.clone().expect("landed on a file");
    let name = landed.file_name().unwrap().to_string_lossy().into_owned();
    assert_eq!(open_files(&e), [name]);
}

#[test]
fn cdo_keeps_the_files_it_edits_and_not_the_ones_it_does_not() {
    let dir = tempfile::tempdir().unwrap();
    project(dir.path());
    let mut e = editor(dir.path());
    collect(&mut e, "needle");
    keys(&mut e, ":cdo s/absent/x/<cr>");
    keys(&mut e, ":q<cr>");
    assert!(open_files(&e).is_empty(), "{:?}", open_files(&e));

    collect(&mut e, "needle");
    keys(&mut e, ":cdo s/needle/pin/<cr>");
    keys(&mut e, ":cfdo w<cr>");
    e.wait_io().unwrap();
    keys(&mut e, ":q<cr>");
    assert_eq!(open_files(&e), ["one.txt", "three.txt", "two.txt"]);
}
