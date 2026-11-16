//! File finder rows (0106): ` STATUS.md      plans/` — the name, then its
//! folder at one column per list. A name too wide for the column is shown
//! alone and whole, as the folder view does (0102 §2.1).
use ratatui::style::Style;
use ratatui::text::Line;

use super::{emphasis, finish, plain, split_match_cols};
use crate::render::card::row_marker;
use crate::render::clip;
use crate::render::file_name::{self, Role};
use crate::render::style::Tokens;

/// Rows measured for the folder column: the bound the engine already uses
/// to size the list pane (`PICKER_MEASURE_ITEMS`), so the cost per frame
/// is fixed however large the tree.
const NAME_SAMPLE: usize = 256;

/// The folder column, measured once per list and never per window, so it
/// does not move while scrolling: the widest name of the first ranked rows
/// plus a gap, between a third and a half of the row.
pub(super) fn file_name_column(picker: &strop_picker::Picker, width: u16) -> usize {
    let widest = picker
        .rows
        .iter()
        .take(NAME_SAMPLE)
        .filter_map(|row| picker.items.get(row.item))
        .map(|item| clip::width(item.text.rsplit('/').next().unwrap_or(&item.text)))
        .max()
        .unwrap_or(0);
    let w = usize::from(width);
    (widest + 2).clamp(w / 3, w / 2)
}

/// The name bright (query matches on it emphasized), the folder in
/// `fg.secondary` at the list's one column, elided from the left so the
/// parent nearest the file survives. A name too wide for the column is
/// shown alone; `finish` cuts it only when it is wider than the row.
pub(super) fn file_row(
    t: &Tokens,
    value: &str,
    match_cols: &[u32],
    name_column: usize,
    width: u16,
    active: bool,
) -> Line<'static> {
    let (dir, name) = match value.rfind('/') {
        Some(i) => (&value[..i], &value[i + 1..]),
        None => ("", value),
    };
    let name_off = dir.chars().count() + usize::from(!dir.is_empty());
    let (dir_cols, name_cols) = split_match_cols(match_cols, dir.chars().count(), name_off);
    let budget = usize::from(width).saturating_sub(2);
    let mut spans = vec![row_marker(active, t), plain(" ", Style::default())];
    let name_w = clip::width(name);
    spans.extend(emphasis(
        t,
        name,
        &name_cols,
        file_name::name_style(Role::Row, t),
    ));
    let room = budget.saturating_sub(name_column);
    if !dir.is_empty() && name_w + 2 <= name_column && room >= 4 {
        spans.push(plain(" ".repeat(name_column - name_w), Style::default()));
        let shown_dir = format!("{dir}/");
        let shown_dir = if clip::width(&shown_dir) > room {
            clip::clip_start(&shown_dir, room).into_owned()
        } else {
            shown_dir
        };
        // columns only survive while the directory is whole
        let cols: &[u32] = if shown_dir.starts_with('…') {
            &[]
        } else {
            &dir_cols
        };
        spans.extend(emphasis(t, &shown_dir, cols, file_name::dir_style(t)));
    }
    finish(t, spans, width, active)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::style::Modifier;
    use strop_picker::{Item, Kind, Payload, Picker};

    fn t() -> Tokens {
        Tokens::new(strop_core::theme::ThemeId::Hone)
    }

    fn text(line: &Line) -> String {
        line.spans.iter().map(|s| s.content.as_ref()).collect()
    }

    /// The cell (char) index where `needle` starts in a row's text.
    fn column(text: &str, needle: &str) -> Option<usize> {
        text.find(needle).map(|byte| text[..byte].chars().count())
    }

    fn picker(names: &[String]) -> Picker {
        let items = names
            .iter()
            .map(|name| {
                let path = format!("src/{name}");
                Item {
                    badge: None,
                    payload: Payload::File(path.clone().into()),
                    text: path,
                }
            })
            .collect();
        let mut picker = Picker::new(Kind::Files, items, false);
        let ranking = strop_picker::rank::rank(&picker.filter_request(), || false)
            .unwrap()
            .unwrap();
        assert!(picker.install_ranking(ranking));
        assert_eq!(picker.rows.len(), names.len());
        picker
    }

    fn named(cells: usize) -> String {
        "n".repeat(cells)
    }

    #[test]
    fn the_name_leads_and_its_folder_stands_at_the_lists_one_column() {
        let t = t();
        let a = file_row(&t, "plans/STATUS.md", &[0, 1], 14, 44, true);
        let b = file_row(
            &t,
            "crates/strop-engine/src/editor/lsp/status.rs",
            &[],
            14,
            44,
            false,
        );
        let (a_text, b_text) = (text(&a), text(&b));
        assert!(a_text.starts_with("▌ STATUS.md"), "{a_text:?}");
        assert_eq!(column(&a_text, "plans/"), Some(16));
        assert_eq!(column(&b_text, "…"), Some(16), "{b_text:?}");
        assert!(b_text.contains("lsp/"), "elided from the left: {b_text:?}");
        assert_eq!(a.width(), 44);
        for width in 0..30 {
            assert!(
                file_row(&t, "plans/STATUS.md", &[], 14, width, true).width() <= width as usize
            );
        }
    }

    #[test]
    fn a_name_too_wide_for_the_column_stands_alone_and_whole() {
        let t = t();
        let long = "plans/0081-product-coherence-and-workspace-experience-handoff.md";
        let cut = text(&file_row(&t, long, &[], 14, 44, false));
        assert_eq!(
            cut,
            format!(
                "  {}…",
                &"0081-product-coherence-and-workspace-experience-handoff.md"[..41]
            )
        );
        assert!(!cut.contains('/'));

        let name = named(58);
        let whole = file_row(&t, &format!("src/{name}"), &[], 35, 71, false);
        assert!(text(&whole).starts_with(&format!("  {name}")));
        assert!(!text(&whole).contains('/'));
        assert!(!text(&whole).contains('…'));
        let cut = text(&file_row(&t, &format!("src/{name}"), &[], 20, 41, false));
        assert_eq!(cut, format!("  {}…", &name[..38]));
    }

    #[test]
    fn the_column_is_between_a_third_and_a_half_of_the_row() {
        let short = picker(&[named(6)]);
        let middling = picker(&[named(6), named(24)]);
        let long = picker(&[named(6), named(60)]);
        assert_eq!(file_name_column(&short, 71), 23);
        assert_eq!(file_name_column(&middling, 71), 26);
        assert_eq!(file_name_column(&long, 71), 35);
        assert_eq!(file_name_column(&short, 41), 13);
        assert_eq!(file_name_column(&middling, 41), 20);
        assert_eq!(file_name_column(&long, 41), 20);
        let mut deep: Vec<String> = (0..300).map(|_| named(6)).collect();
        deep[299] = named(60);
        assert_eq!(file_name_column(&picker(&deep), 71), 23, "past row 256");
    }

    #[test]
    fn matches_stay_emphasized_on_their_own_cells() {
        let t = t();
        // `src/mod.rs`: `s` of the folder and `mo` of the name
        let line = file_row(&t, "src/mod.rs", &[0, 4, 5], 12, 40, false);
        let mut cells = Vec::new();
        for span in &line.spans {
            let bold = span.style.add_modifier.contains(Modifier::BOLD);
            for c in span.content.chars() {
                cells.push((c, bold));
            }
        }
        let bold: Vec<usize> = (0..cells.len()).filter(|&i| cells[i].1).collect();
        assert_eq!(bold, vec![2, 3, 14], "{cells:?}");
        assert_eq!(cells[14].0, 's');
    }
}
