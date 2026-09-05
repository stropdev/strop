//! The input machine (0016): one typed state machine owns every
//! normal-mode key event. The command table (BINDINGS) IS the trie —
//! prefixes derive from row sequences, never a hardcoded list — and a
//! completed walk yields a typed `Action`: a table row with
//! count/register attached, or a grammar `Command` built from typed
//! parser state. Strings never cross the dispatch boundary.
//!
//! What stays textual, deliberately: the free-text inputs — `:ex`,
//! `/search`, `?search`, `|pipe` are modal text fields (0003 §1); they
//! were never key sequences.

use strop_grammar::{Command, Op};

use crate::editor::Key;
use crate::keymap::{self, AbsorbKind, Binding, Handler};

/// Counts combine multiplicatively (vim: 2d3w = 6 words).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParserState {
    /// Digits before the operator.
    pub count1: Option<usize>,
    /// `"x` — the named/system register selector.
    pub register: Option<char>,
    /// The pending operator (d/y/c/>/<), once seen.
    pub op: Option<Op>,
    /// Digits between operator and motion.
    pub count2: Option<usize>,
}

impl ParserState {
    pub fn empty(&self) -> bool {
        self.count1.is_none()
            && self.register.is_none()
            && self.op.is_none()
            && self.count2.is_none()
    }

    /// vim's count multiplication: 2d3w = count1 × count2.
    pub fn count(&self) -> Option<usize> {
        match (self.count1, self.count2) {
            (None, None) => None,
            (a, b) => Some(a.unwrap_or(1) * b.unwrap_or(1)),
        }
    }
}

/// What a completed walk yields — typed, never a key string (0016 §1).
pub enum Action {
    /// A table row: leaf handler, alias, absorb — with the walker's
    /// count/register (and any absorbed argument char) attached.
    Row {
        row: &'static Binding,
        count: Option<usize>,
        register: Option<char>,
        arg: Option<char>,
        /// The key that completed the sequence (J vs ., p vs P).
        key: char,
    },
    /// Operator composition resolved by the grammar from typed state
    /// (op/register/counts injected; only the motion text re-parses).
    Grammar(Box<Command>),
    /// A free-text line opens (: / ? |) — the text layer owns later keys.
    EnterText(char),
    /// Dead sequence: neither table nor grammar — the editor says so
    /// (the unknown-key marker is a contract, not silence).
    Invalid(String),
    /// Mid-sequence: prefix, count, operator, register, absorber.
    Pending,
}

/// The machine: typed parser state + trie position.
#[derive(Debug, Default)]
pub struct Walker {
    pub state: ParserState,
    /// Token path so far (trie position), e.g. ["g"] or ["ctrl-w"].
    path: Vec<String>,
    /// Tokens collected while an operator is pending (the motion text
    /// the grammar completes, e.g. "g"+"e" or "i"+"w").
    motion: String,
}

impl Walker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to ground (Esc, completion, invalid).
    pub fn clear(&mut self) {
        self.state = ParserState::default();
        self.path.clear();
        self.motion.clear();
    }

    /// Checked digit accumulation: counts cap instead of wrapping or
    /// panicking (0015 — adversarial input is "99999…" forever).
    pub const MAX_COUNT: usize = 99_999;

    fn push_digit(count: Option<usize>, c: char) -> usize {
        let d = c.to_digit(10).map(|d| d as usize).unwrap_or(0);
        count
            .unwrap_or(0)
            .checked_mul(10)
            .and_then(|n| n.checked_add(d))
            .unwrap_or(Self::MAX_COUNT)
            .min(Self::MAX_COUNT)
    }

    /// The trie position as the which-key renderer's prefix string
    /// (" g" style — the table's token vocabulary mapped back).
    pub fn prefix_display(&self) -> String {
        let mut s = String::new();
        for t in &self.path {
            s.push_str(if t == "space" { " " } else { t });
        }
        s
    }

    /// The pending state as a display string (statusline/which-key):
    /// the FULL structural state — register, counts, operator, path.
    pub fn display(&self) -> String {
        let mut s = String::new();
        if let Some(r) = self.state.register {
            s.push('"');
            s.push(r);
        }
        if let Some(n) = self.state.count1 {
            s.push_str(&n.to_string());
        }
        if let Some(o) = self.state.op {
            s.push_str(o.key());
        }
        if let Some(n) = self.state.count2 {
            s.push_str(&n.to_string());
        }
        for t in &self.path {
            s.push_str(t);
        }
        s.push_str(&self.motion);
        s
    }

    /// One key event in, one typed step out.
    pub fn feed(&mut self, key: Key) -> Action {
        let token = key_token(key);

        // --- register selector absorbs one char — at GROUND only
        // (f" finds a quote; d" is the quote object; only a bare "
        // selects a register)
        if token == "\"" && self.state.op.is_none() && self.path.is_empty() {
            self.state.register = Some('\0');
            return Action::Pending;
        }
        if self.state.register == Some('\0') {
            if let Key::Char(c) = key {
                self.state.register = Some(c);
                self.path.clear();
                return Action::Pending;
            }
            self.clear();
            return Action::Pending;
        }

        // --- operator pending: digits extend count2 (contextual zero),
        // everything else is motion text for the grammar
        if self.state.op.is_some() {
            if let Key::Char(c) = key {
                if c.is_ascii_digit() && (c != '0' || self.state.count2.is_some()) {
                    self.state.count2 = Some(Self::push_digit(self.state.count2, c));
                    return Action::Pending;
                }
            }
            self.motion.push_str(&token);
            return match strop_grammar::parse(&self.op_string()) {
                strop_grammar::Parse::Complete(mut cmd) => {
                    cmd.register = self.state.register.filter(|r| *r != '\0');
                    cmd.count = self.state.count();
                    self.clear();
                    Action::Grammar(Box::new(cmd))
                }
                strop_grammar::Parse::Incomplete => Action::Pending,
                strop_grammar::Parse::Invalid => {
                    self.clear();
                    Action::Pending
                }
            };
        }

        // --- ground state: counts before anything (contextual zero)
        if let Key::Char(c) = key {
            if c.is_ascii_digit() && (c != '0' || self.state.count1.is_some()) {
                self.state.count1 = Some(Self::push_digit(self.state.count1, c));
                return Action::Pending;
            }
        }

        // --- the trie: extend the path, consult the table
        self.path.push(token);
        let path_str = self.path.concat();
        if let Some(row) = keymap::find_row(&self.path) {
            let row: &'static Binding = row;
            match row.handler {
                Handler::Prefix => Action::Pending,
                Handler::Operator => {
                    // typed op-pending (0016): the operator is machine
                    // state; the motion collects as text for the grammar
                    let c = path_str.chars().last().unwrap_or('\0');
                    if let Some(op) = Op::from_key(c) {
                        self.state.op = Some(op);
                        self.path.clear();
                        self.motion.clear();
                    } else {
                        self.clear();
                    }
                    Action::Pending
                }
                Handler::Motion | Handler::ObjectPrefix => {
                    // single-key grammar atoms ride the grammar too —
                    // the machine's output is always typed
                    match strop_grammar::parse(&path_str) {
                        strop_grammar::Parse::Complete(mut cmd) => {
                            cmd.register = self.state.register;
                            cmd.count = self.state.count();
                            self.clear();
                            Action::Grammar(Box::new(cmd))
                        }
                        strop_grammar::Parse::Incomplete => Action::Pending,
                        strop_grammar::Parse::Invalid => {
                            self.clear();
                            Action::Pending
                        }
                    }
                }
                Handler::AbsorbChar(AbsorbKind::Find) => {
                    // f<c> family: grammar resolves the completed find
                    match strop_grammar::parse(&path_str) {
                        strop_grammar::Parse::Complete(mut cmd) => {
                            cmd.register = self.state.register;
                            cmd.count = self.state.count();
                            self.clear();
                            Action::Grammar(Box::new(cmd))
                        }
                        _ => {
                            self.clear();
                            Action::Pending
                        }
                    }
                }
                Handler::AbsorbChar(_) => {
                    // the placeholder matched this key — it IS the arg
                    let count = self.state.count();
                    let register = self.state.register;
                    let c = self
                        .path
                        .last()
                        .and_then(|t| t.chars().last())
                        .unwrap_or('\0');
                    self.clear();
                    Action::Row {
                        row,
                        count,
                        register,
                        arg: Some(c),
                        key: c,
                    }
                }
                Handler::TextLine => {
                    self.clear();
                    Action::EnterText(path_str.chars().next().unwrap_or(':'))
                }
                Handler::Alias(_) | Handler::Leaf(_) => {
                    let count = self.state.count();
                    let register = self.state.register;
                    let key = self
                        .path
                        .last()
                        .and_then(|t| t.chars().last())
                        .unwrap_or('\0');
                    self.path.clear();
                    self.state = ParserState::default();
                    Action::Row {
                        row,
                        count,
                        register,
                        arg: None,
                        key,
                    }
                }
                Handler::AbsorbRegister | Handler::Soon => {
                    self.clear();
                    Action::Pending
                }
            }
        } else if keymap::any_child(&self.path) {
            Action::Pending
        } else {
            // not in the table: maybe a bare grammar atom (motions that
            // need no row), else a dead key — say so (0016)
            match strop_grammar::parse(&path_str) {
                strop_grammar::Parse::Complete(mut cmd) => {
                    cmd.register = self.state.register;
                    cmd.count = self.state.count();
                    self.clear();
                    Action::Grammar(Box::new(cmd))
                }
                strop_grammar::Parse::Incomplete => Action::Pending,
                strop_grammar::Parse::Invalid => {
                    self.clear();
                    Action::Invalid(path_str)
                }
            }
        }
    }

    /// The motion string as the grammar expects it (op + motion text;
    /// counts/registers inject typed afterwards).
    fn op_string(&self) -> String {
        let mut s = String::new();
        if let Some(o) = self.state.op {
            s.push_str(o.key());
        }
        s.push_str(&self.motion);
        s
    }
}

/// Key events → table tokens (named keys match the table's vocabulary).
fn key_token(key: Key) -> String {
    match key {
        Key::Char(' ') => "space".into(), // the table's leader token
        Key::Char(c) => c.to_string(),
        Key::Esc => "esc".into(),
        Key::Enter => "enter".into(),
        Key::Backspace => "backspace".into(),
        // arrows speak hjkl at the key layer — counts compose through
        // the machine exactly like the letters (0015)
        Key::Up => "k".into(),
        Key::Down => "j".into(),
        Key::Left => "h".into(),
        Key::Right => "l".into(),
        Key::Tab => "tab".into(),
        Key::Backtab => "s-tab".into(),
        Key::CtrlD => "ctrl-d".into(),
        Key::CtrlR => "ctrl-r".into(),
        Key::CtrlO => "ctrl-o".into(),
        Key::CtrlW => "ctrl-w".into(),
        Key::CtrlX => "ctrl-x".into(),
        Key::CtrlU => "ctrl-u".into(),
        Key::CtrlF => "ctrl-f".into(),
        Key::CtrlB => "ctrl-b".into(),
        Key::CtrlCaret => "ctrl-^".into(),
        Key::CtrlV => "ctrl-v".into(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row_events(keys: &str) -> Vec<Vec<Key>> {
        crate::keymap::expand(keys)
            .iter()
            .map(|seq| {
                let mut out = Vec::new();
                for t in seq {
                    match *t {
                        "space" => out.push(Key::Char(' ')),
                        "ctrl-w" => out.push(Key::CtrlW),
                        "ctrl-o" => out.push(Key::CtrlO),
                        "ctrl-r" => out.push(Key::CtrlR),
                        "ctrl-d" => out.push(Key::CtrlD),
                        "ctrl-u" => out.push(Key::CtrlU),
                        "ctrl-f" => out.push(Key::CtrlF),
                        "ctrl-b" => out.push(Key::CtrlB),
                        "ctrl-^" => out.push(Key::CtrlCaret),
                        "ctrl-v" => out.push(Key::CtrlV),
                        "tab" => out.push(Key::Tab),
                        "enter" => out.push(Key::Enter),
                        t if t.contains('<') => {
                            let i = t.find('<').unwrap();
                            for c in t[..i].chars() {
                                out.push(Key::Char(c));
                            }
                            out.push(Key::Char('x'));
                        }
                        t if t.starts_with(':') || t == "/" || t == "?" => {
                            out.push(Key::Char(t.chars().next().unwrap()))
                        }
                        t => out.extend(t.chars().map(Key::Char)),
                    }
                }
                out
            })
            .collect()
    }

    #[test]
    fn zero_is_a_motion_when_no_count2_pending() {
        // vim: d0 deletes to column 0; d10w counts through the 0 (0015)
        let mut w = Walker::new();
        assert!(matches!(w.feed(Key::Char('d')), Action::Pending));
        match w.feed(Key::Char('0')) {
            Action::Grammar(cmd) => {
                assert_eq!(cmd.op, Some(strop_grammar::Op::Delete));
                assert!(matches!(
                    cmd.target,
                    strop_grammar::Target::Motion(strop_grammar::Motion::LineStart)
                ));
            }
            _ => panic!("d0 must complete as a grammar command"),
        }
        let mut w = Walker::new();
        w.feed(Key::Char('d'));
        w.feed(Key::Char('1'));
        assert!(matches!(w.feed(Key::Char('0')), Action::Pending));
        match w.feed(Key::Char('w')) {
            Action::Grammar(cmd) => assert_eq!(cmd.count, Some(10)),
            _ => panic!("d10w keeps its count"),
        }
    }

    #[test]
    fn counts_cap_under_adversarial_input() {
        let mut w = Walker::new();
        for _ in 0..50 {
            w.feed(Key::Char('9'));
        }
        assert_eq!(w.state.count1, Some(Walker::MAX_COUNT));
    }

    #[test]
    fn display_shows_the_full_state() {
        // a pending operator is never invisible (0015)
        let mut w = Walker::new();
        w.feed(Key::Char('"'));
        w.feed(Key::Char('a'));
        w.feed(Key::Char('2'));
        w.feed(Key::Char('d'));
        w.feed(Key::Char('3'));
        assert_eq!(w.display(), "\"a2d3");
    }

    /// 0016's core invariant: every live row's sequence walks the
    /// machine to a typed action — never Invalid, and the machine
    /// grounds afterwards (no state leaks into the next command).
    #[test]
    fn every_row_completes_and_grounds() {
        for b in crate::keymap::BINDINGS
            .iter()
            .filter(|b| b.live && !matches!(b.handler, crate::keymap::Handler::Soon))
        {
            // Soon rows are surface-only verbs — they dispatch in the
            // readonly layer, not on plain buffers
            for events in row_events(b.keys) {
                if events.is_empty() {
                    continue;
                }
                let mut w = Walker::new();
                let mut last = Action::Pending;
                for key in &events {
                    last = w.feed(*key);
                    if matches!(last, Action::Pending) {
                        continue;
                    }
                    break;
                }
                assert!(
                    !matches!(last, Action::Invalid(_)),
                    "{} walked to Invalid",
                    b.keys
                );
                // a completed walk grounds; a legitimately pending one
                // (operator waits for its motion) is ALWAYS visible
                if matches!(last, Action::Pending) {
                    assert!(
                        !w.display().is_empty(),
                        "{} pends invisibly — the statusline must show it",
                        b.keys
                    );
                } else {
                    assert!(
                        w.display().is_empty(),
                        "{} left walker state {:?}",
                        b.keys,
                        w.display()
                    );
                }
            }
        }
    }

    /// Adversarial streams: junk at ground never poisons the next
    /// command's count (the 2zx class from the second review).
    #[test]
    fn junk_never_doubles_a_count() {
        for j in ['=', '\\', ',', ';'] {
            let mut w = Walker::new();
            let _ = w.feed(Key::Char('2'));
            let _ = w.feed(Key::Char(j));
            if let Action::Row { count, .. } = w.feed(Key::Char('x')) {
                assert_ne!(count, Some(4), "2{j}x must not double-count");
            }
        }
    }
}
