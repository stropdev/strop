//! Native source surface over the shared semantic `ViewSnapshot`. The GUI
//! draws the same pane identities, revisions, bounds and bounded text windows
//! the backend prepared; it does not clone or reinterpret document state.

use gpui::{div, prelude::*, px, rgb, text};
use strop_ui_protocol::{PaneSnapshot, ViewSnapshot};

/// Render the shared semantic view as native source cards. Presentation
/// composition is frontend work; every source identity/bound comes from the
/// backend publication.
pub fn view_surface(view: Option<&ViewSnapshot>) -> impl IntoElement {
    let generation = view.map_or(0, |view| view.generation);
    let mut cards = div()
        .id("source-surface")
        .role(gpui::Role::Document)
        .aria_label(format!("Strop source surface, generation {generation}"))
        .size_full()
        .flex()
        .flex_col()
        .gap_2()
        .bg(rgb(0x11111b))
        .text_color(rgb(0xcdd6f4))
        .p_4();
    if let Some(view) = view {
        for (index, pane) in view.panes.iter().enumerate() {
            cards = cards.child(pane_card(index, view.active_pane == index, pane));
        }
    } else {
        cards = cards.child(text!("waiting for the backend's first source window"));
    }
    cards
}

fn pane_card(index: usize, active: bool, pane: &PaneSnapshot) -> impl IntoElement {
    let mut card = div()
        .id(("pane", index))
        .role(gpui::Role::Group)
        .aria_label(format!(
            "Source pane {}, document {:?}, revision {}, {:?} bounds",
            index, pane.document, pane.revision, pane.bounds
        ))
        .flex()
        .flex_col()
        .gap_1()
        .p_3()
        .border_1()
        .border_color(if active { rgb(0x89b4fa) } else { rgb(0x45475a) })
        .bg(rgb(0x181825))
        .child(text!(format!(
            "pane {} · document {:?} · revision {} · {:?} · cursor {}",
            index, pane.document, pane.revision, pane.bounds, pane.cursor
        )));
    for (offset, line) in pane.lines.iter().enumerate() {
        card = card.child(text!(format!(
            "{:>5}  {}",
            pane.window_top + offset + 1,
            line
        )));
    }
    card
}
