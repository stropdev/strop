use super::*;
use crate::editor::{events::AppEvent, trace::drive::Action};
use strop_core::frontend_input::Input;

fn terminal_command(chars: impl Iterator<Item = char>) -> bool {
    // Classify the private command family, not a second Ex dispatcher. Even
    // incomplete/misspelled terminal subcommands keep their arguments private.
    let mut chars = chars.skip_while(|ch| *ch == ':' || ch.is_ascii_whitespace());
    "terminal"
        .chars()
        .all(|expected| chars.next() == Some(expected))
}
impl Editor {
    pub fn private_terminal_document(&self, document: DocumentId) -> bool {
        !self.terminals.capture && self.terminal_document(document).is_some()
    }
    pub fn private_terminal_view(&self) -> bool {
        !self.terminals.capture
            && self
                .panes
                .iter()
                .any(|pane| self.terminal_document(pane.doc).is_some())
    }
    pub(crate) fn private_terminal_prompt(&self) -> bool {
        !self.terminals.capture && terminal_command(self.pending.text().chars())
    }
    pub(crate) fn private_terminal_action(&self, action: &Action) -> bool {
        if self.terminals.capture {
            return false;
        }
        if self
            .panes
            .get(self.active_pane)
            .is_some_and(|pane| self.terminal_document(pane.doc).is_some())
            || self.private_terminal_prompt()
        {
            return true;
        }
        match action {
            Action::Event(AppEvent::TerminalUpdate(_)) => true,
            Action::Frame { .. } => self.private_terminal_view(),
            Action::Event(
                AppEvent::Paste(text) | AppEvent::Input(Input::Paste(text) | Input::Text(text)),
            ) => {
                let current = self.pending.text();
                if !current.starts_with(':') {
                    return false;
                }
                let cursor = self.pending.cursor();
                let Some((before, after)) = current.split_at_checked(cursor) else {
                    return true;
                };
                terminal_command(before.chars().chain(text.chars()).chain(after.chars()))
            }
            _ => false,
        }
    }
    pub fn set_terminal_capture(&mut self, enabled: bool) {
        self.terminals.capture = enabled;
    }
    pub fn terminal_capture_enabled(&self) -> bool {
        self.terminals.capture
    }
    pub fn set_terminal_keyboard(&mut self, flags: u8) -> Result<(), String> {
        if flags & !strop_terminal::model::SUPPORTED_KEYBOARD_FLAGS != 0 {
            return Err("unsupported frontend keyboard profile".into());
        }
        self.terminals.keyboard = flags;
        Ok(())
    }
    pub fn terminal_keyboard_flags(&self) -> u8 {
        self.terminals.keyboard
    }
}
