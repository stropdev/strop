//! Streaming script-token decoding shared by Editor::feed_text and headless.
//! Tokens retain physical modifiers until the engine selects the input owner.
//! Unknown tokens stay literal; a lone '<' never fabricates a closing '>'.
use strop_core::frontend_input::{KeyCode, KeyEvent, Modifiers};

pub fn parse(text: &str) -> Keys<'_> {
    Keys {
        remaining: text,
        literal_bytes: 0,
    }
}
pub struct Keys<'a> {
    remaining: &'a str,
    literal_bytes: usize,
}
impl Iterator for Keys<'_> {
    type Item = KeyEvent;
    fn next(&mut self) -> Option<KeyEvent> {
        let ch = self.remaining.chars().next()?;
        if self.literal_bytes > 0 {
            self.remaining = &self.remaining[ch.len_utf8()..];
            self.literal_bytes -= ch.len_utf8();
            return Some(KeyEvent::press(KeyCode::Char(ch)));
        }
        if ch == '<' {
            if let Some(end) = self.remaining.find('>') {
                if let Some(key) = token(&self.remaining[1..end]) {
                    self.remaining = &self.remaining[end + 1..];
                    return Some(key);
                }
                self.literal_bytes = end;
            }
        }
        self.remaining = &self.remaining[ch.len_utf8()..];
        Some(character(ch))
    }
}

fn character(ch: char) -> KeyEvent {
    let code = match ch {
        '\x1b' => KeyCode::Escape,
        '\r' | '\n' => KeyCode::Enter,
        '\x7f' | '\x08' => KeyCode::Backspace,
        '\t' => KeyCode::Tab,
        '\0' => KeyCode::Null,
        '\x01'..='\x1a' => {
            let mut key = KeyEvent::press(KeyCode::Char(char::from(ch as u8 - 1 + b'a')));
            key.modifiers.control = true;
            return key;
        }
        '\x1c'..='\x1f' => {
            let scalar = match ch {
                '\x1c' => '\\',
                '\x1d' => ']',
                '\x1e' => '6',
                _ => '_',
            };
            let mut key = KeyEvent::press(KeyCode::Char(scalar));
            key.modifiers.control = true;
            return key;
        }
        ch => KeyCode::Char(ch),
    };
    KeyEvent::press(code)
}

fn token(mut text: &str) -> Option<KeyEvent> {
    let mut modifiers = Modifiers::default();
    while let Some((modifier, rest)) = text.split_once('-') {
        if modifier.eq_ignore_ascii_case("c") {
            modifiers.control = true;
        } else if modifier.eq_ignore_ascii_case("s") {
            modifiers.shift = true;
        } else if modifier.eq_ignore_ascii_case("a") || modifier.eq_ignore_ascii_case("m") {
            modifiers.alt = true;
        } else if modifier.eq_ignore_ascii_case("d") || modifier.eq_ignore_ascii_case("super") {
            modifiers.super_key = true;
        } else {
            break;
        }
        text = rest;
    }
    let mut code = NAMES
        .iter()
        .find(|(name, _)| text.eq_ignore_ascii_case(name))
        .map(|(_, code)| *code);
    if code.is_none() && text.starts_with(['f', 'F']) {
        code = text[1..]
            .parse::<u8>()
            .ok()
            .filter(|number| *number != 0)
            .map(KeyCode::Function);
    }
    if code.is_none() {
        let mut chars = text.chars();
        if let Some(ch) = chars.next() {
            if chars.next().is_none() && modifiers != Modifiers::default() {
                code = Some(KeyCode::Char(ch));
            }
        }
    }
    let mut code = code?;
    if code == KeyCode::Tab && modifiers.shift {
        code = KeyCode::BackTab;
    }
    if code == KeyCode::Char('^') && modifiers.control {
        code = KeyCode::Char('6');
        modifiers.shift = true;
    }
    let mut key = KeyEvent::press(code);
    key.modifiers = modifiers;
    Some(key)
}
const NAMES: &[(&str, KeyCode)] = &[
    ("esc", KeyCode::Escape),
    ("cr", KeyCode::Enter),
    ("enter", KeyCode::Enter),
    ("bs", KeyCode::Backspace),
    ("backspace", KeyCode::Backspace),
    ("space", KeyCode::Char(' ')),
    ("lt", KeyCode::Char('<')),
    ("gt", KeyCode::Char('>')),
    ("up", KeyCode::Up),
    ("down", KeyCode::Down),
    ("left", KeyCode::Left),
    ("right", KeyCode::Right),
    ("tab", KeyCode::Tab),
    ("home", KeyCode::Home),
    ("end", KeyCode::End),
    ("pageup", KeyCode::PageUp),
    ("pagedown", KeyCode::PageDown),
    ("delete", KeyCode::Delete),
    ("insert", KeyCode::Insert),
    ("null", KeyCode::Null),
];
