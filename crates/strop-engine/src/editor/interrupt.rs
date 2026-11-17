//! `Ctrl-C` for a long command on the input thread (plan 0108 H4). While
//! `:g` or a ranged `:normal` runs, no key is read, so the terminal's key
//! reader raises this flag the moment it decodes a `Ctrl-C` — one atomic
//! store, never a wait — and the command checks it between lines. The
//! `Ctrl-C` key itself still arrives as input after the command; taking
//! it lowers the flag.
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use strop_core::frontend_input::{Input, KeyCode, KeyKind};

/// The flag one key reader raises and the editor reads.
#[derive(Clone, Debug, Default)]
pub struct Interrupt(Arc<AtomicBool>);

impl Interrupt {
    /// The key reader decoded `Ctrl-C`.
    pub fn raise(&self) {
        self.0.store(true, Ordering::Release);
    }

    /// The input `Ctrl-C` the flag announced is being taken.
    pub(crate) fn take_key(&self, input: &Input) {
        if is_interrupt(input) {
            self.0.store(false, Ordering::Release);
        }
    }

    /// Whether a `Ctrl-C` is waiting behind the running command. The
    /// answer is recorded, so a replay stops where the live run did.
    pub(crate) fn raised(&self, tape: &strop_trace::replay::Tape) -> std::io::Result<bool> {
        tape.call("input.interrupt", &(), || self.0.load(Ordering::Acquire))
    }
}

/// A pressed `Ctrl-C`, the key the reader raises the flag for.
pub fn is_interrupt(input: &Input) -> bool {
    matches!(input, Input::Key(key)
        if key.kind == KeyKind::Press
            && key.modifiers.control
            && matches!(key.code, KeyCode::Char('c' | 'C')))
}
