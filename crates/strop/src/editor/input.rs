//! The input walker (0008 stage 2): key EVENTS walk the command table;
//! pending state is typed, not a string.
//!
//! What dies: the pending-String puns for command STRUCTURE (counts,
//! operators, registers, prefixes). What stays textual, deliberately:
//! the free-text inputs — `:ex`, `/search`, `?search`, `|pipe` are text
//! fields with modal editing (0003 §1); they were never key sequences.
//!
//! The walker assembles operator-pending sequences from typed parts and
//! still hands a complete key string to `strop_grammar::parse` — the
//! grammar owns composition semantics; the trie owns WHEN keys complete.

/// Counts combine multiplicatively (vim: 2d3w = 6 words).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ParserState {
    /// Digits before the operator.
    pub count1: Option<usize>,
    /// `"x` — the named/system register selector.
    pub register: Option<char>,
    /// The pending operator (d/y/c/>/<), once seen.
    pub op: Option<strop_grammar::Op>,
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

    /// The count digits in vim's assembly order (for the grammar string).
    /// vim count multiplication lives in assembly (2d3w = 2d + 3w).
    fn digits(&self) -> String {
        let mut s = String::new();
        if let Some(n) = self.count1 {
            s.push_str(&n.to_string());
        }
        if self.op.is_some() {
            if let Some(n) = self.count2 {
                s.push_str(&n.to_string());
            }
        }
        s
    }

    /// Assemble the grammar string from typed parts: `"a2d3w` shaped.
    pub fn assemble(&self, motion_keys: &str) -> String {
        let mut s = String::new();
        if let Some(r) = self.register {
            s.push('"');
            s.push(r);
        }
        if let Some(n) = self.count1 {
            s.push_str(&n.to_string());
        }
        if let Some(o) = self.op {
            s.push_str(o.key());
        }
        if let Some(n) = self.count2 {
            s.push_str(&n.to_string());
        }
        s.push_str(motion_keys);
        s
    }
}

/// What feeding one key did.
#[derive(Debug, PartialEq, Eq)]
pub enum Walk {
    /// A complete command: this key string goes to the grammar/editor.
    Complete(String),
    /// `:` `/` `?` open a free-text line (modal, 0003 §1) — the text
    /// layer owns every later key.
    EnterText(char),
    /// The walker is mid-sequence (prefix, count, operator, register).
    Pending,
}

/// The normal-mode walker: typed state, table-driven.
#[derive(Debug, Default)]
pub struct Walker {
    pub state: ParserState,
    /// Active prefix path ("g", " ", " g", "m", "'", "`", "[", "]", "r").
    pub prefix: &'static str,
}

impl Walker {
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset to ground (Esc, completion, invalid).
    pub fn clear(&mut self) {
        self.state = ParserState::default();
        self.prefix = "";
    }

    /// The pending keys as a display string (statusline/which-key).
    pub fn display(&self) -> String {
        self.state.digits() + self.prefix
    }

    /// register + count1 render (prefix completions keep them).
    fn head_string(&self) -> String {
        let mut s = String::new();
        if let Some(r) = self.state.register.filter(|r| *r != '\0') {
            s.push('"');
            s.push(r);
        }
        if let Some(n) = self.state.count1 {
            s.push_str(&n.to_string());
        }
        s
    }
}

impl Walker {
    /// Feed one key. `is_free_text` says the editor is on a
    /// `:`/`/`/`?`/`|` text line — those keys never reach the walker.
    pub fn feed(&mut self, c: char) -> Walk {
        use strop_grammar::Op;
        // --- prefixes wait for their child
        if !self.prefix.is_empty() {
            let path = format!("{}{}", self.prefix, c);
            self.prefix = ""; // the caller re-arms it when the child is a prefix
            let keys = format!("{}{}", self.head_string(), path);
            self.state = ParserState::default();
            return Walk::Complete(keys);
        }
        // --- register absorb: `"x`
        if self.state.register == Some('\0') {
            self.state.register = Some(c);
            return Walk::Pending;
        }
        // --- operators absorb counts and motions
        if self.state.op.is_some() {
            if c.is_ascii_digit() {
                let d = self.state.count2.unwrap_or(0) * 10 + c.to_digit(10).unwrap() as usize;
                self.state.count2 = Some(d);
                return Walk::Pending;
            }
            // doubled operator = linewise; i/a = object prefix — both
            // complete (or continue) through the grammar either way
            let keys = self.state.assemble(&c.to_string());
            self.clear();
            return Walk::Complete(keys);
        }
        // --- counts before anything
        if c.is_ascii_digit() && (c != '0' || self.state.count1.is_some()) {
            let d = self.state.count1.unwrap_or(0) * 10 + c.to_digit(10).unwrap() as usize;
            self.state.count1 = Some(d);
            return Walk::Pending;
        }
        match c {
            '"' => {
                self.state.register = Some('\0'); // absorb next
                Walk::Pending
            }
            'g' => {
                self.prefix = "g";
                Walk::Pending
            }
            ' ' => {
                self.prefix = " ";
                Walk::Pending
            }
            'm' => {
                self.prefix = "m";
                Walk::Pending
            }
            '\'' => {
                self.prefix = "'";
                Walk::Pending
            }
            '`' => {
                self.prefix = "`";
                Walk::Pending
            }
            '[' => {
                self.prefix = "[";
                Walk::Pending
            }
            ']' => {
                self.prefix = "]";
                Walk::Pending
            }
            'r' => {
                self.prefix = "r";
                Walk::Pending
            }
            ':' | '/' | '?' => {
                self.clear();
                Walk::EnterText(c)
            }
            'd' => {
                self.state.op = Some(Op::Delete);
                Walk::Pending
            }
            'y' => {
                self.state.op = Some(Op::Yank);
                Walk::Pending
            }
            'c' => {
                self.state.op = Some(Op::Change);
                Walk::Pending
            }
            '>' => {
                self.state.op = Some(Op::Indent);
                Walk::Pending
            }
            '<' => {
                self.state.op = Some(Op::Dedent);
                Walk::Pending
            }
            _ => {
                let keys = self.state.assemble(&c.to_string());
                self.clear();
                Walk::Complete(keys)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_combine_vim_style() {
        let s = ParserState {
            count1: Some(2),
            op: Some(strop_grammar::Op::Delete),
            count2: Some(3),
            ..Default::default()
        };
        assert_eq!(s.assemble("w"), "2d3w");
    }

    #[test]
    fn register_then_count_then_op() {
        let s = ParserState {
            register: Some('a'),
            count1: Some(2),
            op: Some(strop_grammar::Op::Yank),
            ..Default::default()
        };
        assert_eq!(s.assemble("y"), "\"a2yy");
    }
}
