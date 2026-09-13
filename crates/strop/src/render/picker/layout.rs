//! Pure picker card geometry shared by preparation (scroll clamping) and
//! paint, so both agree on the exact same results area.
use ratatui::layout::{Constraint, Direction, Layout, Rect};
use strop_picker::Kind;

/// The floating/workspace card rect for the open kind (0050 §8).
pub(super) fn card(area: Rect, kind: Kind) -> Rect {
    if matches!(kind, Kind::Search | Kind::Files | Kind::Symbols) {
        Rect {
            x: area.x + 1,
            y: area.y,
            width: area.width.saturating_sub(2),
            height: area.height.saturating_sub(1),
        }
    } else {
        let width = ((u32::from(area.width) * 84 / 100) as u16)
            .max(50)
            .min(area.width.saturating_sub(2));
        let height = if kind == Kind::RemoteAddress {
            8
        } else {
            ((u32::from(area.height) * 70 / 100) as u16).max(12)
        }
        .min(area.height.saturating_sub(2));
        Rect {
            x: (area.width - width) / 2,
            y: (area.height - height) / 2,
            width,
            height,
        }
    }
}

/// The results list area for the open kind: the same input/content split and
/// narrow-terminal stacking the paint performs. `None` when the card is too
/// short to show results at all — the active field owns the card and the
/// results viewport must not move.
pub(super) fn results(area: Rect, kind: Kind, replace_visible: bool) -> Option<Rect> {
    let card = card(area, kind);
    let search_mode = kind == Kind::Search;
    let remote_picker = matches!(kind, Kind::RemoteHosts | Kind::RemoteAddress);
    let inner_width = card.width.saturating_sub(4);
    let inner_height = card.height.saturating_sub(2);
    let input_h: u16 = if search_mode && replace_visible { 3 } else { 2 };
    if inner_height < input_h + 1 {
        return None;
    }
    let rows = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Length(input_h), Constraint::Min(1)])
        .split(Rect {
            x: card.x + 2,
            y: card.y + 1,
            width: inner_width,
            height: inner_height,
        });
    if remote_picker {
        return Some(rows[1]);
    }
    if card.width < 64 && rows[1].height >= 12 {
        return Some(
            Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(8), Constraint::Length(8)])
                .split(rows[1])[0],
        );
    }
    if card.width < 64 {
        return Some(rows[1]);
    }
    let list = if search_mode { 60 } else { 55 };
    Some(
        Layout::default()
            .direction(Direction::Horizontal)
            .constraints([
                Constraint::Percentage(list),
                Constraint::Percentage(100 - list),
            ])
            .split(rows[1])[0],
    )
}

/// Rows per logical result row for the open kind.
pub(super) fn per_row(kind: Kind, replace_visible: bool) -> usize {
    if kind == Kind::Search {
        if replace_visible {
            3
        } else {
            2
        }
    } else {
        1
    }
}
