//! The recovery offer (0090 §3, vim's `ATTENTION` baseline): a file just
//! opened has an unsaved draft an earlier session left. The card names the
//! draft and offers the four answers as keycaps; it owns input until one
//! is chosen.

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::{Editor, InputOwner};

use super::card::{card_width, keycap, Card, Hint};
use super::clip::{clip_start, width};
use super::style::Tokens;

const CHOICES: [(&str, &str); 4] = [
    ("r", "recover the draft into this buffer"),
    ("o", "open the file read-only"),
    ("d", "delete the draft"),
    ("e", "edit the file anyway"),
];

const HINTS: &[Hint<'static>] = &[Hint {
    key: "esc",
    desc: "edit anyway",
}];

pub(super) fn render(editor: &Editor, t: &Tokens, frame: &mut Frame) {
    if editor.input_owner() != InputOwner::RecoveryOffer {
        return;
    }
    let Some(offer) = editor.recovery_offer() else {
        return;
    };
    let area = frame.area();
    let summary = "an earlier session left an unsaved draft";
    let note = format!("r{} · {} B", offer.revision, offer.bytes);
    let changed = "the file changed on disk since the draft";
    let choices = CHOICES
        .iter()
        .map(|(key, desc)| width(key) + 4 + width(desc))
        .max()
        .unwrap_or(0);
    let content = width(summary)
        .max(width(&offer.path))
        .max(if offer.source_changed {
            width(changed) + 2
        } else {
            0
        })
        .max(choices)
        + 4;
    let card_w = card_width(content, area.width);
    let body = 3 + usize::from(offer.source_changed) + CHOICES.len();
    let height = (body as u16 + 2).min(area.height.saturating_sub(2));
    if card_w < 12 || height < 5 {
        return;
    }
    let rect = Rect {
        x: area.width.saturating_sub(card_w) / 2,
        y: area.height.saturating_sub(height) / 3,
        width: card_w,
        height,
    };
    let inner = Card::new("recovery draft")
        .note(Some(&note))
        .hints(HINTS)
        .render(frame, rect, t);
    let cells = usize::from(inner.width);
    let mut lines = vec![
        Line::from(Span::styled(
            clip_start(&offer.path, cells).into_owned(),
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(
            super::clip::clip_end(summary, cells).into_owned(),
            Style::default().fg(t.secondary),
        )),
    ];
    if offer.source_changed {
        lines.push(Line::from(vec![
            Span::styled("▲ ", Style::default().fg(t.warning)),
            Span::styled(
                super::clip::clip_end(changed, cells.saturating_sub(2)).into_owned(),
                Style::default().fg(t.warning),
            ),
        ]));
    }
    lines.push(Line::from(""));
    for (key, desc) in CHOICES {
        lines.push(Line::from(vec![
            keycap(key, t),
            Span::styled(format!("  {desc}"), Style::default().fg(t.text)),
        ]));
    }
    frame.render_widget(Paragraph::new(lines), inner);
}
