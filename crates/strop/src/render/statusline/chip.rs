//! The fixed-width mode chip (0088 §2): eight cells for every mode name,
//! so the path beside it never jumps. The colour is the mode token; the
//! label says which surface owns the keys.

use ratatui::style::Color;

use crate::editor::Editor;
use crate::render::style::Tokens;

/// Every chip is exactly this many cells wide.
pub(super) const WIDTH: usize = 8;

/// The chip text (padded to [`WIDTH`]) and its colour.
pub(super) fn chip(editor: &Editor, t: &Tokens) -> (String, Color) {
    let typing = |normal: bool| if normal { t.accent } else { t.mode_insert };
    let (label, color) = if let Some(glue) = editor.picker() {
        (
            picker_label(glue.picker.kind),
            typing(glue.picker.input_normal()),
        )
    } else if let Some(sigil) = editor.pending_sigil() {
        let label = match sigil {
            ':' => "COMMAND",
            '|' => "PIPE",
            _ => "SEARCH",
        };
        (label, typing(editor.input_normal()))
    } else if editor.terminal_input_active() {
        // TERMINAL is its own mode (0090 §4); NORMAL on a terminal buffer
        // is plain NORMAL.
        ("TERMINAL", t.mode_terminal)
    } else if editor.pending().is_active()
        || !editor.walker().prefix_display().is_empty()
        || !editor.walker().state.empty()
    {
        ("PENDING", t.mode_pending)
    } else {
        (editor.mode().chip(), t.mode(editor.mode()))
    };
    (pad(label), color)
}

/// Centre a label in the chip's cells; a label wider than the chip is
/// cut, never allowed to widen it.
fn pad(label: &str) -> String {
    let label: String = label.chars().take(WIDTH).collect();
    let free = WIDTH - label.chars().count();
    let left = free / 2;
    format!("{}{label}{}", " ".repeat(left), " ".repeat(free - left))
}

fn picker_label(kind: strop_picker::Kind) -> &'static str {
    use strop_picker::Kind;
    match kind {
        Kind::Files => "FILES",
        Kind::Buffers => "BUFFERS",
        Kind::Search => "SEARCH",
        Kind::Symbols | Kind::WorkspaceSymbols => "SYMBOLS",
        Kind::Diagnostics => "DIAG",
        Kind::Jumps => "JUMPS",
        Kind::Theme => "THEME",
        Kind::Branches => "BRANCHES",
        Kind::Locations => "LOCS",
        Kind::RemoteHosts | Kind::RemoteAddress => "REMOTE",
        Kind::TabSize | Kind::Actions => "PICKER",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_label_pads_to_the_chip_width() {
        for label in [
            "NORMAL", "INSERT", "VISUAL", "V-LINE", "V-BLOCK", "PENDING", "TERMINAL", "FILES",
            "SEARCH", "SYMBOLS", "BUFFERS", "COMMAND", "PIPE", "DIAG", "JUMPS", "THEME", "LOCS",
            "REMOTE", "PICKER", "",
        ] {
            assert_eq!(pad(label).chars().count(), WIDTH, "{label:?}");
            assert!(pad(label).contains(label));
        }
        assert_eq!(pad("NORMAL"), " NORMAL ");
        assert_eq!(pad("V-BLOCK"), "V-BLOCK ");
        assert_eq!(pad("TERMINAL"), "TERMINAL");
        assert_eq!(pad("TOO-LONG-LABEL"), "TOO-LONG");
    }
}
