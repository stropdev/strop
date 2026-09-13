//! Lossless frontend adapter. Editor normalization belongs after input ownership
//! in the engine; do not expand Alt, filter releases or turn Ctrl-C into quit here.
use crossterm::event as native;
use strop_core::frontend_input::{
    KeyCode, KeyEvent, KeyKind, KeyState, MediaKey, ModifierKey, Modifiers,
};

pub(super) fn key(event: native::KeyEvent) -> KeyEvent {
    let code = match event.code {
        native::KeyCode::Backspace => KeyCode::Backspace,
        native::KeyCode::Enter => KeyCode::Enter,
        native::KeyCode::Left => KeyCode::Left,
        native::KeyCode::Right => KeyCode::Right,
        native::KeyCode::Up => KeyCode::Up,
        native::KeyCode::Down => KeyCode::Down,
        native::KeyCode::Home => KeyCode::Home,
        native::KeyCode::End => KeyCode::End,
        native::KeyCode::PageUp => KeyCode::PageUp,
        native::KeyCode::PageDown => KeyCode::PageDown,
        native::KeyCode::Tab => KeyCode::Tab,
        native::KeyCode::BackTab => KeyCode::BackTab,
        native::KeyCode::Delete => KeyCode::Delete,
        native::KeyCode::Insert => KeyCode::Insert,
        native::KeyCode::F(number) => KeyCode::Function(number),
        native::KeyCode::Char(ch) => KeyCode::Char(ch),
        native::KeyCode::Null => KeyCode::Null,
        native::KeyCode::Esc => KeyCode::Escape,
        native::KeyCode::CapsLock => KeyCode::CapsLock,
        native::KeyCode::ScrollLock => KeyCode::ScrollLock,
        native::KeyCode::NumLock => KeyCode::NumLock,
        native::KeyCode::PrintScreen => KeyCode::PrintScreen,
        native::KeyCode::Pause => KeyCode::Pause,
        native::KeyCode::Menu => KeyCode::Menu,
        native::KeyCode::KeypadBegin => KeyCode::KeypadBegin,
        native::KeyCode::Media(key) => KeyCode::Media(media(key)),
        native::KeyCode::Modifier(key) => KeyCode::Modifier(modifier(key)),
    };
    KeyEvent {
        code,
        modifiers: Modifiers {
            shift: event.modifiers.contains(native::KeyModifiers::SHIFT),
            control: event.modifiers.contains(native::KeyModifiers::CONTROL),
            alt: event.modifiers.contains(native::KeyModifiers::ALT),
            super_key: event.modifiers.contains(native::KeyModifiers::SUPER),
            hyper: event.modifiers.contains(native::KeyModifiers::HYPER),
            meta: event.modifiers.contains(native::KeyModifiers::META),
        },
        kind: match event.kind {
            native::KeyEventKind::Press => KeyKind::Press,
            native::KeyEventKind::Repeat => KeyKind::Repeat,
            native::KeyEventKind::Release => KeyKind::Release,
        },
        state: KeyState {
            keypad: event.state.contains(native::KeyEventState::KEYPAD),
            caps_lock: event.state.contains(native::KeyEventState::CAPS_LOCK),
            num_lock: event.state.contains(native::KeyEventState::NUM_LOCK),
        },
    }
}
fn media(key: native::MediaKeyCode) -> MediaKey {
    match key {
        native::MediaKeyCode::Play => MediaKey::Play,
        native::MediaKeyCode::Pause => MediaKey::Pause,
        native::MediaKeyCode::PlayPause => MediaKey::PlayPause,
        native::MediaKeyCode::Reverse => MediaKey::Reverse,
        native::MediaKeyCode::Stop => MediaKey::Stop,
        native::MediaKeyCode::FastForward => MediaKey::FastForward,
        native::MediaKeyCode::Rewind => MediaKey::Rewind,
        native::MediaKeyCode::TrackNext => MediaKey::TrackNext,
        native::MediaKeyCode::TrackPrevious => MediaKey::TrackPrevious,
        native::MediaKeyCode::Record => MediaKey::Record,
        native::MediaKeyCode::LowerVolume => MediaKey::LowerVolume,
        native::MediaKeyCode::RaiseVolume => MediaKey::RaiseVolume,
        native::MediaKeyCode::MuteVolume => MediaKey::MuteVolume,
    }
}
fn modifier(key: native::ModifierKeyCode) -> ModifierKey {
    match key {
        native::ModifierKeyCode::LeftShift => ModifierKey::LeftShift,
        native::ModifierKeyCode::LeftControl => ModifierKey::LeftControl,
        native::ModifierKeyCode::LeftAlt => ModifierKey::LeftAlt,
        native::ModifierKeyCode::LeftSuper => ModifierKey::LeftSuper,
        native::ModifierKeyCode::LeftHyper => ModifierKey::LeftHyper,
        native::ModifierKeyCode::LeftMeta => ModifierKey::LeftMeta,
        native::ModifierKeyCode::RightShift => ModifierKey::RightShift,
        native::ModifierKeyCode::RightControl => ModifierKey::RightControl,
        native::ModifierKeyCode::RightAlt => ModifierKey::RightAlt,
        native::ModifierKeyCode::RightSuper => ModifierKey::RightSuper,
        native::ModifierKeyCode::RightHyper => ModifierKey::RightHyper,
        native::ModifierKeyCode::RightMeta => ModifierKey::RightMeta,
        native::ModifierKeyCode::IsoLevel3Shift => ModifierKey::IsoLevel3Shift,
        native::ModifierKeyCode::IsoLevel5Shift => ModifierKey::IsoLevel5Shift,
    }
}
