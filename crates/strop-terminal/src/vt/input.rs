use super::{check, ffi, Emission, Vt};
use crate::model::MAX_INPUT_BYTES;
use crate::Error;
use std::ffi::CStr;
use strop_core::frontend_input::{Input, KeyCode, KeyEvent, KeyKind, MediaKey, ModifierKey};

impl Vt {
    pub fn keyboard_capabilities(&mut self, flags: u8) -> Result<(), Error> {
        // SAFETY: this worker exclusively owns the live native terminal.
        check("keyboard capabilities", unsafe {
            ffi::strop_vt_keyboard(self.handle.as_ptr(), flags)
        })
    }

    pub fn focus(&mut self, focused: bool) -> Result<Emission, Error> {
        let mut bytes = [0; 16];
        let mut written = 0;
        // SAFETY: live unique VT owner and bounded writable output for this call.
        check("focus", unsafe {
            ffi::strop_vt_focus(
                self.handle.as_ptr(),
                i32::from(focused),
                bytes.as_mut_ptr(),
                bytes.len(),
                &mut written,
            )
        })?;
        if written > bytes.len() {
            return Err(Error::State("focus encoder exceeded its output bound"));
        }
        Ok(Emission {
            reply: bytes[..written].to_vec(),
            effects: Vec::new(),
        })
    }

    pub fn input(&mut self, input: &Input, paste_confirmed: bool) -> Result<Emission, Error> {
        match input {
            Input::Key(key) => self.key(*key),
            Input::Text(text) => {
                if text.len() > MAX_INPUT_BYTES {
                    return Err(Error::Capacity("committed terminal text"));
                }
                let mut emission = Emission::default();
                for ch in text.chars() {
                    let next = self.key(KeyEvent::press(KeyCode::Char(ch)))?;
                    if next.reply.len() > MAX_INPUT_BYTES.saturating_sub(emission.reply.len()) {
                        return Err(Error::Capacity("encoded terminal text"));
                    }
                    emission.reply.extend(next.reply);
                    emission.effects.extend(next.effects);
                }
                Ok(emission)
            }
            Input::Paste(text) => {
                // Reserve both bracketed-paste delimiters before calling native
                // code: an oversized user paste is a refusal, not parser poison.
                if text.len() > MAX_INPUT_BYTES - 12 {
                    return Err(Error::Capacity("terminal paste"));
                }
                // SAFETY: input bytes live through the synchronous native reader;
                // the VT owner is unique and callbacks only collect bounded data.
                let code = unsafe {
                    ffi::strop_vt_paste(
                        self.handle.as_ptr(),
                        text.as_ptr(),
                        text.len(),
                        i32::from(paste_confirmed),
                    )
                };
                if code == -7 {
                    return Err(Error::PasteNeedsConfirmation);
                }
                check("paste", code)?;
                self.take_events()
            }
        }
    }

    fn key(&mut self, mut key: KeyEvent) -> Result<Emission, Error> {
        if key.modifiers.hyper || key.modifiers.meta {
            return Err(Error::Unavailable(
                "the selected key encoder cannot represent Hyper/Meta modifier state".into(),
            ));
        }
        if matches!(key.code, KeyCode::Null | KeyCode::Char('\0')) {
            // Legacy NUL is Ctrl-Space. Route it through the mode-aware encoder
            // so Alt and enhanced release/repeat events are not discarded.
            key.code = KeyCode::Char(' ');
            key.modifiers.control = true;
        }
        let mut function = [0u8; 5];
        let (name, scalar) = match key.code {
            KeyCode::Char(ch) => (c"", u32::from(ch)),
            KeyCode::Function(number) if (1..=25).contains(&number) => {
                function[0] = b'F';
                let length = if number < 10 {
                    function[1] = b'0' + number;
                    2
                } else {
                    function[1] = b'0' + number / 10;
                    function[2] = b'0' + number % 10;
                    3
                };
                let name = CStr::from_bytes_with_nul(&function[..=length])
                    .map_err(|_| Error::Protocol("invalid function-key encoding".into()))?;
                (name, 0)
            }
            KeyCode::Function(_) => {
                return Err(Error::Unavailable(
                    "function key is outside the native encoder's supported range".into(),
                ))
            }
            KeyCode::Escape => (c"Escape", 0),
            KeyCode::Enter => (c"Enter", 0),
            KeyCode::Backspace => (c"Backspace", 0),
            KeyCode::Up => (c"Up", 0),
            KeyCode::Down => (c"Down", 0),
            KeyCode::Left => (c"Left", 0),
            KeyCode::Right => (c"Right", 0),
            KeyCode::Home => (c"Home", 0),
            KeyCode::End => (c"End", 0),
            KeyCode::PageUp => (c"PageUp", 0),
            KeyCode::PageDown => (c"PageDown", 0),
            KeyCode::Tab => (c"Tab", 0),
            KeyCode::BackTab => {
                key.modifiers.shift = true;
                (c"Tab", 0)
            }
            KeyCode::Delete => (c"Delete", 0),
            KeyCode::Insert => (c"Insert", 0),
            KeyCode::CapsLock => (c"CapsLock", 0),
            KeyCode::ScrollLock => (c"ScrollLock", 0),
            KeyCode::NumLock => (c"NumLock", 0),
            KeyCode::PrintScreen => (c"PrintScreen", 0),
            KeyCode::Pause => (c"Pause", 0),
            KeyCode::Menu => (c"Menu", 0),
            KeyCode::KeypadBegin => (c"KeypadBegin", 0),
            KeyCode::Media(media) => (media_name(media)?, 0),
            KeyCode::Modifier(modifier) => (modifier_name(modifier)?, 0),
            KeyCode::Null => return Err(Error::Protocol("unreachable NUL key branch".into())),
        };
        let modifiers = u32::from(key.modifiers.shift)
            | (u32::from(key.modifiers.alt) << 1)
            | (u32::from(key.modifiers.control) << 2)
            | (u32::from(key.modifiers.super_key) << 3);
        let state = u32::from(key.state.keypad)
            | (u32::from(key.state.caps_lock) << 1)
            | (u32::from(key.state.num_lock) << 2);
        let action = match key.kind {
            KeyKind::Press => 0,
            KeyKind::Repeat => 1,
            KeyKind::Release => 2,
        };
        let mut text = [0u8; 4];
        let text = match key.code {
            KeyCode::Char(ch) => ch.encode_utf8(&mut text).as_bytes(),
            _ => &[],
        };
        let mut output = [0u8; 128];
        let mut written = 0;
        // SAFETY: every pointer names live bounded storage for this synchronous
        // call. The encoder borrows text/name only until it returns and owns no
        // frontend input. Written length is validated before copying output.
        let code = unsafe {
            ffi::strop_vt_key(
                self.handle.as_ptr(),
                name.as_ptr(),
                scalar,
                modifiers,
                action,
                state,
                text.as_ptr().cast(),
                text.len(),
                output.as_mut_ptr(),
                output.len(),
                &mut written,
            )
        };
        if code == -906 {
            return Err(Error::Unavailable(
                "key has no representation in the selected native encoder".into(),
            ));
        }
        check("key", code)?;
        if written > output.len() {
            return Err(Error::Protocol("native key result exceeded buffer".into()));
        }
        let mut emission = self.take_events()?;
        emission.reply.extend_from_slice(&output[..written]);
        Ok(emission)
    }
}

fn media_name(key: MediaKey) -> Result<&'static CStr, Error> {
    match key {
        MediaKey::PlayPause => Ok(c"MediaPlayPause"),
        MediaKey::Stop => Ok(c"MediaStop"),
        MediaKey::TrackNext => Ok(c"MediaTrackNext"),
        MediaKey::TrackPrevious => Ok(c"MediaTrackPrevious"),
        MediaKey::LowerVolume => Ok(c"LowerVolume"),
        MediaKey::RaiseVolume => Ok(c"RaiseVolume"),
        MediaKey::MuteVolume => Ok(c"MuteVolume"),
        _ => Err(Error::Unavailable(
            "media key has no native terminal encoding".into(),
        )),
    }
}
fn modifier_name(key: ModifierKey) -> Result<&'static CStr, Error> {
    match key {
        ModifierKey::LeftShift => Ok(c"LeftShift"),
        ModifierKey::RightShift => Ok(c"RightShift"),
        ModifierKey::LeftControl => Ok(c"LeftControl"),
        ModifierKey::RightControl => Ok(c"RightControl"),
        ModifierKey::LeftAlt => Ok(c"LeftAlt"),
        ModifierKey::RightAlt => Ok(c"RightAlt"),
        ModifierKey::LeftSuper => Ok(c"LeftSuper"),
        ModifierKey::RightSuper => Ok(c"RightSuper"),
        _ => Err(Error::Unavailable(
            "modifier key has no native terminal encoding".into(),
        )),
    }
}
