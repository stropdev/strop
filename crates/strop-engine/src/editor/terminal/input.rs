use super::*;
use crate::editor::{InputOwner, Mode};
use strop_core::frontend_input::{Input, KeyCode, KeyEvent, KeyKind};

fn control(key: KeyEvent, letter: char, raw: char) -> bool {
    // Crossterm's legacy C0 decoder reports 0x1c as Ctrl-4. It is the same
    // legacy chord as Ctrl-\\; retain the original event when forwarding it.
    !key.modifiers.alt
        && !key.modifiers.super_key
        && !key.modifiers.hyper
        && !key.modifiers.meta
        && ((matches!(key.code, KeyCode::Char(ch) if ch.to_ascii_lowercase() == letter || (letter == '\\' && ch == '4'))
            && key.modifiers.control)
            || key.code == KeyCode::Char(raw))
}
enum WindowCommand {
    Inspect,
    Literal,
    Move(char),
}

/// Vim's terminal `t_CTRL-W` grammar, adapted to strop's panes: plain
/// hjkl/w (and arrows) move or cycle panes, `N` (or the Ctrl-N escape)
/// enters the pinned-snapshot inspection, `.` forwards the literal 0x17.
/// Everything else — including modified combos — falls through so the
/// child still receives what a bare keystroke would have sent.
fn window_command(key: KeyEvent) -> Option<WindowCommand> {
    if key.modifiers.control
        || key.modifiers.alt
        || key.modifiers.super_key
        || key.modifiers.hyper
        || key.modifiers.meta
    {
        if key.modifiers.control && matches!(key.code, KeyCode::Char('n' | 'N')) {
            return Some(WindowCommand::Inspect);
        }
        return None;
    }
    match key.code {
        KeyCode::Char('h') | KeyCode::Left => Some(WindowCommand::Move('h')),
        KeyCode::Char('l') | KeyCode::Right => Some(WindowCommand::Move('l')),
        KeyCode::Char('k') | KeyCode::Up => Some(WindowCommand::Move('k')),
        KeyCode::Char('j') | KeyCode::Down => Some(WindowCommand::Move('j')),
        KeyCode::Char('w') => Some(WindowCommand::Move('w')),
        KeyCode::Char('N') => Some(WindowCommand::Inspect),
        KeyCode::Char('.') => Some(WindowCommand::Literal),
        _ => None,
    }
}
impl Editor {
    pub(crate) fn terminal_owns_input(&self) -> bool {
        self.terminal_input_active() && self.input_owner() == InputOwner::Terminal
    }
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
                if key.kind != KeyKind::Release {
                    let (kind, press) = (prefix.kind, prefix.press);
                    match kind {
                        PrefixKind::Escape => {
                            if control(*key, 'n', '\x0e') {
                                self.terminals.prefix = None;
                                self.enter_terminal_normal(session);
                                return;
                            }
                        }
                        PrefixKind::Window => {
                            if let Some(command) = window_command(*key) {
                                self.terminals.prefix = None;
                                match command {
                                    WindowCommand::Inspect => {
                                        self.enter_terminal_normal(session);
                                    }
                                    WindowCommand::Literal => {
                                        self.send_terminal_input(session, Input::Key(press));
                                    }
                                    WindowCommand::Move(direction) => {
                                        self.pane_move(direction);
                                    }
                                }
                                return;
                            }
                        }
                    }
                }
            } else if key.kind == KeyKind::Press {
                if control(*key, '\\', '\x1c') {
                    self.terminals.prefix = Some(Prefix {
                        session,
                        kind: PrefixKind::Escape,
                        focus: self.focus_epoch,
                        press: *key,
                        release: None,
                    });
                    return;
                }
                if control(*key, 'w', '\x17') {
                    self.terminals.prefix = Some(Prefix {
                        session,
                        kind: PrefixKind::Window,
                        focus: self.focus_epoch,
                        press: *key,
                        release: None,
                    });
                    self.message = "ctrl-w: h j k l w panes · N inspect · . literal".into();
                    return;
                }
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

    /// The pinned-snapshot inspection mode shared by both prefix escapes.
    fn enter_terminal_normal(&mut self, session: SessionId) {
        self.send_terminal_focus(session, false);
        self.view_mut().terminal_input = false;
        self.mode = Mode::Normal;
        self.message = "terminal snapshot — i returns to live input".into();
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
                    .input(input, false)
                    .map_err(|error| error.to_string())
            }) {
                Ok(result) => result,
                Err(error) => Err(error.to_string()),
            };
        if let Err(error) = result {
            self.message = error;
        }
    }

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
            if self.terminals.capture {
                self.install_terminal_frame(document);
            } else {
                strop_trace::without_content(|| self.install_terminal_frame(document));
            }
            if let Some(cursor) = self
                .terminal_frame(document, true)
                .map(|frame| frame.cursor_byte())
            {
                self.set_head(cursor);
            }
            self.message = "terminal ended; showing its final snapshot — :terminal explicitly starts a new session".into();
            return true;
        }
        self.terminals.prefix = None;
        self.view_mut().terminal_input = true;
        self.mode = Mode::Normal;
        if !self
            .panes
            .iter()
            .any(|pane| pane.doc == document && !pane.terminal_input)
        {
            if self.terminals.capture {
                self.install_terminal_frame(document);
            } else {
                strop_trace::without_content(|| self.install_terminal_frame(document));
            }
        }
        if self.terminals.focused {
            self.send_terminal_focus(session, true);
        }
        self.message = "terminal input — Ctrl-\\ Ctrl-N enters editor Normal mode".into();
        true
    }
    pub(crate) fn terminal_paste_decision(&mut self, accept: bool) {
        let Some(session) = self
            .terminal_document(self.current())
            .map(|source| source.session)
        else {
            self.message = "current buffer is not a terminal".into();
            return;
        };
        let Some(ticket) = self
            .terminals
            .entries
            .get(&session)
            .and_then(|entry| entry.paste)
        else {
            self.message = "this terminal has no held paste".into();
            return;
        };
        let tape = self.tape.clone();
        let result: Result<(), String> = match tape.call(
            "terminal.paste-decision",
            &(session, ticket, accept),
            || {
                self.terminals
                    .entries
                    .get(&session)
                    .and_then(|entry| entry.service.as_ref())
                    .ok_or_else(|| "terminal service unavailable".to_owned())?
                    .decide_paste(ticket, accept)
                    .map_err(|error| error.to_string())
            },
        ) {
            Ok(result) => result,
            Err(error) => Err(error.to_string()),
        };
        match result {
            Ok(()) if accept => {
                self.enter_terminal_input();
            }
            Ok(()) => {
                self.message = "discarding held terminal paste".into();
            }
            Err(error) => self.message = error,
        }
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
