//! Keystrokes → commands. Incremental: `parse` is called on every
//! pending-key change and answers Incomplete / Invalid / Complete.

use crate::types::*;

/// Decode the char at byte `i` (dynamic args are chars, never bytes —
/// f é must work). None past the end.
fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}

/// Digits at `*i` as a count; 0 means none. A count never starts with
/// `0` — that is the line-start motion.
fn count_at(bytes: &[u8], i: &mut usize) -> usize {
    let mut count = 0usize;
    if bytes.get(*i).is_some_and(|&b| b != b'0') {
        while *i < bytes.len() && bytes[*i].is_ascii_digit() {
            count = count
                .saturating_mul(10)
                .saturating_add((bytes[*i] - b'0') as usize);
            *i += 1;
        }
    }
    count
}

/// Sandwich aliases: b→( ) B→{ } r→[ ] a→< >.
fn surround_pair(c: char) -> Option<(char, char)> {
    match c {
        'b' | '(' | ')' => Some(('(', ')')),
        'B' | '{' | '}' => Some(('{', '}')),
        'r' | '[' | ']' => Some(('[', ']')),
        'a' | '<' | '>' => Some(('<', '>')),
        q @ ('"' | '\'' | '`') => Some((q, q)),
        _ => None,
    }
}

/// The motions that are fixed key sequences. `None` for a sequence that
/// is not a motion; `Some(None)` for a prefix of one.
fn fixed_motion(keys: &[u8]) -> Option<Option<Motion>> {
    let window = |which| Motion::Window { which, line: None };
    Some(Some(match keys {
        b"h" => Motion::Left,
        b"j" => Motion::Down,
        b"k" => Motion::Up,
        b"l" => Motion::Right,
        b"w" => Motion::WordForward,
        b"b" => Motion::WordBackward,
        b"e" => Motion::WordEnd,
        b"W" => Motion::BigWordForward,
        b"B" => Motion::BigWordBackward,
        b"E" => Motion::BigWordEnd,
        b"%" => Motion::MatchPair,
        b"0" | b"g0" => Motion::LineStart,
        b"^" | b"g^" => Motion::FirstNonBlank,
        b"_" => Motion::Underscore,
        b"$" | b"g$" => Motion::LineEnd,
        b"g_" => Motion::LastNonBlank,
        b"|" => Motion::Column,
        b"gg" => Motion::FirstLine,
        b"G" => Motion::LastLine,
        b"ge" => Motion::WordEndBackward,
        b"gE" => Motion::BigWordEndBackward,
        b"gj" => Motion::ScreenDown { column: None },
        b"gk" => Motion::ScreenUp { column: None },
        b"-" => Motion::LineUp,
        b"+" | b"\r" => Motion::LineDown,
        b"{" => Motion::ParagraphBackward,
        b"}" => Motion::ParagraphForward,
        b"(" => Motion::SentenceBackward,
        b")" => Motion::SentenceForward,
        b"[(" => Motion::UnmatchedOpen('('),
        b"[{" => Motion::UnmatchedOpen('{'),
        b"])" => Motion::UnmatchedClose('('),
        b"]}" => Motion::UnmatchedClose('{'),
        b"H" => window(WindowLine::Top),
        b"M" => window(WindowLine::Middle),
        b"L" => window(WindowLine::Bottom),
        b";" => Motion::RepeatFind { reverse: false },
        b"," => Motion::RepeatFind { reverse: true },
        b"n" => Motion::RepeatSearch { reverse: false },
        b"N" => Motion::RepeatSearch { reverse: true },
        b"g" | b"[" | b"]" => return Some(None),
        _ => return None,
    }))
}

/// Parse accumulated pending keys: `["x] [count] [operator] [count]
/// [v|V|ctrl-v] target`.
pub fn parse(keys: &str) -> Parse {
    let bytes = keys.as_bytes();
    let mut i = 0;

    // Named register prefix: "a …
    let mut register = None;
    if bytes.first() == Some(&b'"') {
        match bytes.get(1) {
            None => return Parse::Incomplete,
            // letters and digits, `+` `*` (the clipboard) and vim's
            // special registers
            Some(&r) if r.is_ascii_alphanumeric() || b"+*-_./:%#\"".contains(&r) => {
                register = Some(r as char);
                i = 2;
            }
            Some(_) => return Parse::Invalid,
        }
    }

    let count1 = count_at(bytes, &mut i);
    // Two-key operators first: `gU` must not read as a `g` motion prefix.
    let op = [2usize, 1].iter().find_map(|&len| {
        let op = Op::from_keys(keys.get(i..i + len)?)?;
        i += len;
        Some(op)
    });
    let mut count2 = count_at(bytes, &mut i);
    let mut force = None;
    if op.is_some() {
        force = match bytes.get(i) {
            Some(b'v') => Some(Force::Charwise),
            Some(b'V') => Some(Force::Linewise),
            Some(0x16) => Some(Force::Blockwise),
            _ => None,
        };
        if force.is_some() {
            i += 1;
            let count3 = count_at(bytes, &mut i);
            if count3 > 0 {
                count2 = count2.max(1).saturating_mul(count3);
            }
        }
    }
    // 0 is "no digits" (parse-level); the typed Option distinguishes
    // bare G from 1G (0016)
    let count = if count1 == 0 && count2 == 0 {
        None
    } else {
        Some(count1.max(1).saturating_mul(count2.max(1)))
    };
    let rest = &bytes[i..];
    // dynamic args (delimiters, find-chars) decode from the string, not
    // the bytes — counts/ops above are pure ASCII so i is a boundary
    let rest_str = &keys[i..];
    let complete = |target: Target| {
        Parse::Complete(Command {
            op,
            register,
            count,
            target,
            force,
            keys: keys.into(),
        })
    };

    if rest.is_empty() {
        return Parse::Incomplete;
    }

    // Surround (sandwich lineage): ds<x> / cs<x><y> / ys<motion><x>.
    if let (Some(o @ (Op::Delete | Op::Change | Op::Yank)), [b's', ..], None) = (op, rest, force) {
        let tail = &rest_str[1..];
        match o {
            Op::Delete => {
                let mut cs = tail.chars();
                let Some(ch) = cs.next() else {
                    return Parse::Incomplete;
                };
                if surround_pair(ch).is_none() || cs.next().is_some() {
                    return Parse::Invalid;
                }
                return complete(Target::SurroundDelete(ch));
            }
            Op::Change => {
                let mut cs = tail.chars();
                let (Some(from), Some(to)) = (cs.next(), cs.next()) else {
                    return Parse::Incomplete;
                };
                if surround_pair(from).is_none() || surround_pair(to).is_none() {
                    return Parse::Invalid;
                }
                if cs.next().is_some() {
                    return Parse::Invalid;
                }
                return complete(Target::SurroundChange { from, to });
            }
            _ => {
                // ys<motion><char>: the trailing char is the surround;
                // everything before it must parse as a complete motion/object
                let Some((ch, ch_len)) = tail.chars().next_back().map(|c| (c, c.len_utf8())) else {
                    return Parse::Incomplete;
                };
                let motion_keys = &tail[..tail.len() - ch_len];
                if motion_keys.is_empty() || surround_pair(ch).is_none() {
                    return Parse::Incomplete;
                }
                return match parse(&format!("y{motion_keys}")) {
                    Parse::Complete(sub) => complete(Target::SurroundAdd {
                        ch,
                        inner: Box::new(sub.target),
                    }),
                    other => other,
                };
            }
        }
    }

    // Doubled operator: dd / yy / cc / >> / g?? / gUU / gUgU (linewise).
    if let Some(o) = op {
        let key = o.key();
        if rest_str == key || (key.len() == 2 && rest_str == &key[1..]) {
            return complete(Target::Linewise);
        }
    }

    // Text objects: i/a + object char.
    if rest[0] == b'i' || rest[0] == b'a' {
        let inner = rest[0] == b'i';
        let Some(objc) = char_at(rest_str, 1) else {
            return Parse::Incomplete;
        };
        let bracket = |open, close| Object::Bracket { open, close };
        let obj = match objc {
            'w' => Object::Word,
            'W' => Object::BigWord,
            'p' => Object::Paragraph,
            't' => Object::Tag,
            q @ ('"' | '\'' | '`') => Object::Quote(q),
            '(' | ')' | 'b' => bracket('(', ')'),
            '[' | ']' => bracket('[', ']'),
            '{' | '}' | 'B' => bracket('{', '}'),
            '<' | '>' => bracket('<', '>'),
            _ => return Parse::Invalid,
        };
        if rest_str[1 + objc.len_utf8()..].chars().next().is_some() {
            return Parse::Invalid;
        }
        return complete(Target::Object { inner, obj });
    }

    // Search motions: /pat⏎ forward, ?pat⏎ backward
    if rest[0] == b'/' || rest[0] == b'?' {
        let backward = rest[0] == b'?';
        let tail = &rest[1..];
        return match tail.iter().position(|&b| b == b'\r') {
            None => Parse::Incomplete,
            Some(end) if end == tail.len() - 1 => {
                let pat = String::from_utf8_lossy(&tail[..end]).into_owned();
                if pat.is_empty() {
                    return Parse::Invalid;
                }
                match crate::CompiledQuery::compile(&pat, false) {
                    Err(e) => Parse::QueryError(e),
                    Ok(q) => complete(Target::Motion(if backward {
                        Motion::SearchBackward(q)
                    } else {
                        Motion::Search(q)
                    })),
                }
            }
            _ => Parse::Invalid,
        };
    }

    // One-argument motions: f/F/t/T + char, '/` + mark.
    if matches!(rest[0], b'f' | b'F' | b't' | b'T' | b'\'' | b'`') {
        let Some(ch) = char_at(rest_str, 1) else {
            return Parse::Incomplete;
        };
        if rest_str[1 + ch.len_utf8()..].chars().next().is_some() {
            return Parse::Invalid;
        }
        return complete(Target::Motion(match rest[0] {
            b'\'' | b'`' => Motion::Mark {
                name: ch,
                exact: rest[0] == b'`',
                at: None,
            },
            key => Motion::FindChar {
                ch,
                till: matches!(key, b't' | b'T'),
                backward: matches!(key, b'F' | b'T'),
            },
        }));
    }

    match rest {
        b"gn" | b"gN" => {
            return complete(Target::Match {
                backward: rest == b"gN",
                query: None,
            })
        }
        _ => {}
    }
    match fixed_motion(rest) {
        Some(Some(motion)) => complete(Target::Motion(motion)),
        Some(None) => Parse::Incomplete,
        None => Parse::Invalid,
    }
}
