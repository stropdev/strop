//! The command line's menu (plan 0110 §3.3): a card standing on the
//! line, its name column under the first character being completed.
//! Columns by what it lists — commands: name, shortest abbreviation,
//! description, the Normal key that does the same; paths: names;
//! buffers: number, name, `●`; words, themes, topics: name and a line.
//! On a narrow terminal the key goes first, then the description, then
//! the abbreviation. The rows come ranked from the engine; this only
//! lays them out.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;
use strop_engine::editor::{MenuKind, MenuRow};

use super::super::card::{card_width, row_marker, Card, Hint};
use super::super::clip::{clip_end, width};
use super::super::style::Tokens;
use crate::editor::Editor;

/// The most rows the menu shows; the list scrolls past them.
const ROWS: usize = 10;

/// Which columns a row draws, and how wide each is.
struct Columns {
    name: usize,
    abbrev: usize,
    doc: usize,
    key: usize,
}

impl Columns {
    fn plan(kind: MenuKind, rows: &[MenuRow], room: usize) -> Self {
        let widest = |cell: fn(&MenuRow) -> usize| rows.iter().map(cell).max().unwrap_or(0);
        let mut columns = Self {
            name: widest(|row| width(row.label())).max(4),
            abbrev: match kind {
                MenuKind::Commands | MenuKind::Buffers => widest(|row| width(row.abbrev())),
                _ => 0,
            },
            doc: widest(|row| width(row.doc())),
            key: widest(|row| row.key().map_or(0, width)),
        };
        // narrow: the key goes first, then the description, then the
        // abbreviation; the name is cut last
        for drop in 0..3 {
            if columns.total() <= room {
                break;
            }
            match drop {
                0 => columns.key = 0,
                1 => columns.doc = 0,
                _ => columns.abbrev = 0,
            }
        }
        columns.name = columns
            .name
            .min(room.saturating_sub(columns.total() - columns.name));
        columns
    }

    /// Cells a row needs: marker and gap, then each present column with
    /// two cells before it, and one cell of margin.
    fn total(&self) -> usize {
        let gap = |cells: usize| if cells == 0 { 0 } else { cells + 2 };
        2 + self.name + gap(self.abbrev) + gap(self.doc) + gap(self.key) + 1
    }
}

/// Draw the menu above the bottom row. `body_x` is the cell where the
/// line's text (after the sigil) starts.
pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect, body_x: usize) {
    let Some(menu) = editor.cmdline_menu() else {
        return;
    };
    let rows = menu.rows();
    // the card stands on the line: never over it, never off the top
    let room = usize::from(area.height.saturating_sub(1));
    let visible = rows.len().min(ROWS).min(room.saturating_sub(2));
    if visible == 0 || area.width < 16 {
        return;
    }
    let height = u16::try_from(visible + 2).unwrap_or(u16::MAX);
    let columns = Columns::plan(menu.kind(), rows, usize::from(area.width).saturating_sub(2));
    let card_w = card_width(columns.total() + 2, area.width).min(area.width);
    // the name column starts under the first character being completed
    let body = editor.cmdline().text();
    let body = body.get(1..).unwrap_or("");
    let anchor = body_x + width(body.get(..menu.anchor()).unwrap_or(body));
    let x = u16::try_from(anchor.saturating_sub(3))
        .unwrap_or(u16::MAX)
        .min(area.right() - card_w);
    let card = Rect::new(x, area.bottom() - 1 - height, card_w, height);
    let count = super::super::menu::count(menu.selected(), rows.len());
    let note = match menu.note() {
        Some(note) => format!("{note} · {count}"),
        None => count,
    };
    let mut hints = vec![
        Hint {
            key: "tab",
            desc: "next",
        },
        Hint {
            key: "s-tab",
            desc: "previous",
        },
    ];
    if menu.selected().is_some() {
        hints.push(Hint {
            key: "ctrl-y",
            desc: "accept",
        });
    }
    hints.push(Hint {
        key: "ctrl-e",
        desc: "dismiss",
    });
    let inner = Card::new(menu.title())
        .note(Some(&note))
        .hints(&hints)
        .render(frame, card, t);
    // rows own the full inner width so the selected band reaches the border
    let inner = Rect {
        x: inner.x.saturating_sub(1),
        width: inner.width.saturating_add(2),
        ..inner
    };
    let first = super::super::menu::first_visible(menu.selected(), rows.len(), visible);
    for offset in 0..visible {
        let index = first + offset;
        let Some(row) = rows.get(index) else { break };
        let area = Rect::new(inner.x, inner.y + offset as u16, inner.width, 1);
        draw_row(
            t,
            menu.kind(),
            row,
            &columns,
            menu.selected() == Some(index),
            frame,
            area,
        );
    }
}

fn draw_row(
    t: &Tokens,
    kind: MenuKind,
    row: &MenuRow,
    columns: &Columns,
    selected: bool,
    frame: &mut Frame,
    area: Rect,
) {
    let background = if selected { t.raised } else { t.surface };
    let base = Style::default().fg(t.text).bg(background);
    frame.buffer_mut().set_style(area, base);
    let mut spans = vec![
        row_marker(selected, t).style(base.fg(t.accent)),
        Span::styled(" ", base),
    ];
    if kind == MenuKind::Buffers && columns.abbrev > 0 {
        spans.push(Span::styled(
            format!("{:>width$}  ", row.abbrev(), width = columns.abbrev),
            base.fg(t.muted),
        ));
    }
    // the name, its matched letters in accent + bold (0001 §4)
    let name = clip_end(row.label(), columns.name);
    let matched = base.fg(t.accent).add_modifier(Modifier::BOLD);
    let mut run = String::new();
    let mut run_matched = false;
    for (index, c) in name.chars().enumerate() {
        if row.matched(index) != run_matched && !run.is_empty() {
            let style = if run_matched { matched } else { base };
            spans.push(Span::styled(std::mem::take(&mut run), style));
        }
        run_matched = row.matched(index);
        run.push(c);
    }
    if !run.is_empty() {
        spans.push(Span::styled(run, if run_matched { matched } else { base }));
    }
    spans.push(Span::styled(
        " ".repeat(columns.name.saturating_sub(width(&name))),
        base,
    ));
    if kind == MenuKind::Buffers && row.modified() {
        spans.push(Span::styled("  ●", base.fg(t.warning)));
    }
    if kind != MenuKind::Buffers && columns.abbrev > 0 {
        spans.push(Span::styled(
            format!("  {:<width$}", row.abbrev(), width = columns.abbrev),
            base.fg(t.muted),
        ));
    }
    let doc_room = usize::from(area.width)
        .saturating_sub(spans.iter().map(Span::width).sum::<usize>())
        .saturating_sub(if columns.key > 0 { columns.key + 3 } else { 1 });
    if columns.doc > 0 && doc_room > 2 {
        spans.push(Span::styled(
            format!("  {}", clip_end(row.doc(), doc_room - 2)),
            base.fg(t.secondary),
        ));
    }
    if let Some(key) = row.key().filter(|_| columns.key > 0) {
        let used: usize = spans.iter().map(Span::width).sum();
        let pad = usize::from(area.width).saturating_sub(used + width(key) + 2);
        spans.push(Span::styled(" ".repeat(pad), base));
        spans.push(Span::styled(key.to_owned(), base.fg(t.muted)));
    }
    frame.render_widget(Paragraph::new(Line::from(spans)).style(base), area);
}
