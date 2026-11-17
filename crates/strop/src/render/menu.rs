//! The list-card primitive (plan 0110 §4 step 5): which rows of a long
//! list are in view, and the count the card's top border shows. Insert
//! completion and the command line's menu draw through it, so a list
//! scrolls and counts the same way wherever it appears.

/// The first row in view: the selected row stays on screen, at the
/// bottom when the list scrolls down.
pub(crate) fn first_visible(selected: Option<usize>, len: usize, rows: usize) -> usize {
    selected
        .map_or(0, |index| index.saturating_sub(rows.saturating_sub(1)))
        .min(len.saturating_sub(rows))
}

/// The count in the top border: `2/9` with a row selected, else `9`.
pub(crate) fn count(selected: Option<usize>, len: usize) -> String {
    match selected {
        Some(index) => format!("{}/{len}", index + 1),
        None => len.to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_selected_row_stays_in_view_and_the_window_never_runs_past_the_end() {
        assert_eq!(first_visible(None, 30, 10), 0);
        assert_eq!(first_visible(Some(9), 30, 10), 0);
        assert_eq!(first_visible(Some(10), 30, 10), 1);
        assert_eq!(first_visible(Some(29), 30, 10), 20);
        assert_eq!(first_visible(Some(3), 4, 10), 0);
        assert_eq!(count(None, 9), "9");
        assert_eq!(count(Some(1), 9), "2/9");
    }
}
