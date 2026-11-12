//! What stays put (0098 §5): typing a command, moving the cursor and
//! narrowing the terminal each move as little of the row as they can.

use strop_git::GitContext;

use super::{editor, grid, row};
use crate::editor::Editor;
use crate::render::clip::width;
use crate::render::tokens;

fn cells(row: &str) -> Vec<char> {
    row.chars().collect()
}

/// The column of the first cell painted in the accent as text: the
/// pending keys.
fn pending_column(editor: &Editor, columns: u16) -> u16 {
    let grid = grid(editor, columns, 4);
    (0..columns)
        .find(|&x| grid[(x, 3)].fg == tokens().accent && grid[(x, 3)].symbol() != " ")
        .expect("pending keys on the row")
}

fn on_a_branch(text: &str, path: &str, branch: &str) -> Editor {
    let mut editor = editor(text, Some(path));
    editor.fixture_set_git(Some(GitContext {
        repo: strop_git::RepoTarget::Local {
            workdir: "/w".into(),
        },
        head_sha: None,
        head_branch: Some(branch.into()),
        remotes: Vec::new(),
    }));
    editor
}

#[test]
fn typed_keys_grow_to_the_right_and_move_nothing_else() {
    let mut e = on_a_branch("alpha beta\ngamma\n", "/w/src/f.rs", "main");
    let idle = cells(&row(&e, 80, 4));
    e.feed_text("d");
    let first = pending_column(&e, 80);
    let one = cells(&row(&e, 80, 4));
    e.feed_text("2");
    let two = cells(&row(&e, 80, 4));
    assert_eq!(pending_column(&e, 80), first, "the first key stays put");
    e.feed_text("i");
    let three = cells(&row(&e, 80, 4));
    assert_eq!(pending_column(&e, 80), first, "the first key stays put");
    let slot = usize::from(first)..usize::from(first) + 8;
    assert_eq!(
        three[slot.clone()].iter().collect::<String>(),
        "d2i     ",
        "left-aligned in the slot"
    );
    // outside the slot (and the chip, which names the mode) every cell
    // is where it was before the first key
    for typed in [&one, &two, &three] {
        for x in (10..80).filter(|x| !slot.contains(x)) {
            assert_eq!(typed[x], idle[x], "cell {x} moved");
        }
    }
}

#[test]
fn the_leader_prefix_reads_as_words() {
    let mut e = editor("alpha\n", Some("/w/f.rs"));
    e.feed_text(" g");
    let shown = row(&e, 80, 4);
    assert!(shown.contains("  space g  "), "{shown:?}");
}

#[test]
fn moving_across_a_digit_boundary_moves_no_other_cell() {
    let line = "x".repeat(40);
    let text: String = (0..120).map(|_| format!("{line}\n")).collect();
    let mut e = on_a_branch(&text, "/w/src/f.rs", "main");
    let field = |row: &[char]| {
        let percent = row.iter().position(|ch| *ch == '%').unwrap();
        let colon = row.iter().rposition(|ch| *ch == ':').unwrap();
        (
            percent,
            colon,
            row[..percent - 3].iter().collect::<String>(),
        )
    };
    e.feed_text("8j8l");
    let before = cells(&row(&e, 80, 4));
    assert!(before.iter().collect::<String>().contains("9:9"));
    e.feed_text("j");
    let line_ten = cells(&row(&e, 80, 4));
    e.feed_text("l");
    let column_ten = cells(&row(&e, 80, 4));
    assert!(column_ten.iter().collect::<String>().contains("10:10"));
    e.feed_text("G");
    let last = cells(&row(&e, 80, 4));
    assert!(last.iter().collect::<String>().contains("100%"));
    for moved in [&line_ten, &column_ten, &last] {
        assert_eq!(field(moved), field(&before), "{moved:?}");
    }
    // the row never touches the screen's right edge
    assert_eq!(last[79], ' ');
}

#[test]
fn a_long_branch_shortens_before_it_leaves_and_the_filename_survives() {
    let mut e = on_a_branch(
        "alpha\nbeta\n",
        "/w/crates/strop/src/render/statusline/some_long_module_name.rs",
        "feature/modeline-design",
    );
    let wide = row(&e, 120, 4);
    assert!(wide.contains("feature/modeline-design"), "{wide:?}");
    assert!(wide.contains("crates/strop/src/render/statusline/some_long"));
    let idle = row(&e, 80, 4);
    e.feed_text(" ");
    let pending = row(&e, 80, 4);
    for shown in [&idle, &pending] {
        assert!(shown.contains("feature/"), "the branch stays: {shown:?}");
        assert!(shown.contains("some_long_module_name.rs"), "{shown:?}");
    }
    assert!(pending.contains("space"), "{pending:?}");
    let name = |row: &str| width(&row[..row.find("some_long").unwrap()]);
    assert!(
        name(&idle).abs_diff(name(&pending)) <= "space".len() + 2,
        "a typed key costs its own width, not a whole group: {idle:?} / {pending:?}"
    );
    for columns in 1..=200u16 {
        let shown = row(&e, columns, 4);
        assert_eq!(width(&shown), usize::from(columns), "{columns}: {shown:?}");
        if columns >= 52 {
            assert!(
                shown.contains("some_long_module_name.rs"),
                "{columns}: {shown:?}"
            );
            assert!(shown.contains("space"), "{columns}: {shown:?}");
        }
        if columns >= 24 {
            assert!(shown.contains("PENDING"), "{columns}: {shown:?}");
            assert!(shown.contains("1:1"), "{columns}: {shown:?}");
        }
    }
}

#[test]
fn groups_are_two_cells_apart_and_the_halves_never_closer() {
    let mut e = on_a_branch(
        "alpha\n",
        "/w/crates/strop/src/render/statusline/some_long_module_name.rs",
        "main",
    );
    e.fixture_set_hunks_untracked(true);
    e.feed_text("d");
    for columns in [60u16, 70, 80, 96] {
        let shown = row(&e, columns, 4);
        assert!(shown.contains("  main*  "), "{columns}: {shown:?}");
        assert!(shown.contains(".rs  d"), "{columns}: {shown:?}");
        assert!(!shown.contains(".rs d"), "{columns}: {shown:?}");
    }
}

#[test]
fn the_substitution_summary_stays_in_its_card() {
    let mut e = on_a_branch("alpha one\nalpha two\n", "/w/src/f.rs", "main");
    e.feed_text(":%s/alpha/omega/g");
    let shown = row(&e, 120, 4);
    assert!(!shown.contains("substitution"), "{shown:?}");
    assert!(
        shown.contains("main"),
        "the branch keeps its place: {shown:?}"
    );
    assert!(shown.contains("src/f.rs"), "{shown:?}");
}

#[test]
fn a_recording_macro_is_named_until_it_stops() {
    let mut e = editor("alpha\nbeta\n", Some("/w/f.rs"));
    assert!(!row(&e, 80, 4).contains("REC"));
    e.feed_text("qa");
    let started = grid(&e, 80, 4);
    let shown = row(&e, 80, 4);
    assert!(shown.contains("  NORMAL    REC @a  f.rs"), "{shown:?}");
    let at = cells(&shown)
        .iter()
        .collect::<String>()
        .find("REC")
        .unwrap() as u16;
    assert_eq!(started[(at, 3)].fg, tokens().error);
    e.feed_text("jd");
    assert!(
        row(&e, 80, 4).contains("REC @a"),
        "still recording mid-command"
    );
    e.feed(strop_core::frontend_input::Key::Esc);
    e.feed_text("q");
    assert!(!row(&e, 80, 4).contains("REC"));
}
