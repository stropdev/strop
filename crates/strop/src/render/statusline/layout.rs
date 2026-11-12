//! The modeline's width plan (0098 §3.3): what each part of the row
//! needs, and which parts give way when the row runs out of cells.
//!
//! Layout is one pass over one table. In [`ORDER`], each part takes
//! what it wants from the cells still free: its natural width if that
//! fits, less if it must, nothing if not even its floor fits. So the
//! part lowest in the table shrinks first, shrinks to its floor before
//! it is dropped, and is dropped before the part above it loses a cell
//! — a key typed on a full row costs its own width and no more.

use crate::render::clip::width;

use super::{chip, Modeline};

/// Groups are two cells apart; so are the two halves at their closest.
pub(super) const SEP: usize = 2;
/// The pending keys' slot: typed keys grow to the right inside it.
const SLOT: usize = 8;
/// Smallest fragment still worth its cells (a directory hint, a name).
const MIN_GROUP: usize = 4;
/// A spec shorter than this is noise, not information.
const MIN_SPEC: usize = 3;
/// A branch shortens to this before it leaves (`feature…`).
const MIN_BRANCH: usize = 8;
/// `100%`.
const PERCENT: usize = 4;

/// One thing the row can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Part {
    Chip,
    Rec,
    Git,
    Namespace,
    Dir,
    Name,
    Spec,
    SlotPad,
    Attention,
    Hints,
    Services,
    Diag,
    Indent,
    Search,
    Percent,
    Position,
}

const PARTS: usize = 16;

/// Who is served first, with the share of the free cells a part may
/// take on its turn. The first is the last to leave; the last shrinks
/// first. The filename stops at two thirds on its first turn, so a huge
/// name cannot starve the grammar footer, and takes the rest after it.
const ORDER: [(Part, usize, usize); PARTS + 1] = [
    (Part::Chip, 1, 1),
    (Part::Position, 1, 1),
    (Part::Services, 1, 1),
    (Part::Name, 2, 3),
    (Part::Spec, 1, 1),
    (Part::Name, 1, 1),
    (Part::Namespace, 1, 1),
    (Part::Rec, 1, 1),
    (Part::Attention, 1, 1),
    (Part::Hints, 1, 1),
    (Part::Diag, 1, 1),
    (Part::Search, 1, 1),
    (Part::Indent, 1, 1),
    (Part::Percent, 1, 1),
    (Part::Git, 1, 1),
    (Part::Dir, 1, 1),
    (Part::SlotPad, 1, 1),
];

/// The row without these is not a row: it falls back to the bare
/// `chip · position` layout.
const fn essential(part: Part) -> bool {
    matches!(part, Part::Chip | Part::Position | Part::Services)
}

/// A part that only makes sense beside another.
const fn owner(part: Part) -> Option<Part> {
    match part {
        Part::Namespace | Part::Dir => Some(Part::Name),
        Part::SlotPad => Some(Part::Spec),
        _ => None,
    }
}

/// What a part needs: the separator before it, the fewest cells worth
/// showing, and the cells it would take with room to spare. A part
/// with nothing to show has `natural == 0` and no `lead`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Size {
    pub lead: usize,
    pub floor: usize,
    pub natural: usize,
}

impl Size {
    fn fixed(lead: usize, cells: usize) -> Self {
        Self::elastic(lead, cells, cells)
    }

    fn elastic(lead: usize, floor: usize, natural: usize) -> Self {
        if natural == 0 {
            return Self::default();
        }
        Self {
            lead,
            floor: floor.min(natural),
            natural,
        }
    }
}

pub(super) type Sizes = [Size; PARTS];

/// The cells each shown part was given; `None` is a part that is not
/// on the row.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Grants {
    cells: [Option<usize>; PARTS],
    spare: usize,
}

impl Grants {
    pub(super) fn get(&self, part: Part) -> Option<usize> {
        self.cells[part as usize]
    }

    /// The cells no part took: the flexible gap between the halves.
    pub(super) fn spare(&self) -> usize {
        self.spare
    }
}

/// Share `cells` among the parts. `None` when even the essential parts
/// do not fit.
pub(super) fn allot(sizes: &Sizes, cells: usize) -> Option<Grants> {
    let mut granted: [Option<usize>; PARTS] = [None; PARTS];
    let mut served = [false; PARTS];
    let mut spare = cells;
    for (part, share, of) in ORDER {
        let size = sizes[part as usize];
        if std::mem::replace(&mut served[part as usize], true) {
            // a second turn: top up what the first one left short
            if let Some(have) = granted[part as usize] {
                let more = (size.natural - have).min(spare * share / of);
                granted[part as usize] = Some(have + more);
                spare -= more;
            }
            continue;
        }
        if size.natural == 0 {
            continue;
        }
        if owner(part).is_some_and(|owner| granted[owner as usize].is_none()) {
            continue;
        }
        let Some(room) = spare
            .checked_sub(size.lead)
            .filter(|room| *room >= size.floor)
        else {
            if essential(part) {
                return None;
            }
            continue;
        };
        let take = size.natural.min(room * share / of).max(size.floor);
        granted[part as usize] = Some(take);
        spare -= size.lead + take;
    }
    Some(Grants {
        cells: granted,
        spare,
    })
}

impl Modeline {
    /// The cells each part gets at this width; `None` when the row is
    /// too narrow for anything but [`Modeline::bare`].
    pub(super) fn fit(&self, cells: usize) -> Option<Grants> {
        let mut sizes = self.sizes();
        let grants = allot(&sizes, cells)?;
        // Hints are whole chords (rule 10): the cells a cut chord would
        // have taken go back to the parts below.
        let Some(given) = grants.get(Part::Hints) else {
            return Some(grants);
        };
        let whole = self.hints_cells(self.hints_shown(given));
        if whole == given {
            return Some(grants);
        }
        sizes[Part::Hints as usize] = Size::fixed(SEP, whole);
        allot(&sizes, cells)
    }

    /// Degenerate widths — not even the chip, a separator and the
    /// position fit: the chip text and the cells it may take, the
    /// position getting the rest. Never an empty row. A failed service,
    /// dirty and read-only ride inside the chip, first, so cutting the
    /// chip keeps them.
    pub(super) fn bare(&self, cells: usize) -> (String, usize) {
        let safety = format!(
            "{}{}{}{}",
            self.services.as_ref().map_or("", |_| "!"),
            if self.dirty { "*" } else { "" },
            if self.readonly { "RO" } else { "" },
            self.multicursor.as_deref().unwrap_or("")
        );
        let chip = if safety.is_empty() {
            self.chip.clone()
        } else {
            format!("{safety} {}", self.chip.trim())
        };
        let position = digits(self.line) + 1 + digits(self.col);
        let chip_cells = cells.saturating_sub(position.min((cells / 2).max(MIN_GROUP)));
        (chip, chip_cells)
    }

    fn sizes(&self) -> Sizes {
        let mut sizes = [Size::default(); PARTS];
        let mut set = |part: Part, size: Size| sizes[part as usize] = size;
        set(Part::Chip, Size::fixed(0, chip::WIDTH));
        set(Part::Rec, Size::fixed(SEP, width(&self.rec)));
        let branch = width(&self.git_context);
        if branch > 0 {
            let marks = usize::from(self.worktree_dirty) + usize::from(self.staged_mark);
            set(
                Part::Git,
                Size::elastic(SEP, branch.min(MIN_BRANCH) + marks, branch + marks),
            );
        }
        if self.has_file() {
            let name = width(&self.name);
            let marks = self.file_marks();
            set(
                Part::Name,
                Size::elastic(SEP, name.min(MIN_GROUP) + marks, name + marks),
            );
            let namespace = width(&self.namespace);
            if namespace > 0 {
                // the namespace carries the cell that parts it from the path
                set(
                    Part::Namespace,
                    Size::elastic(0, namespace.min(MIN_GROUP) + 1, namespace + 1),
                );
            }
            set(Part::Dir, Size::elastic(0, MIN_GROUP, width(&self.dir)));
        }
        let spec = width(&self.spec);
        set(Part::Spec, Size::elastic(SEP, MIN_SPEC, spec));
        if spec > 0 {
            set(
                Part::SlotPad,
                Size::elastic(0, 0, SLOT.saturating_sub(spec)),
            );
        }
        if !self.attention.is_empty() {
            let badges: usize = self.attention.iter().map(|badge| width(badge)).sum();
            set(
                Part::Attention,
                Size::fixed(SEP, badges + SEP * (self.attention.len() - 1)),
            );
        }
        set(
            Part::Hints,
            Size::elastic(SEP, self.hints_cells(1), self.hints_cells(self.hints.len())),
        );
        if let Some(badge) = &self.services {
            set(Part::Services, Size::fixed(SEP, badge.width()));
        }
        if self.show_diag() {
            let errors = if self.errors > 0 {
                1 + digits(self.errors)
            } else {
                0
            };
            let warnings = if self.warnings > 0 {
                1 + digits(self.warnings)
            } else {
                0
            };
            set(
                Part::Diag,
                Size::fixed(
                    SEP,
                    errors + warnings + usize::from(self.errors > 0 && self.warnings > 0),
                ),
            );
        }
        set(Part::Indent, Size::fixed(SEP, width(&self.indent)));
        if let Some((_, total)) = self.search {
            set(Part::Search, Size::fixed(SEP, 2 * digits(total) + 1));
        }
        if self.percent.is_some() {
            set(Part::Percent, Size::fixed(SEP, PERCENT));
        }
        set(
            Part::Position,
            Size::fixed(SEP, self.line_cells() + 1 + self.col_cells()),
        );
        sizes
    }

    /// `●`, `[RO]` and the cursor count behind the filename.
    fn file_marks(&self) -> usize {
        2 * usize::from(self.dirty)
            + 5 * usize::from(self.readonly)
            + self.multicursor.as_ref().map_or(0, |mark| 1 + width(mark))
    }

    /// The cells the filename itself gets out of the file group's grant.
    pub(super) fn name_cells(&self, granted: usize) -> usize {
        granted.saturating_sub(self.file_marks())
    }

    /// The line number's field: as wide as the buffer's last line
    /// number, so the colon stays put while the cursor moves.
    pub(super) fn line_cells(&self) -> usize {
        digits(self.lines.max(self.line))
    }

    /// The column's field: at least two cells, then one cell of edge
    /// padding. Its slack falls at the screen edge.
    pub(super) fn col_cells(&self) -> usize {
        (digits(self.col) + 1).max(3)
    }

    /// How many whole hints fit in `granted` cells (rule 10).
    pub(super) fn hints_shown(&self, granted: usize) -> usize {
        (0..=self.hints.len())
            .rev()
            .find(|shown| self.hints_cells(*shown) <= granted)
            .unwrap_or(0)
    }

    /// The cells the first `shown` hints take, separators included.
    fn hints_cells(&self, shown: usize) -> usize {
        let hints: usize = self.hints.iter().take(shown).map(hint_width).sum();
        hints + SEP * shown.saturating_sub(1)
    }
}

/// A keycap (` key `), one cell, the description.
fn hint_width((key, desc): &(String, &'static str)) -> usize {
    width(key) + 2 + 1 + width(desc)
}

pub(super) fn digits(value: usize) -> usize {
    value
        .checked_ilog10()
        .map_or(1, |digits| digits as usize + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sizes(parts: &[(Part, Size)]) -> Sizes {
        let mut sizes = [Size::default(); PARTS];
        sizes[Part::Chip as usize] = Size::fixed(0, 10);
        sizes[Part::Position as usize] = Size::fixed(SEP, 5);
        for (part, size) in parts {
            sizes[*part as usize] = *size;
        }
        sizes
    }

    /// A row with everything on it: long branch, long path, a footer.
    fn full() -> Sizes {
        sizes(&[
            (Part::Rec, Size::fixed(SEP, 6)),
            (Part::Git, Size::elastic(SEP, 9, 25)),
            (Part::Namespace, Size::elastic(0, 5, 9)),
            (Part::Dir, Size::elastic(0, 4, 35)),
            (Part::Name, Size::elastic(SEP, 6, 26)),
            (Part::Spec, Size::elastic(SEP, 3, 30)),
            (Part::SlotPad, Size::elastic(0, 0, 0)),
            (Part::Hints, Size::elastic(SEP, 12, 40)),
            (Part::Services, Size::fixed(SEP, 4)),
            (Part::Diag, Size::fixed(SEP, 5)),
            (Part::Indent, Size::fixed(SEP, 8)),
            (Part::Search, Size::fixed(SEP, 5)),
            (Part::Percent, Size::fixed(SEP, 4)),
        ])
    }

    fn parts() -> Vec<Part> {
        let mut parts = Vec::new();
        for (part, _, _) in ORDER {
            if !parts.contains(&part) {
                parts.push(part);
            }
        }
        parts
    }

    #[test]
    fn every_width_stays_inside_the_row_and_inside_each_parts_bounds() {
        let sizes = full();
        for cells in 0..400 {
            let Some(grants) = allot(&sizes, cells) else {
                assert!(
                    cells < 10 + SEP + 5 + SEP + 4,
                    "bare only when tiny: {cells}"
                );
                continue;
            };
            let mut used = grants.spare();
            for part in parts() {
                let size = sizes[part as usize];
                match grants.get(part) {
                    Some(given) => {
                        assert!(given >= size.floor, "{part:?} under its floor at {cells}");
                        assert!(given <= size.natural, "{part:?} over natural at {cells}");
                        used += size.lead + given;
                    }
                    None => assert!(
                        !essential(part) || size.natural == 0,
                        "{part:?} missing at {cells}"
                    ),
                }
            }
            assert_eq!(used, cells, "every cell is accounted for at {cells}");
        }
    }

    #[test]
    fn a_wide_row_gives_every_part_its_natural_width() {
        let sizes = full();
        let grants = allot(&sizes, 400).unwrap();
        for part in parts() {
            let size = sizes[part as usize];
            if size.natural > 0 {
                assert_eq!(grants.get(part), Some(size.natural), "{part:?}");
            }
        }
    }

    /// The directory shrinks to its floor, then leaves; only then does
    /// the branch lose a cell; the filename is whole throughout.
    #[test]
    fn the_lowest_part_shrinks_then_leaves_before_the_next_gives_a_cell() {
        let sizes = sizes(&[
            (Part::Git, Size::elastic(SEP, 9, 25)),
            (Part::Dir, Size::elastic(0, 4, 35)),
            (Part::Name, Size::fixed(SEP, 24)),
            (Part::Diag, Size::fixed(SEP, 5)),
        ]);
        let natural = 10 + 27 + 35 + 26 + 7 + 7;
        let mut branch_shortened = false;
        for cells in ((natural - 35 - 16)..=natural).rev() {
            let grants = allot(&sizes, cells).unwrap();
            assert_eq!(grants.get(Part::Name), Some(24), "name whole at {cells}");
            assert_eq!(grants.get(Part::Diag), Some(5), "{cells}");
            let branch = grants.get(Part::Git).expect("the branch stays");
            match grants.get(Part::Dir) {
                Some(dir) => {
                    assert!(dir >= 4, "{cells}");
                    assert_eq!(branch, 25, "branch whole while the directory shows");
                }
                None => branch_shortened |= branch < 25,
            }
        }
        assert!(branch_shortened, "the branch shortens instead of leaving");
    }

    /// Narrowing the row, parts leave in the reverse of the table.
    #[test]
    fn parts_leave_from_the_bottom_of_the_table() {
        let sizes = full();
        let first_absent = |part: Part| {
            (0..400)
                .rev()
                .find(|&cells| allot(&sizes, cells).is_none_or(|grants| grants.get(part).is_none()))
                .unwrap()
        };
        let order = [
            Part::Dir,
            Part::Git,
            Part::Percent,
            Part::Indent,
            Part::Search,
            Part::Diag,
            Part::Hints,
            Part::Rec,
            Part::Namespace,
            Part::Spec,
            Part::Name,
        ];
        for pair in order.windows(2) {
            assert!(
                first_absent(pair[0]) > first_absent(pair[1]),
                "{:?} leaves before {:?}",
                pair[0],
                pair[1]
            );
        }
    }

    #[test]
    fn a_huge_name_cannot_starve_the_footer() {
        let sizes = sizes(&[
            (Part::Name, Size::elastic(SEP, 4, 200)),
            (Part::Spec, Size::elastic(SEP, 3, 40)),
        ]);
        let grants = allot(&sizes, 80).unwrap();
        let spec = grants.get(Part::Spec).unwrap();
        let name = grants.get(Part::Name).unwrap();
        assert!(spec >= 18, "the footer keeps a third: {spec}");
        assert!(name > spec, "the name still gets the larger share: {name}");
    }

    #[test]
    fn a_part_needs_its_owner() {
        let sizes = sizes(&[
            (Part::Dir, Size::elastic(0, 4, 20)),
            (Part::Name, Size::fixed(SEP, 12)),
        ]);
        // room for the directory's floor but not for the name
        let grants = allot(&sizes, 10 + SEP + 5 + 6).unwrap();
        assert_eq!(grants.get(Part::Name), None);
        assert_eq!(grants.get(Part::Dir), None);
    }
}
