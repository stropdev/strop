//! Rendering: the render tree's root. Panes and diff decoration live
//! in `buffer`/`diff` (0010 §3); `statusline` owns the modeline and
//! `echo` the message row above it. Colours come from one per-frame
//! `Tokens` (`style.rs`, 0088 §2) and chrome from one primitive module
//! (`card.rs`); this module owns the welcome card and the cursor.
//! Overlay precedence (0001 §5.8 subset): search/incsearch < operator
//! preview < cursor. One accent colour (0001 §4).

use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Paragraph;
use ratatui::Frame;

use crate::editor::{Editor, Mode};

mod blame_card;
mod buffer;
pub(crate) mod card;
mod clip;
mod cmd_card;
mod completion;
pub(crate) mod diff;
mod echo;
#[cfg(test)]
mod echo_tests;
mod field;
pub(crate) mod frame_capture;
#[cfg(test)]
mod frame_tests;
#[cfg(test)]
mod golden;
#[cfg(test)]
mod golden_tests;
mod help;
mod hover_card;
#[path = "picker/mod.rs"]
mod picker;
mod statusline;
pub(crate) mod style;
mod terminal;
#[cfg(test)]
mod terminal_tests;
#[cfg(test)]
mod theme_tests;
#[cfg(test)]
mod welcome_tests;
mod which_key;

pub(crate) use style::Tokens;

/// The tokens a fresh `Editor` renders with (its default palette) —
/// the one place render tests take colours from.
#[cfg(test)]
pub(crate) fn tokens() -> Tokens {
    Tokens::new(strop_core::theme::ThemeId::default())
}

/// Toolkit rect → engine cell geometry (AR01: geometry enters the engine
/// only through this conversion; cells, never pixels).
pub(crate) fn to_cells(rect: Rect) -> strop_engine::editor::prepare::CellRect {
    strop_engine::editor::prepare::CellRect {
        x: rect.x,
        y: rect.y,
        width: rect.width,
        height: rect.height,
    }
}

/// Engine cell geometry → toolkit rect.
pub(crate) fn from_cells(cells: strop_engine::editor::prepare::CellRect) -> Rect {
    Rect {
        x: cells.x,
        y: cells.y,
        width: cells.width,
        height: cells.height,
    }
}

/// AR01: paint is a readonly presentation query. The admitted engine
/// update (`Editor::prepare_view`, engine-side) owns hunk refresh,
/// visible-analysis/preview admission and viewport adjustment; the frame
/// boundary (`frame_capture::draw`, or the test `paint` helper) runs it
/// before this function. Paint may update its own frontend caches (the
/// ratatui cell buffer), never engine authority.
pub fn render(editor: &Editor, frame: &mut Frame) {
    let area = frame.area();
    if editor.panes().is_empty() {
        return;
    }
    // A frontend that skipped preparation paints a stale-revision view:
    // loud in tests, and never an engine mutation in any build.
    debug_assert_eq!(
        editor.view_epoch().geometry,
        strop_engine::editor::prepare::ViewGeometry {
            columns: area.width,
            rows: area.height,
        },
        "paint without prepare_view for this geometry"
    );
    // One style table per frame (0088 §2): built here, borrowed below.
    let t = Tokens::new(editor.theme());

    // pane geometry (heights feed the vertical viewport, widths the
    // horizontal origin) is decided per pane inside render_panes —
    // the full-area numbers were wrong in splits (0031 R6)
    let pane_area = buffer::render_panes(editor, &t, frame, area);
    // The welcome card is part of the pane layer: every card, the echo
    // row and the statusline paint over it, never the reverse.
    render_welcome(editor, &t, frame);
    echo::render(editor, &t, frame, area, pane_area);
    statusline::render(editor, &t, frame, area);
    cmd_card::render_cmd_card(editor, &t, frame);
    let caret = if !cmd_card_active(editor) && !editor.terminal_input_active() {
        place_cursor(editor, &t, frame, pane_area)
    } else {
        None
    };
    picker::render_picker(editor, &t, frame);
    blame_card::render_blame_card(editor, &t, frame);
    hover_card::render_hover_card(editor, &t, frame);
    completion::render(editor, &t, frame, pane_area, caret);
    which_key::render_which_key(editor, &t, frame);
    // Every widget can display external text (paths, LSP messages, shell output).
    // Enforce the printable-cell invariant at the final emission boundary too.
    for cell in &mut frame.buffer_mut().content {
        if cell.symbol().chars().any(char::is_control) {
            cell.set_symbol("\u{fffd}");
        }
    }
}

/// Test-frontend frame boundary: the admitted preparation for this
/// frame's geometry, then the readonly paint — the same two phases the
/// real frontends run through `frame_capture::draw`.
#[cfg(test)]
pub(crate) fn paint(editor: &mut Editor, frame: &mut Frame) {
    let area = frame.area();
    editor.prepare_view(strop_engine::editor::prepare::ViewGeometry {
        columns: area.width,
        rows: area.height,
    });
    render(editor, frame);
}

fn place_cursor(editor: &Editor, t: &Tokens, frame: &mut Frame, area: Rect) -> Option<(u16, u16)> {
    // one projection shared with every painted caret: vertical top +
    // horizontal origin + fixed inset, checked, narrowed once
    let at = buffer::caret_position(
        editor,
        area,
        editor.current(),
        editor.head(),
        editor.view_top(),
        editor.view().hscroll,
    )?;
    // 0064 §2: while the fade-in runs, the Normal-mode block is
    // software-painted toward its final appearance and the native
    // cursor stays hidden; when the window closes the final frame is
    // exactly the unfaded cursor below. Presentation only.
    if matches!(editor.mode(), Mode::Normal)
        && !editor.picker_open()
        && editor.pending_sigil().is_none()
    {
        if let Some(progress) = editor.cursor_fade_progress() {
            let cell = &mut frame.buffer_mut()[at];
            let fg = cell.fg;
            cell.set_bg(style::fade_mix(t.base, t.text, progress));
            cell.set_fg(style::fade_mix(fg, t.base, progress));
            return Some(at);
        }
    }
    crate::render::frame_capture::place_cursor(frame, at);
    Some(at)
}

/// First-launch card: brand + the keys that matter. Only on an
/// empty scratch buffer — once you're editing, it never intrudes.
fn render_welcome(editor: &Editor, t: &Tokens, frame: &mut Frame) {
    if !matches!(editor.cur().source, crate::editor::DocumentSource::Scratch)
        || editor.buf().path.is_some()
        || editor.buf().len_bytes() > 0
        || editor.picker_open()
    {
        return;
    }
    let area = frame.area();
    let keys = [
        ("space", "everything"),
        ("space ?", "keybindings"),
        (":w", "save"),
    ];
    let keys_row: usize = keys
        .iter()
        .map(|(key, desc)| clip::width(key) + 2 + 1 + clip::width(desc) + 3)
        .sum();
    let tagline = "see the cut before you make it.";
    let note = "git signs paint the gutter · ci[ previews the cut";
    let content = keys_row.max(clip::width(note)).max(clip::width(tagline)) + 4;
    let (w, h) = (card::card_width(content, area.width), 9u16);
    if area.width < w + 4 || area.height < h + 4 {
        return;
    }
    let rect = Rect {
        x: (area.width - w) / 2,
        y: (area.height - h) / 3,
        width: w,
        height: h,
    };
    let inner = card::Card::new("").render(frame, rect, t);
    let mut chips = Vec::new();
    for (index, (key, desc)) in keys.iter().enumerate() {
        if index > 0 {
            chips.push(Span::raw("   "));
        }
        chips.push(card::keycap(key, t));
        chips.push(Span::styled(
            format!(" {desc}"),
            Style::default().fg(t.muted),
        ));
    }
    let lines = vec![
        Line::from(""),
        Line::from(Span::styled(
            "strop",
            Style::default().fg(t.text).add_modifier(Modifier::BOLD),
        )),
        Line::from(Span::styled(tagline, Style::default().fg(t.accent))),
        Line::from(""),
        Line::from(chips),
        Line::from(""),
        Line::from(Span::styled(note, Style::default().fg(t.muted))),
    ];
    frame.render_widget(Paragraph::new(lines), inner);
}

/// True when the floating command/search/pipe card owns the caret
/// (0031 R7: every sigil prompt — `:`, `/`, `?`, `|` — takes it).
pub(crate) fn cmd_card_active(editor: &Editor) -> bool {
    !editor.picker_open() && editor.pending_sigil().is_some()
}
