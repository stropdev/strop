//! The one file-name idiom (0088 §2 "File names", 2026-11-10): wherever a
//! path is shown, its directory is context in `fg.secondary` — ending in
//! its own `/` — and its basename is the identity in `fg.text`, bold when
//! the name is a heading (a group header, a pane title, a card's subject)
//! and regular inside a list row. A remote resource leads with its
//! authority, `ssh:host`, in `fg.muted` as its own segment — the
//! statusline's namespace chip — never spliced into the path. Clipping
//! always spends the directory first (`clip_start`), so the authority
//! and the name survive.

use std::path::Path;

use ratatui::style::{Modifier, Style};
use ratatui::text::Span;

use super::clip::{clip_start, width};
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

pub(crate) fn authority_style(t: &Tokens) -> Style {
    Style::default().fg(t.muted)
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

/// A file's display label: an optional authority (`ssh:host`) and the
/// path it applies to. Built from a structured location whenever the
/// caller has one; `parse` is for labels that already travelled as text
/// (a document's `label`, a picker item's text).
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Label {
    pub authority: Option<String>,
    pub path: String,
}

impl Label {
    pub(crate) fn local(path: impl Into<String>) -> Self {
        Self {
            authority: None,
            path: path.into(),
        }
    }

    /// Workspace-relative for a local path under `root`; a remote path
    /// keeps its full form behind its `ssh:host` authority.
    pub(crate) fn from_location(location: &strop_workspace::ResourceLocation, root: &Path) -> Self {
        match location.filesystem.endpoint() {
            Some(endpoint) => Self {
                authority: Some(format!("ssh:{}", endpoint.host())),
                path: strop_workspace::directory::display_path(&location.path),
            },
            None => {
                let relative = location.path.strip_prefix(root).unwrap_or(&location.path);
                Self::local(strop_workspace::directory::display_path(relative))
            }
        }
    }

    /// A canonical `ssh://…/path` label splits into authority and path;
    /// anything else is a local label as written.
    pub(crate) fn parse(text: &str) -> Self {
        match strop_workspace::RemoteFile::parse(text) {
            Ok(file) => Self {
                authority: Some(format!("ssh:{}", file.endpoint().host())),
                path: strop_workspace::directory::display_path(file.path()),
            },
            Err(_) => Self::local(text),
        }
    }

    /// Cells the label occupies: authority, one space, path.
    pub(crate) fn width(&self) -> usize {
        self.authority_width() + width(&self.path)
    }

    fn authority_width(&self) -> usize {
        self.authority.as_deref().map_or(0, |a| width(a) + 1)
    }

    /// Fit `cells`: the path yields from its start (parents nearest the
    /// file survive), the authority stays whole while anything is left
    /// for the name beside it. An absolute path keeps its leading `/` —
    /// the mark that it lies outside the workspace — ahead of the `…`.
    pub(crate) fn clipped(&self, cells: usize) -> Self {
        if self.width() <= cells {
            return self.clone();
        }
        let room = cells.saturating_sub(self.authority_width()).max(1);
        let path = match self.path.strip_prefix('/') {
            Some(rest) if room > 2 => format!("/{}", clip_start(rest, room - 1)),
            _ => clip_start(&self.path, room).into_owned(),
        };
        Self {
            authority: self.authority.clone(),
            path,
        }
    }

    /// `ssh:host dir/name` as spans: authority muted (with its trailing
    /// space), then the directory and name in the idiom's inks.
    pub(crate) fn spans(&self, role: Role, t: &Tokens) -> Vec<Span<'static>> {
        let mut out = Vec::with_capacity(3);
        if let Some(authority) = &self.authority {
            out.push(Span::styled(format!("{authority} "), authority_style(t)));
        }
        out.extend(spans(&self.path, role, t));
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use strop_core::theme::ThemeId;

    #[test]
    fn the_directory_keeps_its_slash_and_a_bare_name_has_none() {
        assert_eq!(
            split("crates/strop/src/lib.rs"),
            ("crates/strop/src/", "lib.rs")
        );
        assert_eq!(split("README.md"), ("", "README.md"));
        assert_eq!(split("…/render/card.rs"), ("…/render/", "card.rs"));
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

    #[test]
    fn a_remote_location_leads_with_its_authority_then_directory_then_name() {
        let t = Tokens::new(ThemeId::Vellum);
        let endpoint = strop_workspace::RemoteEndpoint::parse("ssh://dev@box:2299").unwrap();
        let location = strop_workspace::ResourceLocation::remote(
            endpoint,
            PathBuf::from("/home/dev/proj/notes.txt"),
        );
        let label = Label::from_location(&location, Path::new("/home/dev/proj"));
        assert_eq!(label.authority.as_deref(), Some("ssh:box"));
        assert_eq!(label.path, "/home/dev/proj/notes.txt");
        let spans = label.spans(Role::Heading, &t);
        let text: String = spans.iter().map(|s| s.content.as_ref()).collect();
        assert_eq!(text, "ssh:box /home/dev/proj/notes.txt");
        assert_eq!(spans[0].style.fg, Some(t.muted));
        assert_eq!(spans[1].content, "/home/dev/proj/");
        assert_eq!(spans[1].style.fg, Some(t.secondary));
        assert_eq!(spans[2].content, "notes.txt");
        assert!(spans[2].style.add_modifier.contains(Modifier::BOLD));
        // a canonical URI label parses to the same segments
        assert_eq!(
            Label::parse("ssh://dev@box:2299/home/dev/proj/notes.txt"),
            label
        );
        assert_eq!(
            Label::parse("crates/strop/src/lib.rs"),
            Label::local("crates/strop/src/lib.rs")
        );
    }

    #[test]
    fn a_local_location_under_the_root_is_workspace_relative() {
        let location = strop_workspace::ResourceLocation::local(PathBuf::from("/w/docs/remote.md"));
        assert_eq!(
            Label::from_location(&location, Path::new("/w")),
            Label::local("docs/remote.md")
        );
        assert_eq!(
            Label::from_location(&location, Path::new("/elsewhere")),
            Label::local("/w/docs/remote.md")
        );
    }

    #[test]
    fn clipping_spends_the_path_and_keeps_the_authority_whole() {
        let label = Label {
            authority: Some("ssh:box".into()),
            path: "/home/dev/proj/notes.txt".into(),
        };
        assert_eq!(label.width(), 8 + 24);
        let clipped = label.clipped(20);
        assert_eq!(clipped.authority.as_deref(), Some("ssh:box"));
        assert_eq!(clipped.path, "/…/notes.txt");
        assert_eq!(clipped.width(), 20);
        assert_eq!(label.clipped(40), label);
        let relative = Label::local("crates/strop/src/render/card.rs").clipped(12);
        assert!(relative.path.starts_with('…'), "{relative:?}");
        assert!(relative.path.ends_with("/card.rs"), "{relative:?}");
        assert_eq!(relative.width(), 12);
    }
}
