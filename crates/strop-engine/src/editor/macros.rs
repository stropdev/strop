//! Macros (0016 §macros): `q{reg}` records raw key events, `@{reg}`
//! replays them — replay feeds keys back through the ONE input
//! machine, so a macro is exactly as capable as hands on the keyboard.
//! Counts work (`3@a`), `@@` repeats the last replay.

use super::Editor;
use strop_core::frontend_input::Key;

impl Editor {
    /// `q{reg}`: start recording; `q` while recording stops (handled in
    /// `feed` before the machine sees it — the toggle key never
    /// records itself).
    pub(crate) fn macro_toggle(&mut self, reg: char) {
        if !(reg.is_ascii_alphanumeric() || reg == '"') {
            // (`q:` and `q/` are Vim's command-line window, not registers)
            self.message = format!("q{reg}: not a register to record into");
            return;
        }
        self.recording = Some(reg);
        self.macros.insert(reg, Vec::new());
        self.message = format!("recording @{reg}");
    }

    /// A finished recording is register content (vim: `qa` fills
    /// `"a`, so `"ap` shows the macro). The typed key vector stays the
    /// replay source until something else writes the register.
    pub(crate) fn macro_recorded(&mut self, reg: char) {
        let text: String = self
            .macros
            .get(&reg)
            .into_iter()
            .flatten()
            .filter_map(|key| key_char(*key))
            .collect();
        self.registers
            .set(reg, super::Register::characterwise(text));
    }

    /// `@{reg}`: replay count times. A guard bounds self-replay (`@a`
    /// inside @a) — vim errors at recursion, we stop at 64 deep. A
    /// register no recording owns replays its text as keys (vim
    /// executes whatever `"a` holds).
    pub(crate) fn macro_play(&mut self, reg: char, count: usize) {
        let keys = match self.macros.get(&reg) {
            Some(keys) => keys.clone(),
            None => self
                .register(Some(reg))
                .text
                .chars()
                .map(char_key)
                .collect(),
        };
        if keys.is_empty() {
            self.message = format!("register @{reg} is empty");
            return;
        }
        self.last_macro = Some(reg);
        let depth = self.macro_depth;
        if depth >= 64 {
            self.message = "macro recursion too deep".into();
            return;
        }
        if self.resolution.enabled {
            self.queue_macro(keys, count, depth + 1);
            return;
        }
        for _ in 0..count {
            for key in &keys {
                if self.should_quit {
                    return;
                }
                self.macro_depth = depth + 1;
                self.feed(*key);
                self.macro_depth = depth;
            }
        }
    }

    /// `@@`: the last replayed register (vim).
    pub(crate) fn macro_again(&mut self, count: usize) {
        match self.last_macro {
            Some(reg) => self.macro_play(reg, count),
            None => self.message = "no macro replayed yet".into(),
        }
    }
}

/// Keys that are one character of register text, vim's encoding.
const CONTROL_KEYS: &[(Key, char)] = &[
    (Key::Esc, '\x1b'),
    (Key::Enter, '\r'),
    (Key::Tab, '\t'),
    (Key::CtrlSpace, '\0'),
    (Key::CtrlA, '\x01'),
    (Key::CtrlB, '\x02'),
    (Key::CtrlC, '\x03'),
    (Key::CtrlD, '\x04'),
    (Key::CtrlE, '\x05'),
    (Key::CtrlF, '\x06'),
    (Key::CtrlL, '\x0c'),
    (Key::CtrlN, '\x0e'),
    (Key::CtrlO, '\x0f'),
    (Key::CtrlP, '\x10'),
    (Key::CtrlR, '\x12'),
    (Key::CtrlT, '\x14'),
    (Key::CtrlU, '\x15'),
    (Key::CtrlV, '\x16'),
    (Key::CtrlW, '\x17'),
    (Key::CtrlX, '\x18'),
    (Key::CtrlY, '\x19'),
    (Key::CtrlCaret, '\x1e'),
    (Key::Backspace, '\x7f'),
];

/// The register text of one recorded key; keys with no single-char
/// form (arrows, shift-tab) live only in the typed recording.
fn key_char(key: Key) -> Option<char> {
    match key {
        Key::Char(c) => Some(c),
        key => CONTROL_KEYS
            .iter()
            .find_map(|(control, c)| (*control == key).then_some(*c)),
    }
}

fn char_key(c: char) -> Key {
    CONTROL_KEYS
        .iter()
        .find_map(|(key, control)| (*control == c).then_some(*key))
        .unwrap_or(Key::Char(c))
}
