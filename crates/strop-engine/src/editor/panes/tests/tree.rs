//! The window tree (plans/0104 §8.1, §8.2): vim's split/close/focus rules on
//! the tree, the geometry, and the tiling property over random trees.

use super::super::layout::{minimum, Axis, Layout};
use super::outline;
use crate::editor::prepare::{CellRect, ViewGeometry};
use crate::editor::Editor;
use strop_core::frontend_input::Key;
use strop_core::Buffer;

fn editor() -> Editor {
    Editor::new(Buffer::from_text("alpha\nbeta\ngamma\n"))
}

fn window(e: &mut Editor, keys: &str) {
    for key in keys.chars() {
        e.feed(Key::CtrlW);
        e.feed(Key::Char(key));
    }
}

/// The owner's report: `:sp` then `:vs` splits the lower pane only.
#[test]
fn a_vertical_split_after_a_horizontal_one_divides_the_current_pane() {
    let mut e = editor();
    e.feed_text(":sp<cr>:vs<cr>");
    assert_eq!(outline(&e), r#"["col",[1,["row",[2,3]]]]"#);
    assert_eq!(e.active_pane(), 2);
}

#[test]
fn splits_nest_and_insert_beside_the_current_pane() {
    let mut e = editor();
    e.feed_text(":vs<cr>:sp<cr>");
    assert_eq!(outline(&e), r#"["row",[1,["col",[2,3]]]]"#, "an L");
    let mut e = editor();
    e.feed_text(":vs<cr>:vs<cr>");
    assert_eq!(outline(&e), r#"["row",[1,2,3]]"#, "same axis: a sibling");
    // a split from the first of three lands next to it, numbered 2
    e.focus_pane(0);
    window(&mut e, "s");
    assert_eq!(outline(&e), r#"["row",[["col",[1,2]],3,4]]"#);
    assert_eq!(e.active_pane(), 1);
}

#[test]
fn closing_gives_the_cells_and_focus_to_the_previous_neighbour() {
    let mut e = editor();
    e.feed_text(":vs<cr>:vs<cr>");
    window(&mut e, "c");
    assert_eq!(outline(&e), r#"["row",[1,2]]"#);
    assert_eq!(e.active_pane(), 1, "the previous neighbour");
    e.focus_pane(0);
    window(&mut e, "c");
    assert_eq!(outline(&e), "1");
    assert_eq!(e.active_pane(), 0, "with none before it, the next");
}

#[test]
fn a_close_collapses_and_splices_single_children() {
    let mut e = editor();
    // ["row",[1,["col",[["row",[2,3]],4]]]]
    e.feed_text(":vs<cr>:sp<cr>");
    window(&mut e, "k");
    e.feed_text(":vs<cr>");
    assert_eq!(outline(&e), r#"["row",[1,["col",[["row",[2,3]],4]]]]"#);
    e.focus_pane(3);
    window(&mut e, "c");
    assert_eq!(
        outline(&e),
        r#"["row",[1,2,3]]"#,
        "the inner row joins its grandparent"
    );
    assert!(e.layout().well_formed());
}

#[test]
fn only_keeps_the_current_pane_of_a_nest() {
    let mut e = editor();
    e.feed_text(":sp<cr>:vs<cr>");
    e.feed_text(":only<cr>");
    assert_eq!(
        (outline(&e), e.active_pane(), e.panes.len()),
        ("1".into(), 0, 1)
    );
}

/// 0104 §6 (c): the grid; every direction from every pane, no wrap.
#[test]
fn directions_go_to_the_neighbour_and_stop_at_the_edge() {
    let mut e = editor();
    e.feed_text(":sp<cr>");
    window(&mut e, "k");
    e.feed_text(":vs<cr>");
    window(&mut e, "j");
    e.feed_text(":vs<cr>");
    assert_eq!(outline(&e), r#"["col",[["row",[1,2]],["row",[3,4]]]]"#);
    // (from, key) → to; None stays put
    let cases = [
        (0, 'l', 1),
        (0, 'j', 2),
        (0, 'h', 0),
        (0, 'k', 0),
        (1, 'h', 0),
        (1, 'j', 3),
        (1, 'l', 1),
        (1, 'k', 1),
        (2, 'k', 0),
        (2, 'l', 3),
        (2, 'h', 2),
        (2, 'j', 2),
        (3, 'k', 1),
        (3, 'h', 2),
        (3, 'l', 3),
        (3, 'j', 3),
    ];
    for (from, key, to) in cases {
        e.focus_pane(from);
        window(&mut e, &key.to_string());
        assert_eq!(e.active_pane(), to, "{from} {key}");
    }
}

/// `k` from a wide lower pane goes to the upper pane over the cursor.
#[test]
fn a_direction_follows_the_cursor_column() {
    let line = "x".repeat(70);
    let mut e = Editor::new(Buffer::from_text(&format!("{line}\n")));
    e.feed_text(":sp<cr>");
    window(&mut e, "k");
    e.feed_text(":vs<cr>");
    window(&mut e, "j");
    assert_eq!(outline(&e), r#"["col",[["row",[1,2]],3]]"#);
    e.prepare_view(ViewGeometry {
        columns: 80,
        rows: 24,
    });
    window(&mut e, "k");
    assert_eq!(e.active_pane(), 0, "cursor in column 0");
    window(&mut e, "j");
    e.feed_text("$");
    e.prepare_view(ViewGeometry {
        columns: 80,
        rows: 24,
    });
    window(&mut e, "k");
    assert_eq!(e.active_pane(), 1, "cursor at column 69");
}

/// 0104 §5.2–§5.3: the cells of the layouts strop could already show, and
/// of the owner's case.
#[test]
fn placement_numbers() {
    let area = CellRect::new(0, 0, 80, 24);
    let one = Layout::Leaf.place(area);
    assert_eq!(one.panes, vec![CellRect::new(0, 0, 80, 23)]);
    let mut stacked = Layout::Leaf;
    stacked.split(0, Axis::Column);
    let placed = stacked.place(area);
    assert_eq!(
        placed.panes,
        vec![CellRect::new(0, 1, 80, 10), CellRect::new(0, 12, 80, 11)]
    );
    assert!(placed.dividers.is_empty());
    let mut side = Layout::Leaf;
    side.split(0, Axis::Row);
    let placed = side.place(area);
    assert_eq!(
        placed.panes,
        vec![CellRect::new(0, 1, 39, 22), CellRect::new(40, 1, 40, 22)]
    );
    assert_eq!(placed.dividers, vec![CellRect::new(39, 0, 1, 23)]);
    let mut nested = stacked.clone();
    nested.split(1, Axis::Row);
    let placed = nested.place(area);
    assert_eq!(
        placed.panes,
        vec![
            CellRect::new(0, 1, 80, 10),
            CellRect::new(0, 12, 39, 11),
            CellRect::new(40, 12, 40, 11),
        ]
    );
    assert_eq!(placed.dividers, vec![CellRect::new(39, 11, 1, 12)]);
}

/// A tiny deterministic generator (no new dependency).
struct Lcg(u64);
impl Lcg {
    fn next(&mut self, bound: usize) -> usize {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        ((self.0 >> 33) as usize) % bound
    }
}

/// 0104 §8.2: random split/close sequences keep the tree well formed, and
/// every placement paints each cell above the modeline at most once —
/// exactly once when the area holds the tree's minimum — with no pane
/// smaller than a cell then.
#[test]
fn random_trees_tile_their_area() {
    let mut rng = Lcg(0x5eed);
    for _ in 0..600 {
        let mut layout = Layout::Leaf;
        let mut count = 1usize;
        let mut active = 0usize;
        for _ in 0..1 + rng.next(10) {
            match rng.next(4) {
                0 | 1 => {
                    let axis = if rng.next(2) == 0 {
                        Axis::Row
                    } else {
                        Axis::Column
                    };
                    layout.split(active, axis);
                    count += 1;
                    active += 1;
                }
                2 if count > 1 => {
                    active = layout.remove(active);
                    count -= 1;
                }
                _ => active = (active + 1) % count,
            }
            assert!(layout.well_formed(), "{}", layout.outline());
            assert_eq!(layout.leaves(), count);
            assert!(active < count);
        }
        let (min_w, min_h) = minimum(&layout);
        for (width, height) in [
            (1, 1),
            (7, 5),
            (24, 10),
            (48, 16),
            (80, 24),
            (101, 37),
            (200, 60),
        ] {
            let placed = layout.place(CellRect::new(0, 0, width, height));
            assert_eq!(placed.panes.len(), count);
            let rows = usize::from(height - 1);
            let mut grid = vec![0u8; usize::from(width) * usize::from(height)];
            let mut paint = |rect: CellRect| {
                for y in rect.y..rect.y + rect.height {
                    for x in rect.x..rect.x + rect.width {
                        assert!(
                            usize::from(y) < rows,
                            "{} at {width}x{height}",
                            layout.outline()
                        );
                        grid[usize::from(y) * usize::from(width) + usize::from(x)] += 1;
                    }
                }
            };
            for pane in &placed.panes {
                let titled = count > 1 && pane.width > 0 && pane.height > 0;
                paint(if titled {
                    CellRect::new(pane.x, pane.y - 1, pane.width, pane.height + 1)
                } else {
                    *pane
                });
            }
            for divider in &placed.dividers {
                paint(*divider);
            }
            let fits = usize::from(width) >= min_w && rows >= min_h;
            for (index, cells) in grid[..rows * usize::from(width)].iter().enumerate() {
                assert!(
                    *cells <= 1,
                    "{} at {width}x{height}: cell {index}",
                    layout.outline()
                );
                if fits {
                    assert_eq!(
                        *cells,
                        1,
                        "{} at {width}x{height}: cell {index}",
                        layout.outline()
                    );
                }
            }
            if fits {
                assert!(placed
                    .panes
                    .iter()
                    .all(|pane| pane.width >= 1 && pane.height >= 1));
            }
        }
    }
}
