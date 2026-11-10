//! The one file-name idiom (0088 §2 "File names", 2026-11-10): wherever a
//! path is shown, its directory is context in `fg.secondary` — ending in
//! its own `/` — and its basename is the identity in `fg.text`, bold when
//! the name is a heading (a group header, a pane title, a card's subject)
//! and regular inside a list row. Clipping always spends the directory
//! first (`clip_start`), so the name survives; a path that elides from
//! the left keeps its `…` in the directory span.

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::style::Tokens;

/// What the name is doing where it appears (rule 7: bold = headings).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Role {
    /// The subject of a surface: grep group headers, pane titles, the
    /// preview header, the recovery card, a directory buffer's own path.
    Heading,
    /// One of many: picker rows, the statusline, git file lists,
    /// diagnostic locations.
    Row,
}

/// `(directory, name)`: the directory keeps its trailing `/` so the two
/// spans concatenate back into the path; a path with no `/` is all name.
pub(crate) fn split(path: &str) -> (&str, &str) {
    match path.rfind('/') {
        Some(at) => path.split_at(at + 1),
        None => ("", path),
    }
}

pub(crate) fn dir_style(t: &Tokens) -> Style {
    Style::default().fg(t.secondary)
}

pub(crate) fn name_style(role: Role, t: &Tokens) -> Style {
    let style = Style::default().fg(t.text);
    match role {
        Role::Heading => style.add_modifier(Modifier::BOLD),
        Role::Row => style,
    }
}

/// The two spans of a path, directory first, owning their text so a
/// row built as `'static` can hold them. The directory span is empty
/// for a bare name; it renders as nothing.
pub(crate) fn spans(path: &str, role: Role, t: &Tokens) -> [Span<'static>; 2] {
    let (dir, name) = split(path);
    [
        Span::styled(dir.to_owned(), dir_style(t)),
        Span::styled(name.to_owned(), name_style(role, t)),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use strop_core::theme::ThemeId;

    #[test]
    fn the_directory_keeps_its_slash_and_a_bare_name_has_none() {
        assert_eq!(
            split("crates/strop/src/lib.rs"),
            ("crates/strop/src/", "lib.rs")
        );
        assert_eq!(split("README.md"), ("", "README.md"));
        assert_eq!(split("…/render/card.rs"), ("…/render/", "card.rs"));
        assert_eq!(split("ssh://dev/etc/hosts"), ("ssh://dev/etc/", "hosts"));
        assert_eq!(split("plans/"), ("plans/", ""));
    }

    #[test]
    fn a_heading_is_bold_and_a_row_is_not_and_both_keep_the_inks() {
        let t = Tokens::new(ThemeId::Hone);
        let [dir, name] = spans("docs/remote.md", Role::Heading, &t);
        assert_eq!(dir.content, "docs/");
        assert_eq!(dir.style.fg, Some(t.secondary));
        assert!(!dir.style.add_modifier.contains(Modifier::BOLD));
        assert_eq!(name.content, "remote.md");
        assert_eq!(name.style.fg, Some(t.text));
        assert!(name.style.add_modifier.contains(Modifier::BOLD));
        let [_, row] = spans("docs/remote.md", Role::Row, &t);
        assert_eq!(row.style.fg, Some(t.text));
        assert!(!row.style.add_modifier.contains(Modifier::BOLD));
    }
}
