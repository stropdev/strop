//! Macros (0016 §macros): `q{reg}` records raw key events, `@{reg}`
//! replays them — replay feeds keys back through the ONE input
//! machine, so a macro is exactly as capable as hands on the keyboard.
//! Counts work (`3@a`), `@@` repeats the last replay.

use super::Editor;

impl Editor {
    /// `q{reg}`: start recording; `q` while recording stops (handled in
    /// `feed` before the machine sees it — the toggle key never
    /// records itself).
    pub(crate) fn macro_toggle(&mut self, reg: char) {
        self.recording = Some(reg);
        self.macros.insert(reg, Vec::new());
        self.message = format!("recording @{reg}");
    }

    /// `@{reg}`: replay count times. A guard bounds self-replay (`@a`
    /// inside @a) — vim errors at recursion, we stop at 64 deep.
    pub(crate) fn macro_play(&mut self, reg: char, count: usize) {
        let Some(keys) = self.macros.get(&reg).cloned() else {
            self.message = format!("register @{reg} is empty");
            return;
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
