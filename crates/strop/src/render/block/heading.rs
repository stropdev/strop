//! The ruled heading (0100 §3.2): a rule through the gutter, the name at
//! the text column, its marks, a faint leader and the fact at the right
//! edge. One table decides what gives way at narrow widths (§3.9), laid
//! out by one pass the way the modeline is (0098 §3.3): in table order
//! each part takes what it wants from the cells still free, shrinks to
//! its floor, and leaves before the part above it loses a cell.
//!
//! A heading is set into a rule (0103 §4.1): light for a block, heavy for
//! a seam — the place in one buffer where the rows start belonging to
//! another file. Only the collection has seams.

use ratatui::style::{Color, Style};
use ratatui::text::Span;

use crate::render::clip;
use crate::render::file_name::{self, Label, Role};
use crate::render::style::Tokens;

/// The parts of a heading, in the order they are drawn.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Part {
    Gutter,
    Name,
    Marks,
    Authority,
    Fact,
    Leader,
    Directory,
}

const PARTS: usize = 7;

/// One turn of the pass. The name has two: its floor first, the rest it
/// wants once the marks and the fact are placed.
#[derive(Clone, Copy)]
enum Turn {
    Whole(Part),
    NameFloor,
    NameRest,
}

/// §3.9's table: first row = last to leave.
const ORDER: [Turn; 8] = [
    Turn::Whole(Part::Gutter),
    Turn::NameFloor,
    Turn::Whole(Part::Marks),
    Turn::Whole(Part::Authority),
    Turn::Whole(Part::Fact),
    Turn::NameRest,
    Turn::Whole(Part::Leader),
    Turn::Whole(Part::Directory),
];

/// The fewest cells worth showing and the cells a part would like.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Size {
    pub floor: usize,
    pub want: usize,
}

impl Size {
    /// A part shown whole or not at all.
    pub(crate) fn whole(cells: usize) -> Self {
        Self {
            floor: cells,
            want: cells,
        }
    }
}

pub(crate) type Sizes = [Size; PARTS];

/// The cells each part was given, and the cells no part took (they go to
/// the leader, or to the plain gap when there is no leader).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct Grants {
    cells: [usize; PARTS],
    spare: usize,
}

impl Grants {
    pub(crate) fn get(&self, part: Part) -> usize {
        self.cells[part as usize]
    }

    pub(crate) fn spare(&self) -> usize {
        self.spare
    }
}

/// The least gap between a name and its fact when there is no leader.
pub(crate) const GAP: usize = 2;
/// The leader's cells beyond the plain gap: three cells of rule (with
/// the gap's two cells as the space either side).
const LEADER: usize = 3;
/// The name's last component shows at least this many cells.
const NAME_FLOOR: usize = 8;
/// A cut directory is at least `…/`.
const DIRECTORY_FLOOR: usize = 2;

/// Share `cells` among the parts (pure; §9's allocator contract). A part
/// is served only once what it hangs from is: the directory needs the
/// whole file name. When the fact leaves, the leader still runs the
/// rule to the edge, so the headings of a block end together.
pub(crate) fn layout(sizes: &Sizes, cells: usize) -> Grants {
    let mut granted = [0usize; PARTS];
    let mut spare = cells;
    for turn in ORDER {
        let (part, size) = match turn {
            Turn::Whole(part) => (part, sizes[part as usize]),
            Turn::NameFloor => {
                let name = sizes[Part::Name as usize];
                (Part::Name, Size::whole(name.floor))
            }
            Turn::NameRest => {
                let name = sizes[Part::Name as usize];
                let have = granted[Part::Name as usize];
                if have == 0 {
                    continue;
                }
                let more = (name.want - have).min(spare);
                granted[Part::Name as usize] += more;
                spare -= more;
                continue;
            }
        };
        if size.want == 0 {
            continue;
        }
        let served = match part {
            Part::Directory => {
                let name = sizes[Part::Name as usize];
                name.want != 0 && granted[Part::Name as usize] == name.want
            }
            _ => true,
        };
        if !served || spare < size.floor {
            continue;
        }
        let take = size.want.min(spare);
        granted[part as usize] = take;
        spare -= take;
    }
    Grants {
        cells: granted,
        spare,
    }
}

/// The line a heading is set into (0103 §4.1).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Rule {
    /// A blank gutter and a light leader (Search's group header).
    None,
    /// A block of one buffer: status sections, diff and commit files.
    Light,
    /// Another file's rows start here: heavy, always in `fg.muted`.
    Seam,
}

impl Rule {
    fn glyph(self) -> &'static str {
        match self {
            Rule::None | Rule::Light => "─",
            Rule::Seam => "━",
        }
    }
}

/// What the heading names: a file (drawn in the file-name idiom) or any
/// other words, as styled spans (cut at the end).
pub(crate) enum Name<'a> {
    Path(&'a Label),
    Words(Vec<Span<'static>>),
}

/// How the heading reads against the cursor (0088 rule 3).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Focus {
    /// The block the cursor is in: the rule goes to `fg.muted`.
    Here,
    /// Another block: the name's bold ink drops to `fg.secondary`.
    Elsewhere,
    /// A heading with no cursor block (Search's group header).
    Plain,
}

/// A heading to lay out: `gutter` cells (ruled unless `Rule::None`, the
/// last always a space), the name, its marks (each led by its own space), the fact.
pub(crate) struct Heading<'a> {
    pub gutter: usize,
    pub rule: Rule,
    pub name: Name<'a>,
    pub marks: Vec<Span<'static>>,
    pub fact: Vec<Span<'static>>,
    pub focus: Focus,
}

/// A laid-out heading: the gutter's cells and the text column's.
pub(crate) struct Drawn {
    pub gutter: Vec<Span<'static>>,
    pub text: Vec<Span<'static>>,
}

pub(crate) fn spans_width(spans: &[Span<'_>]) -> usize {
    spans.iter().map(|span| clip::width(&span.content)).sum()
}

/// Keep the beginning of styled spans within `cells`, ending in `…` when
/// cut (the ellipsis takes the ink of the text it ends).
pub(crate) fn clip_spans(spans: Vec<Span<'static>>, cells: usize) -> Vec<Span<'static>> {
    if spans_width(&spans) <= cells {
        return spans;
    }
    let mut out: Vec<Span<'static>> = Vec::new();
    if cells == 0 {
        return out;
    }
    let budget = cells - 1;
    let mut used = 0;
    let mut last = spans.first().map_or_else(Style::default, |span| span.style);
    'spans: for span in spans {
        last = span.style;
        let mut kept = String::new();
        for grapheme in
            unicode_segmentation::UnicodeSegmentation::graphemes(span.content.as_ref(), true)
        {
            let width = clip::width(grapheme);
            if used + width > budget {
                if !kept.is_empty() {
                    out.push(Span::styled(kept, span.style));
                }
                break 'spans;
            }
            used += width;
            kept.push_str(strop_core::layout::printable_grapheme(grapheme));
        }
        out.push(Span::styled(kept, span.style));
    }
    if used < budget {
        // a wide grapheme left a cell the ellipsis cannot use
        out.push(Span::raw(" ".repeat(budget - used)));
    }
    out.push(Span::styled("…", last));
    out
}

impl Heading<'_> {
    fn rule_ink(&self, t: &Tokens) -> Color {
        if self.rule == Rule::Seam || self.focus == Focus::Here {
            t.muted
        } else {
            t.faint
        }
    }

    /// The name's spans whole: authority (separate), directory, file name.
    fn name_parts(&self, t: &Tokens) -> (Option<String>, String, Vec<Span<'static>>) {
        match &self.name {
            Name::Path(label) => {
                let (dir, leaf) = file_name::split(&label.path);
                let mut style = file_name::name_style(Role::Heading, t);
                if self.focus == Focus::Elsewhere {
                    style = style.fg(t.secondary);
                }
                (
                    label.authority.clone(),
                    dir.to_owned(),
                    vec![Span::styled(leaf.to_owned(), style)],
                )
            }
            Name::Words(spans) => {
                let mut spans = spans.clone();
                if self.focus == Focus::Elsewhere {
                    for span in &mut spans {
                        if span.style.fg == Some(t.text) {
                            span.style = span.style.fg(t.secondary);
                        }
                    }
                }
                (None, String::new(), spans)
            }
        }
    }

    /// Lay the heading out across `cells` (gutter included).
    pub(crate) fn draw(&self, t: &Tokens, cells: usize) -> Drawn {
        let rule = Style::default().fg(self.rule_ink(t));
        let (authority, dir, leaf) = self.name_parts(t);
        let leaf_w = spans_width(&leaf);
        let marks_w = spans_width(&self.marks);
        let fact_w = spans_width(&self.fact);
        let authority_w = authority.as_deref().map_or(0, |a| clip::width(a) + 1);
        let mut sizes = Sizes::default();
        sizes[Part::Gutter as usize] = Size::whole(self.gutter);
        sizes[Part::Name as usize] = Size {
            floor: leaf_w.min(NAME_FLOOR),
            want: leaf_w,
        };
        sizes[Part::Marks as usize] = Size::whole(marks_w);
        sizes[Part::Authority as usize] = Size::whole(authority_w);
        sizes[Part::Fact as usize] = Size::whole(if fact_w == 0 { 0 } else { fact_w + GAP });
        sizes[Part::Leader as usize] = Size::whole(if fact_w == 0 { LEADER + 1 } else { LEADER });
        let dir_w = clip::width(&dir);
        sizes[Part::Directory as usize] = Size {
            floor: dir_w.min(DIRECTORY_FLOOR),
            want: dir_w,
        };
        let grants = layout(&sizes, cells);

        let gutter_cells = grants.get(Part::Gutter);
        let gutter = if gutter_cells == 0 {
            Vec::new()
        } else if self.rule != Rule::None {
            vec![
                Span::styled(self.rule.glyph().repeat(gutter_cells - 1), rule),
                Span::raw(" "),
            ]
        } else {
            vec![Span::raw(" ".repeat(gutter_cells))]
        };

        let mut text: Vec<Span<'static>> = Vec::new();
        if grants.get(Part::Authority) != 0 {
            if let Some(authority) = authority {
                text.push(Span::styled(
                    format!("{authority} "),
                    file_name::authority_style(t),
                ));
            }
        }
        let dir_cells = grants.get(Part::Directory);
        if dir_cells != 0 {
            // an absolute directory keeps its leading `/` ahead of the `…`
            // (0088 "File names"): it is the mark of outside the workspace
            let shown = match dir.strip_prefix('/') {
                Some(rest) if dir_cells > 2 && dir_cells < dir_w => {
                    format!("/{}", clip::clip_start(rest, dir_cells - 1))
                }
                _ => clip::clip_start(&dir, dir_cells).into_owned(),
            };
            text.push(Span::styled(shown, file_name::dir_style(t)));
        }
        let name_cells = grants.get(Part::Name);
        if name_cells != 0 {
            text.extend(clip_spans(leaf, name_cells));
        }
        if grants.get(Part::Marks) != 0 {
            text.extend(self.marks.iter().cloned());
        }
        let fact = grants.get(Part::Fact) != 0;
        let used = gutter_cells + spans_width(&text) + if fact { fact_w } else { 0 };
        let gap = cells.saturating_sub(used);
        debug_assert!(gap >= grants.spare(), "spare cells belong to the gap");
        if grants.get(Part::Leader) != 0 && gap > LEADER {
            let trailing = usize::from(fact);
            text.push(Span::styled(
                format!(
                    " {}{}",
                    self.rule.glyph().repeat(gap - 1 - trailing),
                    " ".repeat(trailing)
                ),
                rule,
            ));
        } else if gap != 0 {
            text.push(Span::raw(" ".repeat(gap)));
        }
        if fact {
            text.extend(self.fact.iter().cloned());
        }
        Drawn { gutter, text }
    }
}

/// `●` for modified (`diag.warning`), `[RO]` for read-only (`fg.muted`):
/// the modeline's marks (0100 §3.2), each led by one cell.
pub(crate) fn marks(t: &Tokens, modified: bool, readonly: bool) -> Vec<Span<'static>> {
    let mut marks = Vec::new();
    if modified {
        marks.push(Span::styled(" ●", Style::default().fg(t.warning)));
    }
    if readonly {
        marks.push(Span::styled(" [RO]", Style::default().fg(t.muted)));
    }
    marks
}

/// `+N -M` in the diff inks.
pub(crate) fn stats(t: &Tokens, added: usize, deleted: usize) -> Vec<Span<'static>> {
    vec![
        Span::styled(format!("+{added}"), Style::default().fg(t.diff_add)),
        Span::raw(" "),
        Span::styled(format!("-{deleted}"), Style::default().fg(t.diff_del)),
    ]
}

/// A fact in `fg.secondary` (a count, words).
pub(crate) fn fact(t: &Tokens, text: impl Into<String>) -> Vec<Span<'static>> {
    vec![Span::styled(text.into(), Style::default().fg(t.secondary))]
}

#[cfg(test)]
mod tests;
