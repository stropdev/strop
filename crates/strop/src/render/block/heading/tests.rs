use super::*;
use ratatui::style::Modifier;

fn marks(t: &Tokens, modified: bool, readonly: bool) -> Vec<Span<'static>> {
    let mut marks = Vec::new();
    if modified {
        marks.push(Span::styled(" ●", Style::default().fg(t.warning)));
    }
    if readonly {
        marks.push(Span::styled(" [RO]", Style::default().fg(t.muted)));
    }
    marks
}
use strop_core::theme::ThemeId;

fn t() -> Tokens {
    Tokens::new(ThemeId::Hone)
}

fn text(drawn: &Drawn) -> String {
    drawn
        .gutter
        .iter()
        .chain(&drawn.text)
        .map(|span| span.content.as_ref())
        .collect()
}

fn file<'a>(label: &'a Label, marks: Vec<Span<'static>>, count: &str) -> Heading<'a> {
    Heading {
        gutter: 5,
        ruled: true,
        name: Name::Path(label),
        marks,
        fact: heading_fact(count),
        focus: Focus::Here,
    }
}

fn heading_fact(count: &str) -> Vec<Span<'static>> {
    fact(&t(), count)
}

/// §3.9's pictures, row for row (content widths: the scrollbar column
/// is outside the heading).
#[test]
fn narrow_widths_give_way_in_table_order() {
    let long = Label::local(
        "crates/strop-engine/src/editor/collections/tests/a_very_long_module_name_for_projection_updates.rs",
    );
    let short = Label::local("src/b.rs");
    let t = t();
    let cases = [
        (&long, false, "10 hits", 119, "──── crates/strop-engine/src/editor/collections/tests/a_very_long_module_name_for_projection_updates.rs ─────── 10 hits"),
        (&long, false, "10 hits", 79, "──── …tions/tests/a_very_long_module_name_for_projection_updates.rs ─── 10 hits"),
        (&long, false, "10 hits", 47, "──── a_very_long_module_name_for_proj…  10 hits"),
        (&long, false, "10 hits", 31, "──── a_very_long_modu…  10 hits"),
        (&short, true, "2 hits", 47, "──── src/b.rs ● ──────────────────────── 2 hits"),
        (&short, true, "2 hits", 23, "──── b.rs ● ──── 2 hits"),
    ];
    for (label, modified, fact, cells, want) in cases {
        let heading = file(label, marks(&t, modified, false), fact);
        let drawn = heading.draw(&t, cells);
        assert_eq!(text(&drawn), want, "at {cells}");
        assert_eq!(clip::width(&text(&drawn)), cells, "at {cells}");
    }
}

#[test]
fn inks_follow_the_cursor_block() {
    let t = t();
    let label = Label::local("src/b.rs");
    let ink = |focus: Focus| {
        let heading = Heading {
            focus,
            ..file(&label, Vec::new(), "2 hits")
        };
        let drawn = heading.draw(&t, 40);
        let rule = drawn.gutter[0].style.fg;
        let name = drawn
            .text
            .iter()
            .find(|span| span.content == "b.rs")
            .unwrap()
            .style;
        (rule, name.fg, name.add_modifier.contains(Modifier::BOLD))
    };
    assert_eq!(ink(Focus::Here), (Some(t.muted), Some(t.text), true));
    assert_eq!(
        ink(Focus::Elsewhere),
        (Some(t.faint), Some(t.secondary), true)
    );
    assert_eq!(ink(Focus::Plain), (Some(t.faint), Some(t.text), true));
}

#[test]
fn words_are_cut_at_the_end_with_an_ellipsis() {
    let spans = vec![
        Span::styled("commit ", Style::default()),
        Span::styled("19b4e9aec5", Style::default()),
    ];
    let cut = clip_spans(spans.clone(), 7);
    let joined: String = cut.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(joined, "commit…");
    assert_eq!(clip_spans(spans.clone(), 17), spans);
    let wide = clip_spans(vec![Span::raw("界界界")], 4);
    let joined: String = wide.iter().map(|s| s.content.as_ref()).collect();
    assert_eq!(joined, "界 …");
    assert!(clip_spans(spans, 0).is_empty());
}

fn sizes(name: usize, dir: usize, marks: usize, fact: usize) -> Sizes {
    let mut sizes = Sizes::default();
    sizes[Part::Gutter as usize] = Size::whole(5);
    sizes[Part::Name as usize] = Size {
        floor: name.min(NAME_FLOOR),
        want: name,
    };
    sizes[Part::Marks as usize] = Size::whole(marks);
    sizes[Part::Fact as usize] = Size::whole(if fact == 0 { 0 } else { fact + GAP });
    sizes[Part::Leader as usize] = Size::whole(if fact == 0 { LEADER + 1 } else { LEADER });
    sizes[Part::Directory as usize] = Size {
        floor: dir.min(DIRECTORY_FLOOR),
        want: dir,
    };
    sizes
}

const TABLE: [Part; 7] = [
    Part::Gutter,
    Part::Name,
    Part::Marks,
    Part::Authority,
    Part::Fact,
    Part::Leader,
    Part::Directory,
];

/// §9: every cell is accounted for; no part below its floor or above
/// what it wants; a part that shrank leaves nothing to the parts after
/// it in the table.
#[test]
fn the_allocator_accounts_for_every_cell_and_respects_floors() {
    for name in [1, 3, 8, 9, 30, 200] {
        for dir in [0, 1, 5, 60] {
            for (marks, fact) in [(0, 0), (2, 6), (7, 1), (0, 12)] {
                let sizes = sizes(name, dir, marks, fact);
                for cells in 0..=200 {
                    let grants = layout(&sizes, cells);
                    let total: usize = TABLE.iter().map(|part| grants.get(*part)).sum();
                    assert_eq!(total + grants.spare(), cells);
                    for part in TABLE {
                        let got = grants.get(part);
                        let size = sizes[part as usize];
                        assert!(got <= size.want, "{part:?} over its want");
                        if got != 0 {
                            assert!(got >= size.floor, "{part:?} under its floor");
                        }
                    }
                    let name_cut = grants.get(Part::Name) != 0
                        && grants.get(Part::Name) < sizes[Part::Name as usize].want;
                    if name_cut {
                        assert_eq!(grants.get(Part::Leader), 0);
                        assert_eq!(grants.get(Part::Directory), 0);
                        assert_eq!(grants.spare(), 0, "a cut name takes every free cell");
                    }
                    let dir_cut = grants.get(Part::Directory) != 0
                        && grants.get(Part::Directory) < sizes[Part::Directory as usize].want;
                    if dir_cut {
                        assert_eq!(grants.spare(), 0, "a cut directory takes every free cell");
                    }
                }
            }
        }
    }
}

/// §9: the heading fits at every width from 1 to 200 with names of 1 to
/// 200 cells, wide characters included.
#[test]
fn the_heading_fills_every_width_exactly() {
    let t = t();
    for name in [
        "a",
        "src/lib.rs",
        "界/界界界界界界界界.rs",
        &"x".repeat(200),
    ] {
        let label = Label::local(name);
        for fact in ["", "1 hit", "+12 -3"] {
            for cells in 1..=200 {
                let heading = file(&label, marks(&t, true, true), fact);
                let drawn = heading.draw(&t, cells);
                let width = clip::width(&text(&drawn));
                if cells >= 5 {
                    assert_eq!(width, cells, "{name:?} {fact:?} at {cells}");
                } else {
                    assert!(width <= cells, "{name:?} {fact:?} at {cells}");
                }
                if !fact.is_empty() && width == cells && text(&drawn).contains(fact) {
                    assert!(text(&drawn).ends_with(fact), "the fact is right-aligned");
                }
            }
        }
    }
}
