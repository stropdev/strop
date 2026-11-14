//! Pane chrome (0103, plans/0104 §6): each pane's title rule and the
//! dividers between side-by-side panes, with the junctions where they meet.
//! Paint reads the prepared panes and dividers only (AR03): it never places
//! a window itself.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use strop_core::id::DocumentId;
use strop_engine::editor::prepare::{CellRect, PreparedView};

use super::fixed_spans;
use crate::editor::Editor;
use crate::render::style::Tokens;

/// The pane's identity in its top border: number, workspace-relative
/// name, dirty `●` (warning) and `[RO]`. `Document::label` is the shared
/// document-aware projection — local paths workspace-relative, outside
/// paths absolute, remote resources keeping their authority;
/// never the active-document modeline projection for an inactive pane.
#[allow(clippy::too_many_arguments)]
pub(super) fn render_title_rule(
    editor: &Editor,
    t: &Tokens,
    frame: &mut Frame,
    rect: Rect,
    index: usize,
    doc: DocumentId,
    active: bool,
) {
    let document = editor.doc(doc);
    let root = document
        .buf
        .path
        .as_deref()
        .and_then(|path| editor.workspace_root_path(&strop_workspace::Filesystem::Local, path))
        .unwrap_or(editor.cwd());
    let name = document.label(root);
    let w = usize::from(rect.width);
    // A terminal buffer is read-only by nature, not by flag: `[RO]` would
    // say nothing (0090 §4).
    let is_terminal = matches!(
        document.source,
        strop_engine::editor::DocumentSource::Terminal(_)
    );
    let readonly = document.buf.readonly && !is_terminal;
    // `─ N ` + name + flags + ` ──`: the flags and the rule are reserved
    // before the name clips, so focus, dirty and read-only survive narrow.
    let flags = format!(
        "{}{}",
        if document.buf.is_dirty() { " ●" } else { "" },
        if readonly { " [RO]" } else { "" }
    );
    let number = format!("{} ", index + 1);
    let fixed = 2 + crate::render::clip::width(&number) + crate::render::clip::width(&flags);
    let outside_workspace = document
        .buf
        .path
        .as_deref()
        .is_some_and(|path| path.is_absolute() && path.strip_prefix(root).is_err());
    let retain_authority = outside_workspace || document.remote_metadata().is_some() || is_terminal;
    // The title is the pane's heading (0088 §2 "File names"): authority
    // muted, directory secondary, name bold. An inactive pane is all
    // muted, the name still bold; a terminal's `term://` identity is one
    // name, clipped from its end.
    let label = if is_terminal {
        crate::render::file_name::Label::local(name)
    } else {
        crate::render::file_name::Label::parse(&name)
    };
    // the trailing ` ──` is reserved only while the whole name fits; a
    // tight pane spends every cell on identity instead
    let closes = fixed + label.width() + 3 <= w;
    let budget = w.saturating_sub(fixed + if closes { 3 } else { 0 });
    let label = if is_terminal {
        crate::render::file_name::Label::local(crate::render::clip::clip_end(&label.path, budget))
    } else if retain_authority {
        label.clipped(budget)
    } else {
        crate::render::file_name::Label::local(crate::render::clip::path_suffix(
            &label.path,
            budget,
        ))
    };
    let (dir, leaf) = if is_terminal {
        ("", label.path.as_str())
    } else {
        crate::render::file_name::split(&label.path)
    };
    let border = Style::default().fg(if active { t.muted } else { t.faint });
    let ink = Style::default().fg(if active { t.text } else { t.muted });
    let (dir_ink, leaf_ink) = if active {
        (
            crate::render::file_name::dir_style(t),
            crate::render::file_name::name_style(crate::render::file_name::Role::Heading, t),
        )
    } else {
        (ink, ink.add_modifier(Modifier::BOLD))
    };
    let mut spans = vec![Span::styled("─ ", border), Span::styled(number, ink)];
    if let Some(authority) = &label.authority {
        spans.push(Span::styled(
            format!("{authority} "),
            if active {
                crate::render::file_name::authority_style(t)
            } else {
                ink
            },
        ));
    }
    spans.push(Span::styled(dir.to_owned(), dir_ink));
    spans.push(Span::styled(leaf.to_owned(), leaf_ink));
    if document.buf.is_dirty() {
        spans.push(Span::styled(" ●", Style::default().fg(t.warning)));
    }
    if readonly {
        spans.push(Span::styled(" [RO]", Style::default().fg(t.warning)));
    }
    if closes {
        let used: usize = spans.iter().map(|span| span.width()).sum();
        let rule = w.saturating_sub(used);
        if rule > 0 {
            spans.push(Span::styled(
                format!(" {}", "─".repeat(rule.saturating_sub(1))),
                border,
            ));
        }
    }
    let spans = fixed_spans(spans, w, document.indent.width.max(1));
    frame.render_widget(
        Paragraph::new(Line::from(spans)).style(Style::default().bg(t.base)),
        Rect {
            y: rect.y.saturating_sub(1),
            height: 1,
            width: rect.width,
            x: rect.x,
        },
    );
}

/// The divider glyph at one cell (plans/0104 §6 rule 3): whether a title
/// rule ends against it on the `left`, starts after it on the `right`, and
/// whether the divider continues from `up`.
pub(super) fn junction(left: bool, right: bool, up: bool) -> &'static str {
    match (left, right, up) {
        (false, false, _) => "│",
        (_, _, false) => "┬",
        (true, true, true) => "┼",
        (false, true, true) => "├",
        (true, false, true) => "┤",
    }
}

/// Every divider, `fg.faint` (it belongs to neither pane), with its
/// junctions; then `┴` where a divider ends on a title rule below it — on a
/// rule cell only, a name is never overwritten, in the title's own ink.
pub(super) fn render_dividers(t: &Tokens, frame: &mut Frame, area: Rect, prepared: &PreparedView) {
    let modeline = area.y + area.height.saturating_sub(1);
    let titled: Vec<CellRect> = prepared
        .panes
        .iter()
        .map(|pane| pane.rect)
        .filter(|rect| prepared.panes.len() > 1 && rect.width > 0 && rect.height > 0)
        .collect();
    let in_divider = |x: u16, y: u16| {
        prepared
            .dividers
            .iter()
            .any(|d| d.x == x && (d.y..d.y + d.height).contains(&y))
    };
    for divider in &prepared.dividers {
        let x = divider.x;
        for y in divider.y..(divider.y + divider.height).min(modeline) {
            let left = titled
                .iter()
                .any(|r| r.y.checked_sub(1) == Some(y) && r.x + r.width == x);
            let right = titled
                .iter()
                .any(|r| r.y.checked_sub(1) == Some(y) && r.x == x + 1);
            let up = y > divider.y
                || prepared
                    .dividers
                    .iter()
                    .any(|other| other.x == x && other.y + other.height == y);
            let cell = &mut frame.buffer_mut()[(x, y)];
            cell.set_symbol(junction(left, right, up));
            cell.set_fg(t.faint);
            cell.set_bg(t.base);
        }
        let below = divider.y + divider.height;
        if below < modeline && !in_divider(x, below) {
            let cell = &mut frame.buffer_mut()[(x, below)];
            if cell.symbol() == "─" {
                cell.set_symbol("┴");
            }
        }
    }
}
