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
        if let Some(prefix) = editor.terminal_prefix_hint() {
            return prefix.to_owned();
        }
        if editor.pending().is_active() {
            return editor.pending().text().to_owned();
        }
        let new_output = !editor.terminal_input_active()
            && editor.terminal_has_new_output(editor.current())
            && phase.live();
        if !editor.message().is_empty() {
            return format!(
                "{}{}",
                if new_output {
                    "new output · :terminal-refresh · "
                } else {
                    ""
                },
                editor.message()
            );
        }
        let phase = match phase {
            strop_terminal::model::Phase::Starting => "starting",
            strop_terminal::model::Phase::Running => "running",
            strop_terminal::model::Phase::Closing => "closing",
            strop_terminal::model::Phase::Exited { .. } => "exited",
            strop_terminal::model::Phase::Failed(_) => "failed",
        };
        // 0065 S2: the transient names the escape out of every terminal
        // state — the escape is the discovery surface, not a secret.
        let hint = if editor.terminal_input_active() {
            "F2 normal · ctrl-\\ ctrl-n alternative"
        } else if new_output {
            "new output · :terminal-refresh · i follows"
        } else if phase == "running" || phase == "starting" || phase == "closing" {
            "snapshot · i/I/a/A input"
        } else {
            ""
        };
        let capture = if editor.terminal_capture_enabled() {
            "input + output RECORDED"
        } else {
            "input private"
        };
        return if hint.is_empty() {
            format!("{phase} · {capture}")
        } else {
            format!("{phase} · {hint} · {capture}")
        };
    }
    let preview = match editor.preview() {
        Ok(Some((_, spec))) => Some(spec),
        Ok(None) => None,
        Err(error) => Some(error),
    };
    preview
        .or_else(|| {
            (editor.pending().is_active() && !cmd_card_active(editor))
                .then(|| editor.pending().text().trim_end_matches('\r').to_owned())
        })
        .or_else(|| {
            (!editor.walker().prefix_display().is_empty() || !editor.walker().state.empty())
                .then(|| editor.walker().display())
        })
        .or_else(|| editor.io_status().map(str::to_owned))
        .or_else(|| (!editor.message().is_empty()).then(|| editor.message().to_owned()))
        .or_else(|| editor.directory_visibility_summary().map(str::to_owned))
        .unwrap_or_default()
}
