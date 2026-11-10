use super::*;
use strop_terminal::model::{Palette, Style};

impl Editor {
    /// Cell styles of the installed frame: explicit colors resolve through
    /// that frame's effective palette, OSC overrides included.
    pub fn terminal_style_at(
        &self,
        document: DocumentId,
        byte: usize,
    ) -> Option<(Style, &Palette)> {
        let frame = self.terminal_frame(document)?;
        Some((frame.cell_at_byte(byte)?.style, &frame.palette))
    }
    pub fn terminal_title(&self, document: DocumentId) -> Option<&str> {
        let session = self.terminal_document(document)?.session;
        self.terminals.entries.get(&session)?.title.as_deref()
    }
    /// The active terminal chord's complete escape grammar, while it pends.
    pub fn terminal_prefix_hint(&self) -> Option<&'static str> {
        let session = self.terminal_document(self.current())?.session;
        let _prefix = self
            .terminals
            .prefix
            .as_ref()
            .filter(|prefix| prefix.session == session)?;
        Some("ctrl-\\ ctrl-n → NORMAL · any other key sends to child")
    }
    pub fn terminal_status(&self, document: DocumentId) -> Option<serde_json::Value> {
        let source = self.terminal_document(document)?;
        let entry = self.terminals.entries.get(&source.session)?;
        let (phase, code, signal) = match entry.phase {
            Phase::Starting => ("starting", None, None),
            Phase::Running => ("running", None, None),
            Phase::Closing => ("closing", None, None),
            Phase::Exited { code, signal } => ("exited", code, signal),
            Phase::Failed(_) => ("failed", None, None),
        };
        Some(serde_json::json!({
            "session":source.session, "phase":phase, "exit_code":code, "signal":signal,
            "namespace":entry.launch.filesystem.label(),
            "launch":&entry.launch,
            "launch_location":entry.launch.label(),
            "launch_directory":strop_workspace::directory::display_path(&entry.launch.path),
            "capture":self.terminals.capture,
            "keyboard_flags":entry.keyboard,
            "geometry":source.frame.as_ref().map(|frame| frame.geometry),
            "cursor":source.frame.as_ref().map(|frame| frame.cursor),
            "revision":source.frame.as_ref().map(|frame| frame.revision),
        }))
    }
}
