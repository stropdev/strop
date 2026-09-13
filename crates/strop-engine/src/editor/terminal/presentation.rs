use super::*;
use strop_terminal::model::{Palette, Style};

impl Editor {
    /// Logical inspection retains normal editor layout while styling each byte
    /// from the same immutable terminal generation as the buffer projection.
    pub fn terminal_style_at(
        &self,
        document: DocumentId,
        byte: usize,
    ) -> Option<(Style, &Palette)> {
        let frame = self.terminal_document(document)?.frame.as_ref()?;
        Some((frame.cell_at_byte(byte)?.style, &frame.palette))
    }
    pub fn terminal_title(&self, document: DocumentId) -> Option<&str> {
        let session = self.terminal_document(document)?.session;
        self.terminals.entries.get(&session)?.title.as_deref()
    }
    pub fn terminal_has_new_output(&self, document: DocumentId) -> bool {
        let Some(source) = self.terminal_document(document) else {
            return false;
        };
        let Some(shown) = &source.frame else {
            return false;
        };
        self.terminals
            .entries
            .get(&source.session)
            .and_then(|entry| entry.live.as_ref())
            .is_some_and(|live| live.revision > shown.revision)
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
            "paste_held":entry.paste.is_some(), "capture":self.terminals.capture,
            "keyboard_flags":entry.keyboard,
            "geometry":entry.live.as_ref().map(|frame| frame.geometry),
            "cursor":entry.live.as_ref().map(|frame| frame.cursor),
            "snapshot":source.frame.as_ref().map(|frame| frame.revision),
        }))
    }
}
