//! The grammar footer: what the pending keys will do (0001 §4 "see the
//! cut before you make it"). Precedence is unchanged from the 0.15
//! modeline: a live preview (or its query error) beats the free pending
//! line, which beats the walker's structural state. Messages are not
//! grammar state and never appear here (see `render/echo.rs`).

use crate::editor::Editor;

use crate::render::cmd_card_active;

pub(super) fn spec(editor: &Editor) -> String {
    if editor.terminal_phase(editor.current()).is_some() {
        return if editor.pending().is_active() {
            editor.pending().text().to_owned()
        } else {
            String::new()
        };
    }
    if let Some(state) = editor
        .pending_field_state()
        .filter(|state| state.mode == strop_engine::editor::PendingFieldMode::Normal)
    {
        let owner = state.outer_operator.map_or_else(
            || "prompt pending".to_owned(),
            |operator| format!("{operator:?} pending").to_lowercase(),
        );
        return format!(
            "{owner} · field NORMAL · Esc cancels · {}",
            editor.pending().text().trim_end_matches('\r')
        );
    }
    if let Some(state) = editor.substitute_preview() {
        use strop_engine::editor::SubstitutePreviewState;
        match state {
            SubstitutePreviewState::Ready(_) => {}
            SubstitutePreviewState::Resolving => return "substitute resolving…".into(),
            SubstitutePreviewState::NoTarget => return "no substitution matches".into(),
            SubstitutePreviewState::Incomplete(message)
            | SubstitutePreviewState::Invalid(message)
            | SubstitutePreviewState::Unavailable(message) => return message,
        }
    }
    let preview = match editor.preview() {
        Ok(Some((_, spec))) => Some(spec),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    preview
        .or_else(|| {
            editor
                .preview_is_resolving()
                .then(|| "preview resolving…".into())
        })
        .or_else(|| {
            (editor.pending().is_active() && !cmd_card_active(editor))
                .then(|| editor.pending().text().trim_end_matches('\r').to_owned())
        })
        .or_else(|| {
            (!editor.walker().prefix_display().is_empty() || !editor.walker().state.empty())
                .then(|| editor.walker().display())
        })
        .unwrap_or_default()
}

/// Terminal panes: the keys that move you between the child and the
/// editor, as whole chords (rule 10), plus attention badges for pinned
/// output and a held paste.
pub(super) fn terminal_hints(editor: &Editor) -> (Vec<(String, &'static str)>, Vec<&'static str>) {
    let Some(phase) = editor.terminal_phase(editor.current()) else {
        return (Vec::new(), Vec::new());
    };
    let input = editor.terminal_input_active();
    let live = phase.live();
    let mut hints = Vec::new();
    let mut attention = Vec::new();
    if editor.terminal_prefix_hint().is_some() {
        // the escape chord pends: name its completion and the fallthrough
        hints.push(("ctrl-n".to_owned(), "inspect"));
        hints.push(("any other key".to_owned(), "sends to child"));
        return (hints, attention);
    }
    if input {
        hints.push(("F2".to_owned(), "inspect"));
        hints.push(("ctrl-\\ ctrl-n".to_owned(), "inspect"));
    } else if live {
        hints.push(("i".to_owned(), "input"));
        if editor.terminal_has_new_output(editor.current()) {
            attention.push("out+");
            hints.push(("space a".to_owned(), "refresh"));
        }
    } else {
        attention.push(phase_label(phase));
        hints.push(("space a".to_owned(), "new terminal"));
    }
    if editor.terminal_paste_held(editor.current()) {
        attention.push("paste+");
        hints.push(("space a".to_owned(), "review paste"));
    }
    (hints, attention)
}

fn phase_label(phase: &strop_terminal::model::Phase) -> &'static str {
    use strop_terminal::model::Phase;
    match phase {
        Phase::Starting => "starting",
        Phase::Running => "running",
        Phase::Closing => "closing",
        Phase::Exited { .. } => "exited",
        Phase::Failed(_) => "failed",
    }
}
