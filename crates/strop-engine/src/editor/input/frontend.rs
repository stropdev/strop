//! Editor normalization runs only after the engine has selected the input owner.
//! The native frontend must preserve modifiers and releases for other owners.
use crate::editor::{events::AppEvent, Editor, Key};
use strop_core::frontend_input::{Input, KeyCode, KeyEvent, KeyKind};

impl Editor {
    pub(crate) fn handle_frontend_input(&mut self, input: Input) {
        if matches!(&input, Input::Key(key) if key.kind != KeyKind::Press) {
            self.terminals.keyboard = strop_terminal::model::SUPPORTED_KEYBOARD_FLAGS;
        }
        if self.terminal_owns_input() {
            self.feed_terminal(input);
            return;
        }
        match input {
            Input::Key(key) => {
                for event in editor_events(key).into_iter().flatten() {
                    self.handle_app_event(event);
                }
            }
            Input::Paste(text) => self.handle_app_event(AppEvent::Paste(text)),
            Input::Text(text) => {
                for ch in text.chars() {
                    self.feed(Key::Char(ch));
                }
            }
        }
    }
}

/// Legacy Alt also represents a coalesced Escape prefix. Preserve existing editor
/// semantics, including Esc before an unmapped base key, but never apply to PTYs.
fn editor_events(mut key: KeyEvent) -> [Option<AppEvent>; 2] {
    if key.kind == KeyKind::Release {
        return [None, None];
    }
    let alt = key.modifiers.alt;
    key.modifiers.alt = false;
    let base = if (key.modifiers.control && key.code == KeyCode::Char('c'))
        || key.code == KeyCode::Char('\x03')
    {
        Some(AppEvent::QuitIntent)
    } else {
        editor_key(key).map(AppEvent::EditorKey)
    };
    if alt {
        [Some(AppEvent::EditorKey(Key::Esc)), base]
    } else {
        [base, None]
    }
}

fn editor_key(key: KeyEvent) -> Option<Key> {
    let control = key.modifiers.control;
    Some(match key.code {
        KeyCode::Escape => Key::Esc,
        KeyCode::Enter => Key::Enter,
        KeyCode::Char('d') if control => Key::CtrlD,
        KeyCode::Char('u') if control => Key::CtrlU,
        KeyCode::Char('f') if control => Key::CtrlF,
        KeyCode::Char('b') if control => Key::CtrlB,
        KeyCode::Char('6') if control => Key::CtrlCaret,
        KeyCode::Char('\x1e') => Key::CtrlCaret,
        KeyCode::Char('\x04') => Key::CtrlD,
        KeyCode::Char(' ') if control => Key::CtrlSpace,
        KeyCode::Null | KeyCode::Char('\0') => Key::CtrlSpace,
        KeyCode::Char('o') if control => Key::CtrlO,
        KeyCode::Char('\x0f') => Key::CtrlO,
        KeyCode::Backspace => Key::Backspace,
        KeyCode::Tab if key.modifiers.shift => Key::Backtab,
        KeyCode::Tab => Key::Tab,
        KeyCode::Char('\x12') => Key::CtrlR,
        KeyCode::Char('\x17') => Key::CtrlW,
        KeyCode::Char('\x18') => Key::CtrlX,
        KeyCode::BackTab => Key::Backtab,
        KeyCode::Up => Key::Up,
        KeyCode::Down => Key::Down,
        KeyCode::Left => Key::Left,
        KeyCode::Right => Key::Right,
        KeyCode::Char('i') if control => Key::Tab,
        KeyCode::Char('m') if control => Key::Enter,
        KeyCode::Char('[') if control => Key::Esc,
        KeyCode::Char('n') if control => Key::Down,
        KeyCode::Char('p') if control => Key::Up,
        KeyCode::Char('r') if control => Key::CtrlR,
        KeyCode::Char('x') if control => Key::CtrlX,
        KeyCode::Char('l') if control => Key::CtrlL,
        KeyCode::Char('\x0c') => Key::CtrlL,
        KeyCode::Char('v') if control => Key::CtrlV,
        KeyCode::Char('\x16') => Key::CtrlV,
        KeyCode::Char('w') if control => Key::CtrlW,
        KeyCode::Char(ch) => Key::Char(ch),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::editor::Mode;
    fn send(editor: &mut Editor, code: KeyCode, alt: bool, control: bool) {
        let mut event = KeyEvent::press(code);
        event.modifiers.alt = alt;
        event.modifiers.control = control;
        editor.handle_app_event(AppEvent::Input(Input::Key(event)));
    }
    #[test]
    fn coalesced_escape_leaves_insert_before_applying_motion() {
        let mut editor = Editor::new(strop_core::Buffer::from_text(""));
        editor.feed_text("iwritewrite");
        send(&mut editor, KeyCode::Char('h'), true, false);
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.buf().text().to_string(), "writewrite");
        assert_eq!(editor.head(), 8);
    }
    #[test]
    fn coalesced_escape_is_not_motion_specific() {
        let mut editor = Editor::new(strop_core::Buffer::from_text(""));
        editor.feed_text("iwritewrite");
        send(&mut editor, KeyCode::Char('x'), true, false);
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.buf().text().to_string(), "writewrit");
    }
    #[test]
    fn releases_do_not_edit_and_unmapped_alt_still_escapes() {
        let mut editor = Editor::new(strop_core::Buffer::from_text(""));
        editor.feed_text("iabc");
        let mut event = KeyEvent::press(KeyCode::Char('x'));
        event.kind = KeyKind::Release;
        event.modifiers.alt = true;
        editor.handle_app_event(AppEvent::Input(Input::Key(event)));
        assert_eq!(editor.mode, Mode::Insert);
        assert_eq!(editor.buf().text().to_string(), "abc");
        send(&mut editor, KeyCode::Function(7), true, false);
        assert_eq!(editor.mode, Mode::Normal);
        assert_eq!(editor.buf().text().to_string(), "abc");
    }
    #[test]
    fn legacy_nul_reaches_query_suggestions() {
        let directory = tempfile::tempdir().unwrap();
        let mut editor = Editor::new_in(
            strop_core::Buffer::from_text(""),
            directory.path().to_owned(),
        );
        editor.open_picker(strop_picker::Kind::Files);
        editor.feed_text("lang");
        send(&mut editor, KeyCode::Null, false, false);
        editor.feed(Key::Enter);
        assert_eq!(
            editor.picker.as_ref().unwrap().picker.input.text(),
            "language:"
        );
    }
}
