//! strop-grammar: the pure operator-pending resolver (0001 §5.2).
//!
//! One resolver, two consumers: the app executes what this resolves, the
//! renderer previews what this resolves. No UI code in here, ever.

use strop_core::{Buffer, Range};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Op {
    Delete,
    Yank,
    Change,
}

impl Op {
    pub fn name(self) -> &'static str {
        match self {
            Op::Delete => "delete",
            Op::Yank => "yank",
            Op::Change => "change",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Motion {
    Left,
    Down,
    Up,
    Right,
    WordForward,
    WordBackward,
    WordEnd,
    LineStart,
    LineEnd,
    FirstLine,
    LastLine,
    /// f/F (till=false) and t/T (till=true).
    FindChar {
        ch: u8,
        till: bool,
        backward: bool,
    },
    /// `/pat⏎` — the pattern without the terminator.
    Search(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Object {
    Word,
    Quote(u8),
    Bracket { open: u8, close: u8 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Motion(Motion),
    Object {
        inner: bool,
        obj: Object,
    },
    /// dd / yy / cc, or operator + j/k: whole lines.
    Linewise,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Command {
    pub op: Option<Op>,
    pub count: usize,
    pub target: Target,
    /// The keys that produced this command (dot-repeat, flash, spec footer).
    pub keys: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Parse {
    Incomplete,
    Invalid,
    Complete(Command),
}

/// What the resolver found: the affected bytes plus the spec-footer text.
#[derive(Debug, Clone)]
pub struct Resolved {
    pub range: Range,
    pub inclusive: bool,
    /// e.g. "inner [", "word forward", "find ':'", "search /enum", "3 lines".
    pub spec: String,
}

fn is_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// Parse accumulated pending keys. Prototype grammar: counts, d/y/c,
/// doubled ops, i/a objects, hjkl wbe 0 $ gg G, f/F/t/T<char>, /pat⏎.
pub fn parse(keys: &str) -> Parse {
    let bytes = keys.as_bytes();
    let mut i = 0;

    // vim: a count never starts with 0 — `0` is the line-start motion.
    let mut count1 = 0usize;
    if bytes.first().is_some_and(|&b| b != b'0') {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            count1 = count1 * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }
    }
    let op = match bytes.get(i) {
        Some(b'd') => {
            i += 1;
            Some(Op::Delete)
        }
        Some(b'y') => {
            i += 1;
            Some(Op::Yank)
        }
        Some(b'c') => {
            i += 1;
            Some(Op::Change)
        }
        _ => None,
    };
    let mut count2 = 0usize;
    if bytes.get(i).is_some_and(|&b| b != b'0') {
        while i < bytes.len() && bytes[i].is_ascii_digit() {
            count2 = count2 * 10 + (bytes[i] - b'0') as usize;
            i += 1;
        }
    }
    let count = (count1.max(1)) * (count2.max(1));
    let rest = &bytes[i..];

    if rest.is_empty() {
        return Parse::Incomplete;
    }

    // Doubled operator: dd / yy / cc (linewise).
    if let (Some(o), &[b]) = (op, rest) {
        let matches = matches!(
            (o, b),
            (Op::Delete, b'd') | (Op::Yank, b'y') | (Op::Change, b'c')
        );
        if matches {
            return Parse::Complete(Command {
                op,
                count,
                target: Target::Linewise,
                keys: keys.into(),
            });
        }
    }

    // Text objects: i/a + object char.
    if rest[0] == b'i' || rest[0] == b'a' {
        let inner = rest[0] == b'i';
        if rest.len() < 2 {
            return Parse::Incomplete;
        }
        let obj = match rest[1] {
            b'w' => Object::Word,
            q @ (b'"' | b'\'' | b'`') => Object::Quote(q),
            b'(' | b')' => Object::Bracket {
                open: b'(',
                close: b')',
            },
            b'[' | b']' => Object::Bracket {
                open: b'[',
                close: b']',
            },
            b'{' | b'}' => Object::Bracket {
                open: b'{',
                close: b'}',
            },
            b'<' | b'>' => Object::Bracket {
                open: b'<',
                close: b'>',
            },
            _ => return Parse::Invalid,
        };
        if rest.len() > 2 {
            return Parse::Invalid;
        }
        return Parse::Complete(Command {
            op,
            count,
            target: Target::Object { inner, obj },
            keys: keys.into(),
        });
    }

    // Search motion: /pat⏎
    if rest[0] == b'/' {
        let tail = &rest[1..];
        return match tail.iter().position(|&b| b == b'\r') {
            None => Parse::Incomplete,
            Some(end) if end == tail.len() - 1 => {
                let pat = String::from_utf8_lossy(&tail[..end]).into_owned();
                if pat.is_empty() {
                    Parse::Invalid
                } else {
                    Parse::Complete(Command {
                        op,
                        count,
                        target: Target::Motion(Motion::Search(pat)),
                        keys: keys.into(),
                    })
                }
            }
            _ => Parse::Invalid,
        };
    }

    // f/F/t/T + char.
    if matches!(rest[0], b'f' | b'F' | b't' | b'T') {
        if rest.len() < 2 {
            return Parse::Incomplete;
        }
        if rest.len() > 2 {
            return Parse::Invalid;
        }
        return Parse::Complete(Command {
            op,
            count,
            target: Target::Motion(Motion::FindChar {
                ch: rest[1],
                till: matches!(rest[0], b't' | b'T'),
                backward: matches!(rest[0], b'F' | b'T'),
            }),
            keys: keys.into(),
        });
    }

    let motion = match rest {
        b"h" => Motion::Left,
        b"j" => Motion::Down,
        b"k" => Motion::Up,
        b"l" => Motion::Right,
        b"w" => Motion::WordForward,
        b"b" => Motion::WordBackward,
        b"e" => Motion::WordEnd,
        b"0" => Motion::LineStart,
        b"$" => Motion::LineEnd,
        b"gg" => Motion::FirstLine,
        b"G" => Motion::LastLine,
        b"g" => return Parse::Incomplete,
        _ => return Parse::Invalid,
    };
    Parse::Complete(Command {
        op,
        count,
        target: Target::Motion(motion),
        keys: keys.into(),
    })
}

// ---- resolution ---------------------------------------------------------

fn word_forward(buf: &Buffer, mut pos: usize) -> usize {
    let n = buf.len_bytes();
    if pos >= n {
        return n;
    }
    let start_class = is_word(buf.byte(pos));
    while pos < n
        && is_word(buf.byte(pos)) == start_class
        && buf.byte(pos) != b'\n'
        && !buf.byte(pos).is_ascii_whitespace()
    {
        pos += 1;
    }
    while pos < n && (buf.byte(pos).is_ascii_whitespace()) {
        pos += 1;
    }
    pos
}

fn word_backward(buf: &Buffer, mut pos: usize) -> usize {
    if pos == 0 {
        return 0;
    }
    pos -= 1;
    while pos > 0 && buf.byte(pos).is_ascii_whitespace() {
        pos -= 1;
    }
    let class = is_word(buf.byte(pos));
    while pos > 0 && !buf.byte(pos - 1).is_ascii_whitespace() && is_word(buf.byte(pos - 1)) == class
    {
        pos -= 1;
    }
    pos
}

fn word_end(buf: &Buffer, mut pos: usize) -> usize {
    let n = buf.len_bytes();
    if pos + 1 >= n {
        return n.saturating_sub(1);
    }
    pos += 1;
    while pos < n && buf.byte(pos).is_ascii_whitespace() {
        pos += 1;
    }
    let class = is_word(buf.byte(pos));
    while pos + 1 < n
        && !buf.byte(pos + 1).is_ascii_whitespace()
        && is_word(buf.byte(pos + 1)) == class
    {
        pos += 1;
    }
    pos
}

/// Find the enclosing bracket pair around `pos` (nesting-aware scan).
/// Cursor on either delimiter counts as inside the pair (vim semantics):
/// the backward scan starts just inside a close, the forward scan just
/// past the open.
fn bracket_pair(buf: &Buffer, pos: usize, open: u8, close: u8) -> Option<(usize, usize)> {
    let n = buf.len_bytes();
    if n == 0 {
        return None;
    }
    let mut o = pos.min(n - 1);
    if buf.byte(o) == close && o > 0 {
        o -= 1;
    }
    // scan back for the unmatched open
    let mut depth = 0i32;
    loop {
        let b = buf.byte(o);
        if b == close {
            depth += 1;
        } else if b == open {
            if depth == 0 {
                break;
            }
            depth -= 1;
        }
        if o == 0 {
            return None;
        }
        o -= 1;
    }
    let open_pos = o;
    // scan forward from just past the open for its close
    let mut depth = 0i32;
    let mut c = open_pos + 1;
    loop {
        if c >= n {
            return None;
        }
        let b = buf.byte(c);
        if b == open {
            depth += 1;
        } else if b == close {
            if depth == 0 {
                return Some((open_pos, c));
            }
            depth -= 1;
        }
        c += 1;
    }
}

/// Quote pair on the current line (vim scans the line).
fn quote_pair(buf: &Buffer, pos: usize, q: u8) -> Option<(usize, usize)> {
    let line = buf.line_of(pos);
    let start = buf.line_start(line);
    let end = buf.line_end(line);
    let open = (start..=pos.min(end)).rev().find(|&i| buf.byte(i) == q)?;
    let close = (open + 1..end).find(|&i| buf.byte(i) == q)?;
    if pos > close {
        return None;
    }
    Some((open, close))
}

fn inner_word(buf: &Buffer, pos: usize) -> Option<(usize, usize)> {
    if pos >= buf.len_bytes() || !is_word(buf.byte(pos)) {
        return None;
    }
    let mut s = pos;
    while s > 0 && is_word(buf.byte(s - 1)) {
        s -= 1;
    }
    let mut e = pos;
    while e + 1 < buf.len_bytes() && is_word(buf.byte(e + 1)) {
        e += 1;
    }
    Some((s, e + 1)) // half-open
}

/// Search forward for `pat` (prototype: plain substring; 0001 §2.5's
/// transpiled regex lands with the real search layer).
pub fn search_forward(buf: &Buffer, from: usize, pat: &str) -> Option<usize> {
    let text = buf.rope.byte_slice(from.min(buf.len_bytes())..).to_string();
    text.find(pat).map(|i| from + i)
}

/// All matches of `pat` (incsearch highlight).
pub fn search_all(buf: &Buffer, pat: &str) -> Vec<usize> {
    if pat.is_empty() {
        return vec![];
    }
    // prototype: materializes; §2.5 promises rope-chunk search before M0 ships
    let text = buf.rope.to_string();
    text.match_indices(pat).map(|(i, _)| i).collect()
}

/// Resolve a complete command against the buffer at `cursor`.
/// This is THE function: execute and preview both consume it.
pub fn resolve(buf: &Buffer, cursor: usize, cmd: &Command) -> Option<Resolved> {
    let count = cmd.count.max(1);
    let (range, inclusive, mut spec) = match &cmd.target {
        Target::Linewise => {
            let line = buf.line_of(cursor);
            let start = buf.line_start(line);
            let end_line = (line + count).min(buf.len_lines());
            let end = if end_line >= buf.len_lines() {
                buf.len_bytes()
            } else {
                buf.line_start(end_line)
            };
            (
                Range::linewise(start, end),
                true,
                format!("{count} line{}", if count > 1 { "s" } else { "" }),
            )
        }
        Target::Object { inner, obj } => {
            let (s, e, spec) = match obj {
                Object::Word => {
                    let (s, e) = inner_word(buf, cursor)?;
                    (
                        s,
                        e,
                        if *inner {
                            "inner word".to_string()
                        } else {
                            "around word".to_string()
                        },
                    )
                }
                Object::Quote(q) => {
                    let (o, c) = quote_pair(buf, cursor, *q)?;
                    let spec =
                        format!("{} {}", if *inner { "inner" } else { "around" }, *q as char);
                    if *inner {
                        (o + 1, c, spec)
                    } else {
                        (o, c + 1, spec)
                    }
                }
                Object::Bracket { open, close } => {
                    let (o, c) = bracket_pair(buf, cursor, *open, *close)?;
                    let spec = format!(
                        "{} {}",
                        if *inner { "inner" } else { "around" },
                        *open as char
                    );
                    if *inner {
                        (o + 1, c, spec)
                    } else {
                        (o, c + 1, spec)
                    }
                }
            };
            (Range::charwise(s, e), true, spec)
        }
        Target::Motion(m) => match m {
            Motion::Left | Motion::Right => {
                let mut pos = cursor;
                for _ in 0..count {
                    pos = if *m == Motion::Left {
                        pos.saturating_sub(1)
                    } else {
                        (pos + 1).min(buf.len_bytes())
                    };
                }
                let (s, e) = if pos <= cursor {
                    (pos, cursor)
                } else {
                    (cursor, pos)
                };
                let name = if *m == Motion::Left { "left" } else { "right" };
                (Range::charwise(s, e), false, name.to_string())
            }
            Motion::Down | Motion::Up => {
                // operator + j/k is linewise in vim
                if cmd.op.is_some() {
                    let line = buf.line_of(cursor);
                    let (a, b) = if *m == Motion::Down {
                        (line, line + count)
                    } else {
                        (line.saturating_sub(count), line)
                    };
                    let start = buf.line_start(a);
                    let end = if b + 1 >= buf.len_lines() {
                        buf.len_bytes()
                    } else {
                        buf.line_start(b + 1)
                    };
                    (
                        Range::linewise(start, end),
                        true,
                        format!("{} lines", b - a + 1),
                    )
                } else {
                    let line = buf.line_of(cursor);
                    let target = if *m == Motion::Down {
                        (line + count).min(buf.len_lines() - 1)
                    } else {
                        line.saturating_sub(count)
                    };
                    let col = buf
                        .col_of(cursor)
                        .min(buf.line_end(target) - buf.line_start(target));
                    let pos = buf.line_start(target) + col;
                    (
                        Range::charwise(pos, pos),
                        false,
                        if *m == Motion::Down {
                            "down".into()
                        } else {
                            "up".into()
                        },
                    )
                }
            }
            Motion::WordForward => {
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_forward(buf, pos);
                }
                // exclusive: [cursor, target)
                (
                    Range::charwise(cursor.min(pos), pos.max(cursor)),
                    false,
                    "word forward".to_string(),
                )
            }
            Motion::WordBackward => {
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_backward(buf, pos);
                }
                (
                    Range::charwise(pos, cursor),
                    false,
                    "word backward".to_string(),
                )
            }
            Motion::WordEnd => {
                let mut pos = cursor;
                for _ in 0..count {
                    pos = word_end(buf, pos);
                }
                (
                    Range::charwise(cursor.min(pos), pos.max(cursor) + 1),
                    true,
                    "word end".to_string(),
                )
            }
            Motion::LineStart => {
                let s = buf.line_start(buf.line_of(cursor));
                (
                    Range::charwise(s.min(cursor), s.max(cursor)),
                    false,
                    "line start".to_string(),
                )
            }
            Motion::LineEnd => {
                let e = buf.line_end(buf.line_of(cursor));
                let e = e.saturating_sub(1).max(buf.line_start(buf.line_of(cursor)));
                (
                    Range::charwise(cursor.min(e), cursor.max(e) + 1),
                    true,
                    "line end".to_string(),
                )
            }
            Motion::FirstLine | Motion::LastLine => {
                let target = if *m == Motion::FirstLine {
                    count - 1
                } else {
                    buf.len_lines() - 1
                };
                let target = target.min(buf.len_lines() - 1);
                let (a, b) = if buf.line_of(cursor) <= target {
                    (buf.line_of(cursor), target)
                } else {
                    (target, buf.line_of(cursor))
                };
                let start = buf.line_start(a);
                let end = if b + 1 >= buf.len_lines() {
                    buf.len_bytes()
                } else {
                    buf.line_start(b + 1)
                };
                let name = if *m == Motion::FirstLine { "gg" } else { "G" };
                (Range::linewise(start, end), true, name.to_string())
            }
            Motion::FindChar { ch, till, backward } => {
                let line = buf.line_of(cursor);
                let (lo, hi) = (buf.line_start(line), buf.line_end(line));
                let mut found = None;
                let mut hits = 0;
                if !backward {
                    let mut i = cursor + 1;
                    while i < hi {
                        if buf.byte(i) == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                        i += 1;
                    }
                } else {
                    let mut i = cursor.min(hi);
                    while i > lo {
                        i -= 1;
                        if buf.byte(i) == *ch {
                            hits += 1;
                            if hits == count {
                                found = Some(i);
                                break;
                            }
                        }
                    }
                }
                let target = found?;
                // till lands one before/after the char
                let land = if *till {
                    if *backward {
                        target + 1
                    } else {
                        target.saturating_sub(1).max(cursor.min(target))
                    }
                } else {
                    target
                };
                let inclusive = !till;
                let (s, e) = if land >= cursor {
                    (cursor, land + 1)
                } else {
                    (land, cursor + 1)
                };
                let verb = if *till { "till" } else { "find" };
                (
                    Range::charwise(s, e),
                    inclusive,
                    format!("{verb} '{}'", *ch as char),
                )
            }
            Motion::Search(pat) => {
                let target = search_forward(buf, cursor + 1, pat)?;
                // exclusive: up to but not including the match
                (
                    Range::charwise(cursor, target),
                    false,
                    format!("search /{pat}"),
                )
            }
        },
    };
    if range.is_empty() && cmd.op.is_some() {
        return None;
    }
    if let Some(op) = cmd.op {
        spec = format!(
            "{}, {}, {} bytes · {}",
            op.name(),
            spec,
            range.len(),
            if inclusive { "inclusive" } else { "exclusive" }
        );
    } else {
        spec = format!(
            "{spec} · {}",
            if inclusive { "inclusive" } else { "exclusive" }
        );
    }
    Some(Resolved {
        range,
        inclusive,
        spec,
    })
}

/// Where the cursor lands when `cmd` is a bare motion (no operator).
pub fn cursor_after(buf: &Buffer, _cursor: usize, cmd: &Command, r: &Resolved) -> usize {
    match &cmd.target {
        Target::Motion(Motion::Down | Motion::Up) => r.range.start,
        Target::Motion(Motion::WordBackward | Motion::LineStart) => r.range.start,
        Target::Motion(Motion::WordForward) => r.range.end.min(buf.len_bytes().saturating_sub(1)),
        Target::Motion(Motion::WordEnd | Motion::LineEnd) => r.range.end.saturating_sub(1),
        Target::Motion(Motion::FirstLine | Motion::LastLine) => {
            let line = if matches!(cmd.target, Target::Motion(Motion::FirstLine)) {
                cmd.count - 1
            } else {
                buf.len_lines() - 1
            };
            buf.line_start(line.min(buf.len_lines() - 1))
        }
        Target::Motion(Motion::FindChar { backward, .. }) => {
            if *backward {
                r.range.start
            } else {
                r.range.end.saturating_sub(1)
            }
        }
        Target::Motion(Motion::Search(_)) => r.range.end,
        _ => r.range.start,
    }
}

#[cfg(test)]
pub mod tests {
    use super::*;

    const SRC: &str = "fn f(xs: &[Item]) -> Edge {\n    let edge = hone(xs);\n}\n";

    pub fn cmd(keys: &str) -> Command {
        match parse(keys) {
            Parse::Complete(c) => c,
            other => panic!("{keys} parsed as {other:?}"),
        }
    }

    fn resolve_str(buf: &Buffer, cursor: usize, keys: &str) -> String {
        let c = cmd(keys);
        let r = resolve(buf, cursor, &c).expect("resolvable");
        buf.slice_string(r.range)
    }

    #[test]
    fn bracket_object_from_inside() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find("Item").unwrap() + 2; // inside [Item]
        assert_eq!(resolve_str(&buf, cursor, "di["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_open() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find('[').unwrap();
        assert_eq!(resolve_str(&buf, cursor, "ci["), "Item");
    }

    #[test]
    fn bracket_object_cursor_on_close() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find(']').unwrap();
        assert_eq!(resolve_str(&buf, cursor, "di["), "Item");
    }

    #[test]
    fn bracket_object_around_includes_delimiters() {
        let buf = Buffer::from_text(SRC);
        let cursor = SRC.find("Item").unwrap();
        assert_eq!(resolve_str(&buf, cursor, "da["), "[Item]");
    }

    #[test]
    fn bracket_object_nested() {
        let buf = Buffer::from_text("f(a, g(b, c), d)\n");
        let cursor = 9; // 'b' — inside inner parens
        assert_eq!(resolve_str(&buf, cursor, "di("), "b, c");
        // from 'a', the enclosing pair is the outer one
        assert_eq!(resolve_str(&buf, 3, "di("), "a, g(b, c), d");
    }

    #[test]
    fn word_motions_and_objects() {
        let buf = Buffer::from_text("let edge = hone(xs);\n");
        assert_eq!(resolve_str(&buf, 0, "dw"), "let ");
        assert_eq!(resolve_str(&buf, 0, "de"), "let");
        assert_eq!(resolve_str(&buf, 4, "diw"), "edge");
    }

    #[test]
    fn doubled_operator_is_linewise() {
        let buf = Buffer::from_text(SRC);
        let r = resolve(&buf, 3, &cmd("dd")).unwrap();
        assert!(r.range.linewise);
        assert_eq!(buf.slice_string(r.range), "fn f(xs: &[Item]) -> Edge {\n");
    }

    #[test]
    fn find_and_till() {
        let buf = Buffer::from_text("edge.polish(Finish::Mirror)\n");
        assert_eq!(resolve_str(&buf, 0, "df:"), "edge.polish(Finish:");
        assert_eq!(resolve_str(&buf, 0, "dt:"), "edge.polish(Finish");
    }

    #[test]
    fn search_motion_is_exclusive() {
        let buf = Buffer::from_text(SRC);
        let c = cmd("d/hone\r");
        let r = resolve(&buf, 0, &c).unwrap();
        assert_eq!(
            buf.slice_string(r.range),
            "fn f(xs: &[Item]) -> Edge {\n    let edge = "
        );
        assert!(!r.inclusive);
    }

    #[test]
    fn counts_multiply() {
        let buf = Buffer::from_text("one two three four\n");
        assert_eq!(resolve_str(&buf, 0, "d2w"), "one two ");
    }

    #[test]
    fn spec_footer_names_the_target() {
        let buf = Buffer::from_text(SRC);
        let r = resolve(&buf, SRC.find("Item").unwrap(), &cmd("ci[")).unwrap();
        assert!(r.spec.contains("change"), "{:?}", r.spec);
        assert!(r.spec.contains("inner ["), "{:?}", r.spec);
        assert!(r.spec.contains("inclusive"), "{:?}", r.spec);
    }
}

#[cfg(test)]
mod zero_tests {
    use super::*;

    #[test]
    fn zero_is_line_start_not_count() {
        let buf = Buffer::from_text("    let edge = hone(xs);\n");
        let r = resolve(&buf, 10, &super::tests::cmd("d0")).unwrap();
        assert_eq!(buf.slice_string(r.range), "    let ed");
        // and bare 0 is a complete motion, not a pending count
        assert!(matches!(parse("0"), Parse::Complete(_)));
        // counts still parse past the rule
        assert!(matches!(parse("10dd"), Parse::Complete(_)));
    }
}
