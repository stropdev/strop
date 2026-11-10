//! The tty byte decoder: raw terminal input → lossless frontend events.
//!
//! strop owns this instead of crossterm's reader because crossterm folds an
//! `ESC ESC` that arrives in one read into a single Esc (B4) — a fast double
//! Escape, which vim reads as two keys. Here a second ESC always starts a
//! new key. Otherwise the semantics follow crossterm's unix parser (legacy
//! xterm sequences, SS3, the kitty keyboard protocol, bracketed paste and
//! focus reports), so the engine sees the same facts as before. Editor
//! normalization belongs after input ownership in the engine; nothing here
//! expands Alt, filters releases or turns Ctrl-C into quit.
use strop_core::frontend_input::{
    KeyCode, KeyEvent, KeyKind, KeyState, MediaKey, ModifierKey, Modifiers,
};

const ESC: u8 = 0x1b;

#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) enum Decoded {
    Key(KeyEvent),
    Paste(String),
    Focus(bool),
}

/// Bytes of a sequence that has not finished arriving.
#[derive(Default)]
pub(super) struct Decoder {
    pending: Vec<u8>,
}

enum Step {
    /// An event and the bytes it consumed.
    Event(usize, Decoded),
    /// A complete sequence that carries no input (a terminal report, mouse
    /// we never enabled, garbage): drop these bytes.
    Skip(usize),
    /// The bytes so far are a prefix of a sequence: wait for the next read.
    Incomplete,
}

impl Decoder {
    /// Feed one read. `more` says the read filled its buffer, so further
    /// bytes are already waiting: a trailing lone ESC is then a sequence
    /// prefix, otherwise it is the Escape key (crossterm's rule, no timer).
    pub(super) fn feed(&mut self, bytes: &[u8], more: bool, out: &mut Vec<Decoded>) {
        self.pending.extend_from_slice(bytes);
        let mut start = 0;
        while start < self.pending.len() {
            match step(&self.pending[start..], more) {
                Step::Event(used, event) => {
                    out.push(event);
                    start += used;
                }
                Step::Skip(used) => start += used,
                Step::Incomplete => break,
            }
        }
        self.pending.drain(..start);
    }
}

fn key(code: KeyCode, modifiers: Modifiers) -> Decoded {
    Decoded::Key(KeyEvent {
        code,
        modifiers,
        ..KeyEvent::press(code)
    })
}

fn plain(code: KeyCode) -> Decoded {
    key(code, Modifiers::default())
}

fn control(ch: char) -> Decoded {
    key(
        KeyCode::Char(ch),
        Modifiers {
            control: true,
            ..Modifiers::default()
        },
    )
}

fn step(buffer: &[u8], more: bool) -> Step {
    match buffer[0] {
        ESC => escape(buffer, more),
        b'\r' => Step::Event(1, plain(KeyCode::Enter)),
        b'\t' => Step::Event(1, plain(KeyCode::Tab)),
        0x7f => Step::Event(1, plain(KeyCode::Backspace)),
        byte @ 0x01..=0x1a => Step::Event(1, control(char::from(byte - 0x01 + b'a'))),
        byte @ 0x1c..=0x1f => Step::Event(1, control(char::from(byte - 0x1c + b'4'))),
        0 => Step::Event(1, control(' ')),
        _ => utf8(buffer),
    }
}

fn utf8(buffer: &[u8]) -> Step {
    let width = match buffer[0] {
        0x00..=0x7f => 1,
        0xc0..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf7 => 4,
        _ => return Step::Skip(1),
    };
    if buffer.len() < width {
        // A partial character only waits while its continuation bytes are
        // plausible; anything else is garbage, dropped byte by byte.
        return if buffer[1..].iter().all(|byte| byte & 0xc0 == 0x80) {
            Step::Incomplete
        } else {
            Step::Skip(1)
        };
    }
    let Some(ch) = std::str::from_utf8(&buffer[..width])
        .ok()
        .and_then(|text| text.chars().next())
    else {
        return Step::Skip(1);
    };
    let modifiers = Modifiers {
        shift: ch.is_uppercase(),
        ..Modifiers::default()
    };
    Step::Event(width, key(KeyCode::Char(ch), modifiers))
}

fn escape(buffer: &[u8], more: bool) -> Step {
    let Some(&next) = buffer.get(1) else {
        return if more {
            Step::Incomplete
        } else {
            Step::Event(1, plain(KeyCode::Escape))
        };
    };
    match next {
        // Two Escapes are two keys, never Alt-Esc (B4, vim behaviour).
        ESC => Step::Event(1, plain(KeyCode::Escape)),
        b'[' => csi(buffer),
        b'O' => ss3(buffer),
        _ => match step(&buffer[1..], more) {
            Step::Event(used, Decoded::Key(mut event)) => {
                event.modifiers.alt = true;
                Step::Event(used + 1, Decoded::Key(event))
            }
            Step::Event(used, other) => Step::Event(used + 1, other),
            Step::Skip(used) => Step::Skip(used + 1),
            Step::Incomplete => Step::Incomplete,
        },
    }
}

fn ss3(buffer: &[u8]) -> Step {
    let Some(&last) = buffer.get(2) else {
        return Step::Incomplete;
    };
    let code = match last {
        b'D' => KeyCode::Left,
        b'C' => KeyCode::Right,
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'H' => KeyCode::Home,
        b'F' => KeyCode::End,
        b'P'..=b'S' => KeyCode::Function(1 + last - b'P'),
        _ => return Step::Skip(3),
    };
    Step::Event(3, plain(code))
}

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

fn csi(buffer: &[u8]) -> Step {
    // The linux console's F1–F5: `CSI [ A` … `CSI [ E`.
    if buffer.get(2) == Some(&b'[') {
        return match buffer.get(3) {
            None => Step::Incomplete,
            Some(&last @ b'A'..=b'E') => Step::Event(4, plain(KeyCode::Function(1 + last - b'A'))),
            Some(_) => Step::Skip(4),
        };
    }
    if buffer.starts_with(PASTE_START) {
        let body = &buffer[PASTE_START.len()..];
        return match body.windows(PASTE_END.len()).position(|w| w == PASTE_END) {
            Some(end) => Step::Event(
                PASTE_START.len() + end + PASTE_END.len(),
                Decoded::Paste(String::from_utf8_lossy(&body[..end]).into_owned()),
            ),
            None => Step::Incomplete,
        };
    }
    // Parameter and intermediate bytes, then one final byte (ECMA-48).
    let Some(final_at) = buffer[2..]
        .iter()
        .position(|byte| (0x40..=0x7e).contains(byte))
        .map(|at| at + 2)
    else {
        return Step::Incomplete;
    };
    let used = final_at + 1;
    let Ok(params) = std::str::from_utf8(&buffer[2..final_at]) else {
        return Step::Skip(used);
    };
    match csi_event(params, buffer[final_at]) {
        Some(event) => Step::Event(used, event),
        None => Step::Skip(used),
    }
}

fn csi_event(params: &str, last: u8) -> Option<Decoded> {
    if params.starts_with(['?', '<', '>', '=']) {
        return None; // terminal reports and mouse: never input here
    }
    if params.is_empty() {
        return Some(match last {
            b'D' => plain(KeyCode::Left),
            b'C' => plain(KeyCode::Right),
            b'A' => plain(KeyCode::Up),
            b'B' => plain(KeyCode::Down),
            b'H' => plain(KeyCode::Home),
            b'F' => plain(KeyCode::End),
            b'Z' => key(
                KeyCode::BackTab,
                Modifiers {
                    shift: true,
                    ..Modifiers::default()
                },
            ),
            b'I' => Decoded::Focus(true),
            b'O' => Decoded::Focus(false),
            // Kitty omits the `1` of `CSI 1 P` without modifiers; `CSI R`
            // stays a cursor report, as in crossterm.
            b'P' => plain(KeyCode::Function(1)),
            b'Q' => plain(KeyCode::Function(2)),
            b'S' => plain(KeyCode::Function(4)),
            _ => return None,
        });
    }
    match last {
        b'~' => special_key(params),
        b'u' => kitty_key(params),
        // cursor position reports and rxvt mouse
        b'R' | b'M' => None,
        _ => modified_key(params, last),
    }
}

/// `mask[:kind]` of a modified key: crossterm's shift/alt/ctrl/super/
/// hyper/meta bits plus the caps/num lock state.
struct Mask {
    modifiers: Modifiers,
    kind: KeyKind,
    state: KeyState,
}

fn mask(field: Option<&str>) -> Option<Mask> {
    let mut parts = field?.split(':');
    let bits = parts.next()?.parse::<u8>().ok()?.saturating_sub(1);
    let kind = match parts.next().and_then(|kind| kind.parse::<u8>().ok()) {
        Some(2) => KeyKind::Repeat,
        Some(3) => KeyKind::Release,
        _ => KeyKind::Press,
    };
    Some(Mask {
        modifiers: Modifiers {
            shift: bits & 1 != 0,
            alt: bits & 2 != 0,
            control: bits & 4 != 0,
            super_key: bits & 8 != 0,
            hyper: bits & 16 != 0,
            meta: bits & 32 != 0,
        },
        kind,
        state: KeyState {
            keypad: false,
            caps_lock: bits & 64 != 0,
            num_lock: bits & 128 != 0,
        },
    })
}

fn with_mask(code: KeyCode, mask: Option<Mask>, keypad: bool) -> Decoded {
    let mask = mask.unwrap_or(Mask {
        modifiers: Modifiers::default(),
        kind: KeyKind::Press,
        state: KeyState::default(),
    });
    Decoded::Key(KeyEvent {
        code,
        modifiers: mask.modifiers,
        kind: mask.kind,
        state: KeyState {
            keypad,
            ..mask.state
        },
    })
}

/// `CSI 1 ; mask[:kind] A` — xterm's modified cursor and F1–F4 keys.
fn modified_key(params: &str, last: u8) -> Option<Decoded> {
    let mut fields = params.split(';');
    fields.next();
    let mask = mask(fields.next()).or_else(|| {
        // `CSI 5 A`: an old-style bare modifier digit.
        let digit = params.chars().last()?.to_digit(10)? as u8;
        mask(Some(&digit.to_string()))
    });
    let code = match last {
        b'A' => KeyCode::Up,
        b'B' => KeyCode::Down,
        b'C' => KeyCode::Right,
        b'D' => KeyCode::Left,
        b'F' => KeyCode::End,
        b'H' => KeyCode::Home,
        b'P' => KeyCode::Function(1),
        b'Q' => KeyCode::Function(2),
        b'S' => KeyCode::Function(4),
        _ => return None,
    };
    Some(with_mask(code, mask, false))
}

/// `CSI number ; mask[:kind] ~` — home/insert/delete/end/paging and F5+.
fn special_key(params: &str) -> Option<Decoded> {
    let mut fields = params.split(';');
    let number = fields.next()?.parse::<u8>().ok()?;
    let code = match number {
        1 | 7 => KeyCode::Home,
        2 => KeyCode::Insert,
        3 => KeyCode::Delete,
        4 | 8 => KeyCode::End,
        5 => KeyCode::PageUp,
        6 => KeyCode::PageDown,
        11..=15 => KeyCode::Function(number - 10),
        17..=21 => KeyCode::Function(number - 11),
        23..=26 => KeyCode::Function(number - 12),
        28..=29 => KeyCode::Function(number - 15),
        31..=34 => KeyCode::Function(number - 17),
        _ => return None,
    };
    Some(with_mask(code, mask(fields.next()), false))
}

/// `CSI code[:shifted[:base]] ; mask[:kind] [; text] u` — the kitty
/// keyboard protocol (and fixterms' `CSI code ; mask u`).
fn kitty_key(params: &str) -> Option<Decoded> {
    let mut fields = params.split(';');
    let mut codes = fields.next()?.split(':');
    let codepoint = codes.next()?.parse::<u32>().ok()?;
    let mut mask = mask(fields.next()).unwrap_or(Mask {
        modifiers: Modifiers::default(),
        kind: KeyKind::Press,
        state: KeyState::default(),
    });
    let (mut code, keypad) = match functional(codepoint) {
        Some(found) => found,
        None => (
            match char::from_u32(codepoint)? {
                '\x1b' => KeyCode::Escape,
                '\r' => KeyCode::Enter,
                '\t' if mask.modifiers.shift => KeyCode::BackTab,
                '\t' => KeyCode::Tab,
                '\x7f' => KeyCode::Backspace,
                ch => KeyCode::Char(ch),
            },
            false,
        ),
    };
    if let KeyCode::Modifier(modifier) = code {
        let held = &mut mask.modifiers;
        match modifier {
            ModifierKey::LeftAlt | ModifierKey::RightAlt => held.alt = true,
            ModifierKey::LeftControl | ModifierKey::RightControl => held.control = true,
            ModifierKey::LeftShift | ModifierKey::RightShift => held.shift = true,
            ModifierKey::LeftSuper | ModifierKey::RightSuper => held.super_key = true,
            ModifierKey::LeftHyper | ModifierKey::RightHyper => held.hyper = true,
            ModifierKey::LeftMeta | ModifierKey::RightMeta => held.meta = true,
            ModifierKey::IsoLevel3Shift | ModifierKey::IsoLevel5Shift => {}
        }
    }
    // Report-alternate-keys: with Shift held, the layout's shifted scalar
    // replaces the base key and absorbs the Shift (kitty's `59:58;2u` is `:`).
    if mask.modifiers.shift {
        if let Some(shifted) = codes
            .next()
            .and_then(|code| code.parse::<u32>().ok())
            .and_then(char::from_u32)
        {
            code = KeyCode::Char(shifted);
            mask.modifiers.shift = false;
        }
    }
    Some(with_mask(code, Some(mask), keypad))
}

/// Kitty's private-use functional key codes; the bool marks keypad keys.
fn functional(codepoint: u32) -> Option<(KeyCode, bool)> {
    const KEYPAD_CHARS: &[u8; 10] = b"0123456789";
    let keypad = |code| Some((code, true));
    match codepoint {
        57399..=57408 => keypad(KeyCode::Char(char::from(
            KEYPAD_CHARS[(codepoint - 57399) as usize],
        ))),
        57409 => keypad(KeyCode::Char('.')),
        57410 => keypad(KeyCode::Char('/')),
        57411 => keypad(KeyCode::Char('*')),
        57412 => keypad(KeyCode::Char('-')),
        57413 => keypad(KeyCode::Char('+')),
        57414 => keypad(KeyCode::Enter),
        57415 => keypad(KeyCode::Char('=')),
        57416 => keypad(KeyCode::Char(',')),
        57417 => keypad(KeyCode::Left),
        57418 => keypad(KeyCode::Right),
        57419 => keypad(KeyCode::Up),
        57420 => keypad(KeyCode::Down),
        57421 => keypad(KeyCode::PageUp),
        57422 => keypad(KeyCode::PageDown),
        57423 => keypad(KeyCode::Home),
        57424 => keypad(KeyCode::End),
        57425 => keypad(KeyCode::Insert),
        57426 => keypad(KeyCode::Delete),
        57427 => keypad(KeyCode::KeypadBegin),
        _ => {
            let code = match codepoint {
                57358 => KeyCode::CapsLock,
                57359 => KeyCode::ScrollLock,
                57360 => KeyCode::NumLock,
                57361 => KeyCode::PrintScreen,
                57362 => KeyCode::Pause,
                57363 => KeyCode::Menu,
                57376..=57398 => KeyCode::Function((codepoint - 57376 + 13) as u8),
                57428 => KeyCode::Media(MediaKey::Play),
                57429 => KeyCode::Media(MediaKey::Pause),
                57430 => KeyCode::Media(MediaKey::PlayPause),
                57431 => KeyCode::Media(MediaKey::Reverse),
                57432 => KeyCode::Media(MediaKey::Stop),
                57433 => KeyCode::Media(MediaKey::FastForward),
                57434 => KeyCode::Media(MediaKey::Rewind),
                57435 => KeyCode::Media(MediaKey::TrackNext),
                57436 => KeyCode::Media(MediaKey::TrackPrevious),
                57437 => KeyCode::Media(MediaKey::Record),
                57438 => KeyCode::Media(MediaKey::LowerVolume),
                57439 => KeyCode::Media(MediaKey::RaiseVolume),
                57440 => KeyCode::Media(MediaKey::MuteVolume),
                57441 => KeyCode::Modifier(ModifierKey::LeftShift),
                57442 => KeyCode::Modifier(ModifierKey::LeftControl),
                57443 => KeyCode::Modifier(ModifierKey::LeftAlt),
                57444 => KeyCode::Modifier(ModifierKey::LeftSuper),
                57445 => KeyCode::Modifier(ModifierKey::LeftHyper),
                57446 => KeyCode::Modifier(ModifierKey::LeftMeta),
                57447 => KeyCode::Modifier(ModifierKey::RightShift),
                57448 => KeyCode::Modifier(ModifierKey::RightControl),
                57449 => KeyCode::Modifier(ModifierKey::RightAlt),
                57450 => KeyCode::Modifier(ModifierKey::RightSuper),
                57451 => KeyCode::Modifier(ModifierKey::RightHyper),
                57452 => KeyCode::Modifier(ModifierKey::RightMeta),
                57453 => KeyCode::Modifier(ModifierKey::IsoLevel3Shift),
                57454 => KeyCode::Modifier(ModifierKey::IsoLevel5Shift),
                _ => return None,
            };
            Some((code, false))
        }
    }
}

#[cfg(test)]
mod tests;
