use std::fmt::Write;

use super::Editor;

impl Editor {
    pub(super) fn explain_terminal(&self, text: &mut String) {
        let document = self.current();
        let Some(launch) = self.terminal_launch_location(document) else {
            return;
        };
        let Some(status) = self.terminal_status(document) else {
            return;
        };
        text.push_str("\n[terminal]\n");
        let _ = writeln!(text, "  launch    {}", launch.label());
        text.push_str("  directory launch selection only; shell cd/title cannot retarget it\n");
        let _ = writeln!(
            text,
            "  phase     {}",
            status
                .get("phase")
                .and_then(|phase| phase.as_str())
                .unwrap_or("unknown")
        );
        let _ = writeln!(
            text,
            "  paste     {}",
            if status
                .get("paste_held")
                .and_then(|held| held.as_bool())
                .unwrap_or(false)
            {
                "held for explicit ticket-bound decision"
            } else {
                "none held"
            }
        );
        if self.terminal_has_new_output(document) {
            text.push_str("  snapshot  pinned; newer output is retained for explicit refresh\n");
        }
    }
}
