//! Tab pages (plans/0113 §8.1, §8.2): the model's arithmetic against the
//! formulas of §3, written out here, and parked panes following their
//! documents. The commands are `ex/tests/tabs.rs`; Neovim agreement is the
//! `tab/` corpus of the differential harness.

use super::*;
use crate::editor::panes::Closing;
use crate::editor::Document;
use strop_core::Buffer;

/// `n` tab pages, the k-th (1-based) showing a buffer whose text is `tk`;
/// tab page `at` (0-based) is current.
fn tabs(n: usize, at: usize) -> Editor {
    let mut e = Editor::new(Buffer::from_text("t1"));
    for k in 2..=n {
        let id = e
            .docs
            .try_insert(Document::scratch(Buffer::from_text(&format!("t{k}"))))
            .unwrap();
        open(&mut e, Pane::fresh(id, Default::default()));
    }
    go(&mut e, at);
    assert_eq!(current(&e), at);
    e
}

/// The text each tab page's current pane shows, in tab order.
fn order(e: &Editor) -> Vec<String> {
    (0..count(&e))
        .map(|index| {
            let summary = summary(&e, index).unwrap();
            e.doc(summary.document).buf.text().to_string()
        })
        .collect()
}

fn names(n: usize) -> Vec<String> {
    (1..=n).map(|k| format!("t{k}")).collect()
}

fn every_pane(e: &Editor) -> usize {
    e.panes.len() + e.tabs.panes().count()
}

#[test]
fn a_new_tab_page_opens_after_the_current_one_and_takes_the_focus() {
    let mut e = tabs(3, 0);
    let id = e
        .docs
        .try_insert(Document::scratch(Buffer::from_text("new")))
        .unwrap();
    open(&mut e, Pane::fresh(id, Default::default()));
    assert_eq!(order(&e), ["t1", "new", "t2", "t3"]);
    assert_eq!(current(&e), 1);
    assert_eq!(summary(&e, 4), None);
}

#[test]
fn gt_and_gt_with_a_count_land_where_neovim_does() {
    for n in 1..=5 {
        for c in 0..n {
            let mut e = tabs(n, c);
            step(&mut e, false, None);
            assert_eq!(current(&e), (c + 1) % n, "gt from {c} of {n}");
            step(&mut e, true, None);
            assert_eq!(current(&e), c, "gT undoes gt");
            step(&mut e, true, None);
            step(&mut e, false, None);
            assert_eq!(current(&e), c, "gt undoes gT");
            assert_eq!(order(&e), names(n), "steps never reorder");
            for count in 0..=7 {
                let mut e = tabs(n, c);
                step(&mut e, false, Some(count));
                let expected = match count {
                    0 => (c + 1) % n,
                    count if count <= n => count - 1,
                    _ => c,
                };
                assert_eq!(current(&e), expected, "{count}gt from {c} of {n}");
                if count > n {
                    assert_eq!(e.message, format!("no tab {count}"));
                }
            }
            for count in 1..=7 {
                let mut e = tabs(n, c);
                step(&mut e, true, Some(count));
                let expected = (c + n - count % n) % n;
                assert_eq!(current(&e), expected, "{count}gT from {c} of {n}");
            }
        }
    }
}

#[test]
fn moving_keeps_the_others_in_order() {
    for n in 1..=5 {
        for c in 0..n {
            for index in 0..n {
                let mut e = tabs(n, c);
                move_to(&mut e, index);
                let mut expected = names(n);
                let moved = expected.remove(c);
                expected.insert(index, moved);
                assert_eq!(order(&e), expected, "move {c} to {index} of {n}");
                assert_eq!(current(&e), index);
            }
        }
    }
}

#[test]
fn closing_any_tab_page_lands_where_neovim_does() {
    for n in 1..=5 {
        for c in 0..n {
            for index in 0..n {
                let mut e = tabs(n, c);
                let closed = close(&mut e, index);
                if n == 1 {
                    assert!(!closed, "the last tab page stays");
                    assert_eq!(order(&e), names(1));
                    continue;
                }
                assert!(closed);
                let mut expected = names(n);
                expected.remove(index);
                assert_eq!(order(&e), expected, "close {index} from {c} of {n}");
                let expected_current = match index {
                    index if index == c => c.min(n - 2),
                    index if index < c => c - 1,
                    _ => c,
                };
                assert_eq!(
                    current(&e),
                    expected_current,
                    "close {index} from {c} of {n}"
                );
                assert_eq!(e.docs.len(), n, "its buffers stay open");
            }
        }
    }
}

#[test]
fn tab_only_keeps_this_one_and_its_panes() {
    let mut e = tabs(4, 2);
    e.window_command('v');
    only(&mut e);
    assert_eq!(order(&e), ["t3"]);
    assert_eq!(e.panes.len(), 2);
    assert_eq!(e.active_pane, 1);
    assert_eq!(e.docs.len(), 4);
}

#[test]
fn g_tab_returns_to_the_tab_page_used_before() {
    let mut e = tabs(4, 0);
    previous(&mut e);
    assert_eq!(current(&e), 3, "tabs() came to 1 from 4");
    let mut e = tabs(4, 0);
    step(&mut e, false, Some(3));
    previous(&mut e);
    assert_eq!(current(&e), 0, "after 3gt");
    previous(&mut e);
    assert_eq!(current(&e), 2, "twice returns");
    // the previous one closed: nothing
    close(&mut e, 0);
    assert_eq!(current(&e), 1);
    previous(&mut e);
    assert_eq!(current(&e), 1);
    // a move keeps it findable
    let mut e = tabs(4, 0);
    step(&mut e, false, Some(4));
    move_to(&mut e, 0);
    assert_eq!(order(&e), ["t4", "t1", "t2", "t3"]);
    previous(&mut e);
    assert_eq!(current(&e), 1);
    assert_eq!(order(&e)[current(&e)], "t1");
    // after :tabonly there is nothing to return to
    only(&mut e);
    previous(&mut e);
    assert_eq!(count(&e), 1);
}

#[test]
fn ctrl_w_t_moves_the_pane_out_of_a_nest() {
    let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
    e.window_command('v');
    e.window_command('s');
    e.window_command('k');
    assert_eq!(
        e.layout().outline().to_string(),
        r#"["row",[1,["col",[2,3]]]]"#
    );
    assert_eq!(e.active_pane, 1);
    let mut expected = e.layout().clone();
    let focus = expected.remove(1);
    e.window_command('T');
    assert_eq!(count(&e), 2);
    assert_eq!(current(&e), 1);
    assert_eq!(e.panes.len(), 1);
    assert_eq!(*e.layout(), Layout::Leaf);
    step(&mut e, false, Some(1));
    assert_eq!(*e.layout(), expected, "the source tree is remove's result");
    assert_eq!(e.active_pane, focus, "and its focus remove's answer");
    // one pane: nothing
    e.window_command('o');
    e.window_command('T');
    assert_eq!(count(&e), 2);
    assert_eq!(e.message, "already only one window");
}

#[test]
fn closing_a_pane_owns_its_answer() {
    let mut e = Editor::new(Buffer::from_text("alpha\n"));
    assert_eq!(e.closing(), Closing::Buffer);
    e.window_command('v');
    assert_eq!(e.closing(), Closing::Pane);
    e.window_command('T');
    assert_eq!(e.closing(), Closing::TabPage);
    // `ctrl-w c` on a tab page's only pane closes the tab page
    e.window_command('c');
    assert_eq!(count(&e), 1);
    assert_eq!(e.panes.len(), 1);
    assert_eq!(e.closing(), Closing::Buffer);
    e.window_command('c');
    assert_eq!(e.message, "cannot close the last window");
    assert_eq!(e.docs.len(), 1);
}

/// A seeded walk over §3's commands and `ctrl-w s v c o`: the invariants
/// hold after every step and the panes change only by the command's own
/// amount.
#[test]
fn a_random_walk_keeps_every_tab_page_well_formed() {
    let mut seed: u64 = 0x5eed_0113;
    let mut next = |bound: usize| {
        seed = seed
            .wrapping_mul(6_364_136_223_846_793_005)
            .wrapping_add(1_442_695_040_888_963_407);
        (seed >> 33) as usize % bound
    };
    for _ in 0..2_000 {
        let mut e = Editor::new(Buffer::from_text("alpha\nbeta\n"));
        for _ in 0..12 {
            let panes = every_pane(&e);
            let here = e.panes.len();
            let parked = e.tabs.panes().count();
            let tabs = count(&e);
            let expected = match next(11) {
                0 => {
                    step(&mut e, false, None);
                    panes
                }
                1 => {
                    let count = next(7);
                    step(&mut e, true, (count > 0).then_some(count));
                    panes
                }
                2 => {
                    previous(&mut e);
                    panes
                }
                3 => {
                    let pane = e.view().clone();
                    open(&mut e, pane);
                    panes + 1
                }
                4 => {
                    e.window_command('T');
                    panes
                }
                5 => {
                    let closed = close_current(&mut e);
                    assert_eq!(closed, tabs > 1);
                    if closed {
                        panes - here
                    } else {
                        panes
                    }
                }
                6 => {
                    only(&mut e);
                    panes - parked
                }
                7 => {
                    let index = next(tabs);
                    move_to(&mut e, index);
                    panes
                }
                8 => {
                    e.window_command(if next(2) == 0 { 's' } else { 'v' });
                    panes + 1
                }
                9 => {
                    e.window_command('c');
                    if here > 1 || tabs > 1 {
                        panes - 1
                    } else {
                        panes
                    }
                }
                _ => {
                    e.window_command('o');
                    panes - (here - 1)
                }
            };
            assert!(well_formed(&e));
            assert!(e.layout_matches_panes());
            assert_eq!(every_pane(&e), expected);
        }
    }
}

// --- §8.2: parked panes follow their documents ---------------------------

/// An edit in tab page 2 above a cursor parked in tab page 1 on the same
/// buffer: back in tab page 1 the cursor is on the same text.
#[test]
fn an_edit_elsewhere_moves_a_parked_cursor_with_its_text() {
    let mut e = Editor::new(Buffer::from_text("a\nb\nc\n"));
    e.feed_text("G");
    let head = e.head();
    e.push_jump();
    let at = e
        .view()
        .jumps
        .entries
        .iter()
        .position(|record| record.offset == head)
        .unwrap();
    let pane = e.view().clone();
    open(&mut e, pane);
    e.feed_text("ggOnew<esc>");
    go(&mut e, 0);
    assert_eq!(e.buf().line_text(e.buf().line_of(e.head())), "c");
    // and its jump list moved with it
    assert_eq!(e.view().jumps.entries[at].offset, head + 4);
}

/// `:bd` of a buffer shown only in a parked tab page: that pane shows the
/// next buffer and coming back finds a live one.
#[test]
fn deleting_a_buffer_only_a_parked_pane_shows() {
    let mut e = tabs(2, 0);
    let parked = summary(&e, 1).unwrap().document;
    e.run_ex(&format!("bd {}", crate::editor::ex::buffer_number(parked)));
    assert!(e.docs.get(parked).is_none(), "{}", e.message);
    assert!(well_formed(&e));
    step(&mut e, false, None);
    assert_eq!(current(&e), 1);
    assert!(e.docs.get(e.current()).is_some());
}

/// The first real file replaces the empty scratch buffer in a parked pane
/// too.
#[test]
fn the_first_file_replaces_the_scratch_in_a_parked_pane() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("first.txt");
    std::fs::write(&path, "first\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text(""), dir.path().to_owned());
    let scratch = e.current();
    let pane = e.view().clone();
    open(&mut e, pane);
    e.feed_text(&format!(":e {}<cr>", path.display()));
    e.wait_io().unwrap();
    assert!(e.docs.get(scratch).is_none(), "the scratch buffer went");
    assert!(e.tabs.panes().all(|pane| pane.doc == e.current()));
    assert!(well_formed(&e));
}

/// A terminal whose job ends while parked: on return it is not in
/// TERMINAL input.
#[test]
fn a_terminal_that_ends_while_parked_comes_back_in_normal() {
    let dir = tempfile::tempdir().unwrap();
    let mut e = Editor::new_in(Buffer::from_text("origin"), dir.path().to_owned());
    let terminal = e.terminal_fixture(
        &[("$", strop_terminal::model::Style::default())],
        strop_terminal::model::Phase::Running,
    );
    assert!(e.enter_terminal_input());
    let other = e
        .docs
        .try_insert(Document::scratch(Buffer::from_text("other")))
        .unwrap();
    open(&mut e, Pane::fresh(other, Default::default()));
    e.terminal_fixture_exit(terminal, 0);
    assert!(e.tabs.panes().all(|pane| !pane.terminal_input));
    go(&mut e, 0);
    assert_eq!(e.current(), terminal);
    assert!(!e.terminal_input_active());
}

/// A loaned file shown only in a parked tab page is shown: closing the
/// collection that loaned it keeps it open.
#[test]
fn a_loan_shown_in_a_parked_pane_is_not_released() {
    let dir = tempfile::tempdir().unwrap();
    for name in ["one.txt", "two.txt"] {
        std::fs::write(dir.path().join(name), "needle\n").unwrap();
    }
    let mut e = Editor::new_in(Buffer::from_text("origin\n"), dir.path().to_owned());
    e.open_search(false);
    e.feed(strop_core::frontend_input::Key::CtrlU);
    e.paste_bracketed("needle");
    e.wait_picker();
    e.feed(strop_core::frontend_input::Key::CtrlO);
    e.wait_io().unwrap();
    let collection = e.current();
    assert!(e.collections.contains_key(&collection));
    let loaned = e
        .docs
        .iter()
        .find_map(|(id, document)| document.loan.map(|_| id))
        .unwrap();
    open(&mut e, Pane::fresh(loaned, Default::default()));
    go(&mut e, 0);
    e.run_ex("bd");
    assert!(e.docs.get(collection).is_none(), "{}", e.message);
    assert!(e.docs.get(loaned).is_some(), "a parked pane shows it");
    assert!(well_formed(&e));
}

/// A collection refreshed while a parked pane shows it: that pane's old
/// caret addressed text that is gone, so it starts over.
#[test]
fn a_collection_refreshed_under_a_parked_pane_resets_it() {
    use strop_core::frontend_input::Key;
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(dir.path().join("a.txt"), "hit one\nx\nhit two\n").unwrap();
    let mut e = Editor::new_in(Buffer::from_text("origin\n"), dir.path().to_owned());
    let search = |e: &mut Editor| {
        e.open_search(false);
        e.feed(Key::CtrlU);
        e.paste_bracketed("hit");
        e.wait_picker();
        e.feed(Key::CtrlO);
        e.wait_io().unwrap();
    };
    search(&mut e);
    let collection = e.current();
    e.feed_text("G");
    assert_ne!(e.head(), 0);
    let origin = e
        .docs
        .iter()
        .find_map(|(id, document)| (id != collection && document.buf.path.is_none()).then_some(id))
        .unwrap();
    open(&mut e, Pane::fresh(origin, Default::default()));
    search(&mut e);
    assert_eq!(e.current(), collection, "the same collection, refreshed");
    let parked = e.tabs.panes().next().unwrap();
    assert_eq!(parked.doc, collection);
    assert_eq!(parked.sels.primary().head, 0);
}

/// The seed carries tab pages; one whose parked pane names a dead
/// document is refused.
#[test]
fn tab_pages_survive_the_seed_and_a_dead_parked_document_is_refused() {
    use crate::editor::trace::seed::Seed;
    let mut e = Editor::new_in(
        Buffer::from_text("one\ntwo\n"),
        std::path::PathBuf::from("/virtual"),
    );
    e.window_command('v');
    let pane = e.view().clone();
    open(&mut e, pane);
    let pane = e.view().clone();
    open(&mut e, pane);
    go(&mut e, 1);
    // the seed boundary precedes services; switching tab pages started
    // git discovery, which is not what is under test here
    e.git_discovery = strop_core::worker::Load::Idle;
    let tape = || std::rc::Rc::new(strop_trace::replay::Tape::replay(Vec::new()));
    let replayed = Seed::capture(&e).unwrap().into_editor(tape()).unwrap();
    assert_eq!(
        serde_json::to_value(&replayed.tabs).unwrap(),
        serde_json::to_value(&e.tabs).unwrap()
    );
    assert_eq!(current(&replayed), 1);
    assert_eq!(
        crate::editor::state_json(&replayed),
        crate::editor::state_json(&e)
    );

    let mut serialized = serde_json::to_value(Seed::capture(&e).unwrap()).unwrap();
    let doc = &mut serialized["tabs"]["parked"][0]["panes"][0]["doc"];
    *doc = serde_json::json!({"slot": 7, "generation": 3});
    let seed: Seed = serde_json::from_value(serialized).unwrap();
    assert!(seed.into_editor(tape()).is_err());
}
