//! Keystrokes → commands. Incremental: `parse` is called on every
//! pending-key change and answers Incomplete / Invalid / Complete.

use crate::types::*;

/// Decode the char at byte `i` (dynamic args are chars, never bytes —
/// f é must work). None past the end.
fn char_at(s: &str, i: usize) -> Option<char> {
    s.get(i..)?.chars().next()
}

/// Parse accumulated pending keys. Prototype grammar: registers, counts,
/// d/y/c, doubled ops, i/a objects, hjkl wbeWBE 0 $ gg G %, f/F/t/T<char>, /pat⏎.
pub fn parse(keys: &str) -> Parse {
    let bytes = keys.as_bytes();
    let mut i = 0;

    // Named register prefix: "a …
    let mut register = None;
    if bytes.first() == Some(&b'"') {
        match bytes.get(1) {
            None => return Parse::Incomplete,
            // `+` is vim's system-clipboard register
            Some(&r) if r.is_ascii_alphanumeric() || r == b'+' => {
                register = Some(r as char);
                i = 2;
            }
            Some(_) => return Parse::Invalid,
        }
    }

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
        Some(b'>') => {
            i += 1;
            Some(Op::Indent)
        }
        Some(b'<') => {
            i += 1;
            Some(Op::Dedent)
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
    // 0 is "no digits" (parse-level); the typed Option distinguishes
    // bare G from 1G (0016)
    let count = if count1 == 0 && count2 == 0 {
        None
    } else {
        Some(count1.max(1) * count2.max(1))
    };
    let rest = &bytes[i..];
    // dynamic args (delimiters, find-chars) decode from the string, not
    // the bytes — counts/ops above are pure ASCII so i is a boundary
    let rest_str = &keys[i..];

    if rest.is_empty() {
        return Parse::Incomplete;
    }

    // Surround (sandwich lineage): ds<x> / cs<x><y> / ys<motion><x>.
    // Aliases: b→( ) B→{ } r→[ ] a→< >.
    if let (Some(o), [b's', ..]) = (op, rest) {
        let tail = &rest_str[1..];
        let map = |c: char| match c {
            'b' | '(' | ')' => Some(('(', ')')),
            'B' | '{' | '}' => Some(('{', '}')),
            'r' | '[' | ']' => Some(('[', ']')),
            'a' | '<' | '>' => Some(('<', '>')),
            q @ ('"' | '\'' | '`') => Some((q, q)),
            _ => None,
        };
        match o {
            Op::Delete => {
                let mut cs = tail.chars();
                let Some(ch) = cs.next() else {
                    return Parse::Incomplete;
                };
                let Some(_) = map(ch) else {
                    return Parse::Invalid;
                };
                if cs.next().is_some() {
                    return Parse::Invalid;
                }
                return Parse::Complete(Command {
                    op,
                    register,
                    count,
                    target: Target::SurroundDelete(ch),
                    keys: keys.into(),
                });
            }
            Op::Change => {
                let mut cs = tail.chars();
                let (Some(from), Some(to)) = (cs.next(), cs.next()) else {
                    return Parse::Incomplete;
                };
                let (Some(_), Some(_)) = (map(from), map(to)) else {
                    return Parse::Invalid;
                };
                if cs.next().is_some() {
                    return Parse::Invalid;
                }
                return Parse::Complete(Command {
                    op,
                    register,
                    count,
                    target: Target::SurroundChange { from, to },
                    keys: keys.into(),
                });
            }
            Op::Yank => {
                // ys<motion><char>: the trailing char is the surround;
                // everything before it must parse as a complete motion/object
                let Some((ch, ch_len)) = tail.chars().next_back().map(|c| (c, c.len_utf8())) else {
                    return Parse::Incomplete;
                };
                let motion_keys = &tail[..tail.len() - ch_len];
                if motion_keys.is_empty() {
                    return Parse::Incomplete;
                }
                let Some(_) = map(ch) else {
                    return Parse::Incomplete;
                };
                let motion_str = format!("y{motion_keys}");
                match parse(&motion_str) {
                    Parse::Complete(sub) => {
                        return Parse::Complete(Command {
                            op,
                            register,
                            count,
                            target: Target::SurroundAdd {
                                ch,
                                inner: Box::new(sub.target),
                            },
                            keys: keys.into(),
                        });
                    }
                    Parse::Incomplete => return Parse::Incomplete,
                    Parse::Invalid => return Parse::Invalid,
                }
            }
            _ => {}
        }
    }

    // Doubled operator: dd / yy / cc (linewise).
    if let (Some(o), &[b]) = (op, rest) {
        let matches = matches!(
            (o, b),
            (Op::Delete, b'd')
                | (Op::Yank, b'y')
                | (Op::Change, b'c')
                | (Op::Indent, b'>')
                | (Op::Dedent, b'<')
        );
        if matches {
            return Parse::Complete(Command {
                op,
                register,
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
        let Some(objc) = char_at(rest_str, 1) else {
            return Parse::Incomplete;
        };
        let obj = match objc {
            'w' => Object::Word,
            q @ ('"' | '\'' | '`') => Object::Quote(q),
            '(' | ')' => Object::Bracket {
                open: '(',
                close: ')',
            },
            '[' | ']' => Object::Bracket {
                open: '[',
                close: ']',
            },
            '{' | '}' => Object::Bracket {
                open: '{',
                close: '}',
            },
            '<' | '>' => Object::Bracket {
                open: '<',
                close: '>',
            },
            _ => return Parse::Invalid,
        };
        if rest_str[1 + objc.len_utf8()..].chars().next().is_some() {
            return Parse::Invalid;
        }
        return Parse::Complete(Command {
            op,
            register,
            count,
            target: Target::Object { inner, obj },
            keys: keys.into(),
        });
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
                    Parse::Invalid
                } else {
                    Parse::Complete(Command {
                        op,
                        register,
                        count,
                        target: Target::Motion(if backward {
                            Motion::SearchBackward(pat)
                        } else {
                            Motion::Search(pat)
                        }),
                        keys: keys.into(),
                    })
                }
            }
            _ => Parse::Invalid,
        };
    }

    // f/F/t/T + char.
    if matches!(rest[0], b'f' | b'F' | b't' | b'T') {
        let Some(ch) = char_at(rest_str, 1) else {
            return Parse::Incomplete;
        };
        if rest_str[1 + ch.len_utf8()..].chars().next().is_some() {
            return Parse::Invalid;
        }
        return Parse::Complete(Command {
            op,
            register,
            count,
            target: Target::Motion(Motion::FindChar {
                ch,
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
        b"W" => Motion::BigWordForward,
        b"B" => Motion::BigWordBackward,
        b"E" => Motion::BigWordEnd,
        b"%" => Motion::MatchPair,
        b"0" => Motion::LineStart,
        b"^" => Motion::FirstNonBlank,
        b"$" => Motion::LineEnd,
        b"|" => Motion::Column,
        b"gg" => Motion::FirstLine,
        b"ge" => Motion::WordEndBackward,
        b"gE" => Motion::BigWordEndBackward,
        b"{" => Motion::ParagraphBackward,
        b"}" => Motion::ParagraphForward,
        b"G" => Motion::LastLine,
        b"g" => return Parse::Incomplete,
        _ => return Parse::Invalid,
    };
    Parse::Complete(Command {
        op,
        register,
        count,
        target: Target::Motion(motion),
        keys: keys.into(),
    })
}
