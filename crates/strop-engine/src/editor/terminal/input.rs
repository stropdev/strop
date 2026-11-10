use super::*;
use crate::editor::{InputOwner, Mode};
use strop_core::frontend_input::{Input, KeyCode, KeyEvent, KeyKind};

/// `letter` with Control held, or the C0 byte `raw` a legacy decoder
/// delivers for it. Crossterm's legacy decoder reports the 0x1c byte of
/// Ctrl-\ as Ctrl-4 (vim documents the two as the same key), so that
/// spelling is the same chord — not a second one.
fn control(key: KeyEvent, letter: char, raw: char) -> bool {
    !key.modifiers.alt
        && !key.modifiers.super_key
        && !key.modifiers.hyper
        && !key.modifiers.meta
        && ((matches!(key.code, KeyCode::Char(ch) if ch.to_ascii_lowercase() == letter || (letter == '\\' && ch == '4'))
            && key.modifiers.control)
            || key.code == KeyCode::Char(raw))
}
impl Editor {
    pub(crate) fn terminal_owns_input(&self) -> bool {
        self.terminal_input_active() && self.input_owner() == InputOwner::Terminal
    }
    /// TERMINAL mode: every key goes to the child — Esc and Ctrl-W
    /// included — except the one escape chord `Ctrl-\ Ctrl-N` (0090 §4).
    pub(crate) fn feed_terminal(&mut self, input: Input) {
        let Some(session) = self
            .terminal_document(self.current())
            .map(|document| document.session)
        else {
            return;
        };
        if self
            .terminals
            .prefix
            .as_ref()
            .is_some_and(|prefix| prefix.session != session || prefix.focus != self.focus_epoch)
        {
            self.terminals.prefix = None;
        }
        if let Input::Key(key) = &input {
            if let Some(prefix) = &mut self.terminals.prefix {
                if key.kind == KeyKind::Release && key.code == prefix.press.code {
                    prefix.release = Some(*key);
                    return;
                }
                if key.kind != KeyKind::Release && control(*key, 'n', '\x0e') {
                    self.terminals.prefix = None;
                    self.enter_terminal_normal();
                    return;
                }
            } else if key.kind == KeyKind::Press && control(*key, '\\', '\x1c') {
                self.terminals.prefix = Some(Prefix {
                    session,
                    focus: self.focus_epoch,
                    press: *key,
                    release: None,
                });
                return;
            }
        }
        if let Some(prefix) = self.terminals.prefix.take() {
            self.send_terminal_input(session, Input::Key(prefix.press));
            if let Some(release) = prefix.release {
                self.send_terminal_input(session, Input::Key(release));
            }
        }
        self.send_terminal_input(session, input);
    }

    /// `Ctrl-\ Ctrl-N`: NORMAL mode on the live buffer, cursor on the
    /// child's cursor (Neovim). No message: the echo row would cover the
    /// prompt line (0089 T12).
    fn enter_terminal_normal(&mut self) {
        let cursor = self
            .terminal_frame(self.current())
            .map(|frame| frame.cursor_byte());
        self.view_mut().terminal_input = false;
        self.mode = Mode::Normal;
        if let Some(cursor) = cursor {
            self.set_head(cursor);
        }
        self.message.clear();
    }

    fn send_terminal_input(&mut self, session: SessionId, input: Input) {
        let (kind, bytes) = match &input {
            Input::Key(_) => ("key", 0),
            Input::Text(text) => ("text", text.len()),
            Input::Paste(text) => ("paste", text.len()),
        };
        let tape = self.tape.clone();
        let result: Result<(), String> =
            match tape.call("terminal.input", &(session, kind, bytes), || {
                self.terminals
                    .entries
                    .get(&session)
                    .and_then(|entry| entry.service.as_ref())
                    .ok_or_else(|| "terminal service unavailable".to_owned())?
                    .input(input)
                    .map_err(|error| error.to_string())
            }) {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
        if let Err(error) = result {
            self.message = error;
        }
    }

    /// `i a I A` on a terminal buffer: TERMINAL mode at the child's
    /// cursor. An exited session has no child to type into; the key is
    /// consumed and says so.
    pub(crate) fn enter_terminal_input(&mut self) -> bool {
        let document = self.current();
        let Some(session) = self
            .terminal_document(document)
            .map(|source| source.session)
        else {
            return false;
        };
        if !self
            .terminals
            .entries
            .get(&session)
            .is_some_and(|entry| entry.phase.live())
        {
            self.message = "terminal has exited — :terminal starts a new session".into();
            return true;
        }
        self.terminals.prefix = None;
        self.view_mut().terminal_input = true;
        self.mode = Mode::Normal;
        if let Some(cursor) = self
            .terminal_frame(document)
            .map(|frame| frame.cursor_byte())
        {
            self.set_head(cursor);
        }
        self.message.clear();
        true
    }

    pub(crate) fn terminal_focus_changed(&mut self, focused: bool) {
        self.terminals.focused = focused;
        if !focused {
            self.terminals.prefix = None;
        }
        if self.terminal_input_active() {
            if let Some(session) = self
                .terminal_document(self.current())
                .map(|source| source.session)
            {
                self.send_terminal_focus(session, focused);
            }
        }
    }
    fn send_terminal_focus(&mut self, session: SessionId, focused: bool) {
        let tape = self.tape.clone();
        let result: Result<(), String> =
            match tape.call("terminal.focus", &(session, focused), || {
                self.terminals
                    .entries
                    .get(&session)
                    .and_then(|entry| entry.service.as_ref())
                    .ok_or_else(|| "terminal service unavailable".to_owned())?
                    .focus(focused)
                    .map_err(|error| error.to_string())
            }) {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
        if let Err(error) = result {
            self.message = error;
        }
    }
}
