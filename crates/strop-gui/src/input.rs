//! Native input facts → frontend-neutral protocol input. The engine owns
//! input ownership and semantics; this module only prevents GPUI's logical
//! keybinding projection from dropping physical modifiers or committed text.
//!
//! Windows IME composition stays frontend-local until commit. A committed
//! string is one `Input::Text`; physical key repeats and releases remain
//! visible so terminal and editor owners can preserve their existing policy.

use gpui::{KeyDownEvent, KeyUpEvent};
use strop_core::frontend_input::{Input, KeyCode, KeyEvent, KeyKind, KeyState, Modifiers};

/// One keyboard event in the protocol's physical fact form. `None` is an
/// honest unknown (a non-scalar named key or an input source that gives no
/// typed/code distinction); the caller must not guess a vim binding.
pub fn key_input(event: &KeyDownEvent) -> Option<Input> {
    let key = &event.keystroke;
    let code = key_code(&key.key)?;
    let mut input = KeyEvent {
        code,
        modifiers: modifiers(key.modifiers),
        kind: if event.is_held {
            KeyKind::Repeat
        } else {
            KeyKind::Press
        },
        state: KeyState::default(),
    };
    if let Some(text) = key.key_char.as_deref() {
        if let Some(character) = single_char(text) {
            input.code = KeyCode::Char(character);
        }
    }
    Some(Input::Key(input))
}

/// Key releases matter to terminal ownership and must not be coalesced.
pub fn key_release_input(event: &KeyUpEvent) -> Option<Input> {
    let mut input = KeyEvent {
        code: key_code(&event.keystroke.key)?,
        modifiers: modifiers(event.keystroke.modifiers),
        kind: KeyKind::Release,
        state: KeyState::default(),
    };
    if let Some(character) = event.keystroke.key_char.as_deref().and_then(single_char) {
        input.code = KeyCode::Char(character);
    }
    Some(Input::Key(input))
}

fn modifiers(input: gpui::Modifiers) -> Modifiers {
    Modifiers {
        shift: input.shift,
        control: input.control,
        alt: input.alt,
        super_key: input.platform,
        hyper: false,
        meta: false,
    }
}

fn single_char(text: &str) -> Option<char> {
    let mut chars = text.chars();
    let character = chars.next()?;
    chars.next().is_none().then_some(character)
}

fn key_code(key: &str) -> Option<KeyCode> {
    if let Some(character) = single_char(key) {
        return Some(KeyCode::Char(character));
    }
    match key {
        "escape" => Some(KeyCode::Escape),
        "enter" | "return" => Some(KeyCode::Enter),
        "backspace" => Some(KeyCode::Backspace),
        "up" => Some(KeyCode::Up),
        "down" => Some(KeyCode::Down),
        "left" => Some(KeyCode::Left),
        "right" => Some(KeyCode::Right),
        "home" => Some(KeyCode::Home),
        "end" => Some(KeyCode::End),
        "pageup" | "page-up" => Some(KeyCode::PageUp),
        "pagedown" | "page-down" => Some(KeyCode::PageDown),
        "tab" => Some(KeyCode::Tab),
        "backtab" | "shift-tab" => Some(KeyCode::BackTab),
        "delete" => Some(KeyCode::Delete),
        "insert" => Some(KeyCode::Insert),
        key if key.starts_with(['f', 'F']) => key[1..]
            .parse::<u8>()
            .ok()
            .filter(|number| *number != 0)
            .map(KeyCode::Function),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn down(key: &str, character: Option<&str>, control: bool, is_held: bool) -> KeyDownEvent {
        KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers: gpui::Modifiers {
                    control,
                    ..Default::default()
                },
                key: key.to_string(),
                key_char: character.map(str::to_string),
            },
            is_held,
            prefer_character_input: false,
        }
    }

    #[test]
    fn physical_ctrl_stays_ctrl_even_with_typed_character() {
        let Some(Input::Key(input)) = key_input(&down("v", Some("v"), true, false)) else {
            panic!("Ctrl-V maps to physical input")
        };
        assert_eq!(input.code, KeyCode::Char('v'));
        assert!(input.modifiers.control);
        assert_eq!(input.kind, KeyKind::Press);
    }

    #[test]
    fn key_repeat_and_release_survive_as_distinct_kinds() {
        let Some(Input::Key(repeat)) = key_input(&down("a", Some("a"), false, true)) else {
            panic!("repeated key maps to physical input")
        };
        assert_eq!(repeat.kind, KeyKind::Repeat);
        let release = KeyUpEvent {
            keystroke: gpui::Keystroke {
                key: "a".into(),
                key_char: Some("a".into()),
                ..Default::default()
            },
        };
        let Some(Input::Key(input)) = key_release_input(&release) else {
            panic!("released key maps to physical input")
        };
        assert_eq!(input.kind, KeyKind::Release);
    }

    #[test]
    fn altgr_typed_text_wins_but_keeps_physical_modifiers() {
        let event = KeyDownEvent {
            keystroke: gpui::Keystroke {
                modifiers: gpui::Modifiers {
                    control: true,
                    alt: true,
                    ..Default::default()
                },
                key: "q".into(),
                key_char: Some("ß".into()),
            },
            is_held: false,
            prefer_character_input: true,
        };
        let Some(Input::Key(input)) = key_input(&event) else {
            panic!("AltGr text maps to physical input")
        };
        assert_eq!(input.code, KeyCode::Char('ß'));
        assert!(input.modifiers.control);
        assert!(input.modifiers.alt);
    }

    #[test]
    fn named_navigation_keys_have_physical_codes() {
        for (name, code) in [
            ("enter", KeyCode::Enter),
            ("up", KeyCode::Up),
            ("page-up", KeyCode::PageUp),
            ("shift-tab", KeyCode::BackTab),
            ("f4", KeyCode::Function(4)),
        ] {
            let Some(Input::Key(input)) = key_input(&down(name, None, false, false)) else {
                panic!("{name} maps to physical input")
            };
            assert_eq!(input.code, code);
        }
    }

    #[test]
    fn unknown_named_keys_refuse_a_guessed_mapping() {
        assert!(key_input(&down("media-play", None, false, false)).is_none());
    }
}
