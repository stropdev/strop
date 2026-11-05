//! Transient status selection and its load-bearing precedence.

use crate::editor::Editor;

use crate::render::cmd_card_active;

/// The transient right-side text. Precedence is unchanged from the
/// 0.15 modeline: a live preview (or its query error) beats the free
/// pending line, which beats the walker's structural state, which
/// beats save/load activity, which beats the last message. Nothing is
/// filtered here — whatever wins is what the user sees.
pub(super) fn transient(editor: &Editor) -> String {
    if let Some(phase) = editor.terminal_phase(editor.current()) {
        if editor.pending().is_active() {
            return editor.pending().text().to_owned();
        }
        let input = editor.terminal_input_active();
        let live = phase.live();
        let new_output = !input && editor.terminal_has_new_output(editor.current()) && live;
        let paste = editor.terminal_paste_held(editor.current());
        let phase_label = match phase {
            strop_terminal::model::Phase::Starting => "starting",
            strop_terminal::model::Phase::Running => "running",
            strop_terminal::model::Phase::Closing => "closing",
            strop_terminal::model::Phase::Exited { .. } => "exited",
            strop_terminal::model::Phase::Failed(_) => "failed",
        };
        let mut essential = if input {
            let mut cue = "F2 inspect".to_owned();
            if paste {
                cue.push_str(" paste+");
            }
            cue.push_str(" · ctrl-\\ ctrl-n alternative · terminal input");
            cue
        } else if live {
            let mut cue = "i input".to_owned();
            if new_output {
                cue.push_str(" out+");
            }
            if paste {
                cue.push_str(" paste+");
            }
            cue
        } else {
            format!("Space a New terminal · {phase_label} · snapshot")
        };
        if let Some(prefix) = editor.terminal_prefix_hint() {
            essential = format!("F2 inspect · {prefix}");
        }
        let mut optional = Vec::new();
        if new_output {
            optional.push("Space a Refresh".to_owned());
        }
        if paste {
            optional.push("Space a Review/Send/Discard".to_owned());
        }
        if let Some(record) = editor.presented_feedback().filter(|record| {
            matches!(
                record.owner(),
                strop_engine::editor::feedback::FeedbackOwner::Terminal { session, .. }
                    if editor.terminal_document(editor.current()).is_some_and(|terminal| terminal.session == *session)
            )
        }) {
            optional.push(record.summary().to_owned());
        }
        if !editor.message().is_empty() {
            optional.push(editor.message().to_owned());
        }
        optional.push(phase_label.to_owned());
        optional.push(
            if editor.terminal_capture_enabled() {
                "input + output RECORDED"
            } else {
                "input private"
            }
            .to_owned(),
        );
        optional.extend(
            [(!input && live).then_some("i/I/a/A input")]
                .into_iter()
                .flatten()
                .map(str::to_owned),
        );
        return std::iter::once(essential)
            .chain(optional)
            .collect::<Vec<_>>()
            .join(" · ");
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
        .or_else(|| editor.io_status().map(str::to_owned))
        .or_else(|| {
            editor.presented_feedback().map(|record| {
                if editor.presented_feedback_has_actions() {
                    format!("{} · Space a actions", record.summary())
                } else {
                    format!("{} · :explain", record.summary())
                }
            })
        })
        .or_else(|| (!editor.message().is_empty()).then(|| editor.message().to_owned()))
        .or_else(|| editor.directory_visibility_summary().map(str::to_owned))
        .unwrap_or_default()
}
