//! Word motions: w/W b/B e/E ge/gE and cw's target — unicode-honest
//! (a motion never classifies halves of one char differently).

use strop_core::Buffer;

pub(crate) fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Word-class of a byte: WORD motions (big) only split on whitespace.
pub(crate) fn class_of(b: u8, big: bool) -> u8 {
    if big {
        u8::from(!b.is_ascii_whitespace())
    } else {
        u8::from(is_word(b))
    }
}

/// Word class of the *char* containing byte `pos`. Multibyte chars
/// classify by their decoded char (é is a word char, 🦀 is not), and
/// continuation bytes inherit their char's class — a motion that
/// classified halves of one char differently would stop mid-char and
/// hand a misaligned byte offset to ropey (the unicode `x` crash).
pub(crate) fn class_at(buf: &Buffer, pos: usize, big: bool) -> u8 {
    let b = buf.byte(pos);
    if b.is_ascii() {
        return class_of(b, big);
    }
    let mut lead = pos;
    while lead > 0 && buf.byte(lead) & 0xC0 == 0x80 {
        lead -= 1;
    }
    let len = match buf.byte(lead) {
        b if b & 0xE0 == 0xC0 => 2,
        b if b & 0xF0 == 0xE0 => 3,
        b if b & 0xF8 == 0xF0 => 4,
        _ => 1,
    };
    let bytes: Vec<u8> = (0..len).map(|i| buf.byte(lead + i)).collect();
    let ch = std::str::from_utf8(&bytes)
        .ok()
        .and_then(|s| s.chars().next());
    match (ch, big) {
        (Some(c), true) => u8::from(!c.is_whitespace()),
        (Some(c), false) => u8::from(c.is_alphanumeric() || c == '_'),
        (None, _) => 0,
    }
}

pub(crate) fn word_forward(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos >= n {
        return n;
    }
    let start_class = class_at(buf, pos, big);
    while pos < n && class_at(buf, pos, big) == start_class && !buf.byte(pos).is_ascii_whitespace()
    {
        pos += 1;
    }
    while pos < n && (buf.byte(pos).is_ascii_whitespace()) {
        pos += 1;
    }
    pos
}

pub(crate) fn word_backward(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && buf.byte(pos).is_ascii_whitespace() {
        pos -= 1;
    }
    let class = class_at(buf, pos, big);
    while pos > 0
        && !buf.byte(pos - 1).is_ascii_whitespace()
        && class_at(buf, pos - 1, big) == class
    {
        pos -= 1;
    }
    pos
}

pub(crate) fn word_end(buf: &Buffer, mut pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos + 1 >= n {
        return n.saturating_sub(1);
    }
    pos += 1;
    while pos < n && buf.byte(pos).is_ascii_whitespace() {
        pos += 1;
    }
    let class = class_at(buf, pos, big);
    while pos + 1 < n
        && !buf.byte(pos + 1).is_ascii_whitespace()
        && class_at(buf, pos + 1, big) == class
    {
        pos += 1;
    }
    pos
}

/// ge/gE: the end of the PREVIOUS word — vim never answers with the
/// word the cursor sits in, so a cursor inside a word walks to its
/// start first, then steps past.
pub(crate) fn word_end_backward(buf: &Buffer, pos: usize, big: bool) -> usize {
    if pos == 0 {
        return 0;
    }
    let mut p = pos;
    if !buf.byte(p).is_ascii_whitespace() {
        let class = class_at(buf, p, big);
        while p > 0 && !buf.byte(p - 1).is_ascii_whitespace() && class_at(buf, p - 1, big) == class
        {
            p -= 1;
        }
    }
    if p == 0 {
        return 0;
    }
    p -= 1;
    while p > 0 && buf.byte(p).is_ascii_whitespace() {
        p -= 1;
    }
    p
}

/// Is this line blank (empty or whitespace only)? Paragraph motions.
pub(crate) fn line_blank(buf: &Buffer, line: usize) -> bool {
    let (s, e) = (buf.line_start(line), buf.line_end(line));
    (s..e).all(|p| buf.byte(p).is_ascii_whitespace())
}

/// cw's target (vim): end of the word UNDER the cursor — unlike `e`,
/// never jumps to the next word when the cursor is already on a word's
/// last char. On whitespace, behaves like `e`.
pub(crate) fn change_word_end(buf: &Buffer, pos: usize, big: bool) -> usize {
    let n = buf.len_bytes();
    if pos >= n || buf.byte(pos).is_ascii_whitespace() {
        return word_end(buf, pos, big);
    }
    let class = class_at(buf, pos, big);
    let mut end = pos;
    while end + 1 < n
        && !buf.byte(end + 1).is_ascii_whitespace()
        && class_at(buf, end + 1, big) == class
    {
        end += 1;
    }
    end
}
