//! Physical/logical input facts before an editor or terminal chooses semantics.
//! No toolkit types, grammar normalization, native work or implicit key expansion.
use serde::{Deserialize, Serialize};

pub mod notation;

/// Normalized editor keys for admitted semantic input. Physical frontend input
/// retains its modifiers until the engine selects its owner.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Key {
    Char(char),
    Esc,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Tab,
    Backtab,
    /// Replace picker: exclude the row's whole file (vscode's toggle).
    CtrlD,
    CtrlR,
    /// vim's jump-back (ctrl-i forward is Tab in a terminal).
    CtrlO,
    /// ctrl-space: query suggestions in a query field (0051 R02).
    CtrlSpace,
    /// Insert completion selection/acceptance; fields retain their own keys.
    CtrlN,
    CtrlP,
    CtrlY,
    CtrlE,
    CtrlW,
    /// Replace picker: exclude/include the selected match (0007 §2).
    CtrlX,
    /// vim ctrl-u/ctrl-f/ctrl-b: half/full page up.
    CtrlU,
    CtrlF,
    CtrlB,
    /// vim ctrl-^: alternate buffer.
    CtrlCaret,
    /// vim ctrl-v: visual block mode.
    CtrlV,
    /// vim ctrl-l: force a full terminal repaint (desync recovery).
    CtrlL,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyCode {
    Char(char),
    Function(u8),
    Escape,
    Enter,
    Backspace,
    Up,
    Down,
    Left,
    Right,
    Home,
    End,
    PageUp,
    PageDown,
    Tab,
    BackTab,
    Delete,
    Insert,
    Null,
    CapsLock,
    ScrollLock,
    NumLock,
    PrintScreen,
    Pause,
    Menu,
    KeypadBegin,
    Media(MediaKey),
    Modifier(ModifierKey),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum MediaKey {
    Play,
    Pause,
    PlayPause,
    Reverse,
    Stop,
    FastForward,
    Rewind,
    TrackNext,
    TrackPrevious,
    Record,
    LowerVolume,
    RaiseVolume,
    MuteVolume,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModifierKey {
    LeftShift,
    LeftControl,
    LeftAlt,
    LeftSuper,
    LeftHyper,
    LeftMeta,
    RightShift,
    RightControl,
    RightAlt,
    RightSuper,
    RightHyper,
    RightMeta,
    IsoLevel3Shift,
    IsoLevel5Shift,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Modifiers {
    pub shift: bool,
    pub control: bool,
    pub alt: bool,
    pub super_key: bool,
    pub hyper: bool,
    pub meta: bool,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum KeyKind {
    #[default]
    Press,
    Repeat,
    Release,
}

#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyState {
    pub keypad: bool,
    pub caps_lock: bool,
    pub num_lock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct KeyEvent {
    pub code: KeyCode,
    pub modifiers: Modifiers,
    pub kind: KeyKind,
    pub state: KeyState,
}

impl KeyEvent {
    pub const fn press(code: KeyCode) -> Self {
        Self {
            code,
            modifiers: Modifiers {
                shift: false,
                control: false,
                alt: false,
                super_key: false,
                hyper: false,
                meta: false,
            },
            kind: KeyKind::Press,
            state: KeyState {
                keypad: false,
                caps_lock: false,
                num_lock: false,
            },
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum Input {
    Key(KeyEvent),
    /// Committed text, not a physical-key reconstruction or IME preedit.
    Text(String),
    Paste(String),
}
