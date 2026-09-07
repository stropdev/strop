use super::*;

impl Parser<'_> {
    /// `[...]` — `self.at` is on the `[`.
    pub(super) fn parse_class(&mut self, at: usize) -> Result<Node, QueryError> {
        self.at += 1;
        let mut set = CharSet::new();
        if self.peek() == Some(b'^') {
            set.negated = true;
            self.at += 1;
        }
        let mut first = true;
        loop {
            let c = self.peek().ok_or(QueryError::UnclosedClass { at })?;
            if c == b']' && !first {
                self.at += 1;
                break;
            }
            first = false;
            // POSIX class [:name:]
            if c == b'[' && self.pat[self.at..].starts_with(b"[:") {
                let rest = &self.pat[self.at..];
                let Some(end) = find_sub(rest, b":]") else {
                    return Err(QueryError::BadClass { at });
                };
                let mask = posix_mask(&rest[2..end]).ok_or(QueryError::BadClass { at })?;
                set.ascii |= mask;
                set.empty = false;
                self.at += end + 2;
                continue;
            }
            // `&&` intersection: typed refusal (vim-only, rare)
            if c == b'&' && self.pat[self.at..].starts_with(b"&&") {
                return Err(QueryError::Unsupported {
                    construct: "[a&&b] class intersection",
                    at,
                });
            }
            let lo = self.class_char(at)?;
            // range?
            if self.peek() == Some(b'-') && self.pat.get(self.at + 1).copied() != Some(b']') {
                if self.pat.get(self.at + 1).is_none() {
                    return Err(QueryError::UnclosedClass { at });
                }
                self.at += 1; // the '-'
                let hi = self.class_char(at)?;
                let (Some(lo), Some(hi)) = (char::from_u32(lo), char::from_u32(hi)) else {
                    return Err(QueryError::BadClass { at });
                };
                if lo > hi {
                    return Err(QueryError::BadClass { at });
                }
                add_range(&mut set, lo, hi);
                continue;
            }
            if lo < 128 {
                set.ascii |= 1u128 << lo;
            } else if let Some(ch) = char::from_u32(lo) {
                set.unicode_ranges.push(ch..=ch);
            }
            set.empty = false;
        }
        Ok(Node::Set(set))
    }

    /// One collection member: a char (UTF-8) or a control escape.
    pub(super) fn class_char(&mut self, at: usize) -> Result<u32, QueryError> {
        let c = self.peek().ok_or(QueryError::UnclosedClass { at })?;
        if c == b'\\' {
            let n = self.peek2().ok_or(QueryError::TrailingBackslash { at })?;
            self.at += 2;
            return Ok(match n {
                b'n' => b'\n' as u32,
                b'r' => b'\r' as u32,
                b't' => b'\t' as u32,
                b'e' => 0x1b,
                b'b' => 0x08,
                b'f' => 0x0c,
                other => other as u32,
            });
        }
        let ch = self
            .char_at(self.at)
            .map_err(|_| QueryError::BadClass { at })?;
        self.at += ch.len_utf8();
        Ok(ch as u32)
    }
}

fn add_range(set: &mut CharSet, lo: char, hi: char) {
    set.empty = false;
    for byte in (lo as u32)..=(hi as u32).min(127) {
        set.ascii |= 1u128 << byte;
    }
    if hi >= '\u{80}' {
        set.unicode_ranges.push(lo.max('\u{80}')..=hi);
    }
}
// ---------------------------------------------------------------- classes

/// ASCII classes exactly as vim defines them (`\d` is [0-9], not
/// Unicode digits; the corpus pins this). Note the split vim itself
/// has: `\w` is ASCII here while `\<`/`\>` boundaries use the
/// editor's char-classified word model.
pub(super) fn letter_class(c: u8) -> Option<fn() -> CharSet> {
    let f: fn() -> CharSet = match c {
        b'd' => set_digits,
        b'D' => || negate(set_digits()),
        b's' => set_space,
        b'S' => || negate(set_space()),
        b'w' => set_word,
        b'W' => || negate(set_word()),
        b'a' => set_alpha,
        b'A' => || negate(set_alpha()),
        b'l' => set_lower,
        b'L' => || negate(set_lower()),
        b'u' => set_upper,
        b'U' => || negate(set_upper()),
        b'x' => set_hex,
        b'X' => || negate(set_hex()),
        b'o' => set_octal,
        b'O' => || negate(set_octal()),
        b'h' => set_head,
        b'H' => || negate(set_head()),
        _ => return None,
    };
    Some(f)
}
fn set_digits() -> CharSet {
    range_set(b'0', b'9')
}
fn set_space() -> CharSet {
    let mut s = range_set(b' ', b' ');
    s.ascii |= 1u128 << (b'\t' as u32);
    s
}
/// Vim's \w is ASCII [0-9A-Za-z_].
fn set_word() -> CharSet {
    let mut s = set_alnum();
    s.ascii |= 1u128 << (b'_' as u32);
    s
}
fn set_alpha() -> CharSet {
    let mut s = range_set(b'a', b'z');
    s.ascii |= range_set(b'A', b'Z').ascii;
    s
}
fn set_lower() -> CharSet {
    range_set(b'a', b'z')
}
fn set_upper() -> CharSet {
    range_set(b'A', b'Z')
}
fn set_hex() -> CharSet {
    let mut s = range_set(b'0', b'9');
    s.ascii |= range_set(b'a', b'f').ascii;
    s.ascii |= range_set(b'A', b'F').ascii;
    s
}
fn set_octal() -> CharSet {
    range_set(b'0', b'7')
}
fn set_head() -> CharSet {
    let mut s = set_alpha();
    s.ascii |= 1u128 << (b'_' as u32);
    s
}
fn set_alnum() -> CharSet {
    let mut s = set_alpha();
    s.ascii |= range_set(b'0', b'9').ascii;
    s
}

fn negate(mut s: CharSet) -> CharSet {
    s.negated = !s.negated;
    s
}

fn range_set(lo: u8, hi: u8) -> CharSet {
    let mut s = CharSet::new();
    for b in lo..=hi {
        s.ascii |= 1u128 << (b as u32);
    }
    s.empty = false;
    s
}

fn posix_mask(name: &[u8]) -> Option<u128> {
    let s = match name {
        b"alpha" => set_alpha(),
        b"digit" => set_digits(),
        b"lower" => set_lower(),
        b"upper" => set_upper(),
        b"space" => {
            let mut s = set_space();
            for b in [b'\n', b'\r', 0x0b, 0x0c] {
                s.ascii |= 1u128 << (b as u32);
            }
            s
        }
        b"blank" => {
            let mut s = range_set(b' ', b' ');
            s.ascii |= 1u128 << (b'\t' as u32);
            s
        }
        b"punct" => {
            let mut s = CharSet::new();
            for b in 0x21u8..=0x7e {
                if !b.is_ascii_alphanumeric() {
                    s.ascii |= 1u128 << (b as u32);
                }
            }
            s
        }
        b"alnum" => set_alnum(),
        b"xdigit" => set_hex(),
        b"cntrl" => {
            let mut s = CharSet::new();
            for b in 0u8..=0x1f {
                s.ascii |= 1u128 << (b as u32);
            }
            s.ascii |= 1u128 << 0x7f;
            s
        }
        b"graph" => {
            let mut s = CharSet::new();
            for b in 0x21u8..=0x7e {
                s.ascii |= 1u128 << (b as u32);
            }
            s
        }
        b"print" => {
            let mut s = CharSet::new();
            for b in 0x20u8..=0x7e {
                s.ascii |= 1u128 << (b as u32);
            }
            s
        }
        b"word" => set_word(),
        _ => return None,
    };
    Some(s.ascii)
}
