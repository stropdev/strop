//! Keystrokes → commands. Incremental: `parse` is called on every
//! pending-key change and answers Incomplete / Invalid / Complete.

use crate::types::*;

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
            Some(&r) if r.is_ascii_alphanumeric() => {
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
    let count = (count1.max(1)) * (count2.max(1));
    let rest = &bytes[i..];

    if rest.is_empty() {
        return Parse::Incomplete;
    }

    // Surround (sandwich lineage): ds<x> / cs<x><y> / ys<motion><x>.
    // Aliases: b→( ) B→{ } r→[ ] a→< >.
    if let (Some(o), rest0 @ [b's', ..]) = (op, rest) {
        let tail = &rest0[1..];
        let map = |c: u8| match c {
            b'b' | b'(' | b')' => Some((b'(', b')')),
            b'B' | b'{' | b'}' => Some((b'{', b'}')),
            b'r' | b'[' | b']' => Some((b'[', b']')),
            b'a' | b'<' | b'>' => Some((b'<', b'>')),
            q @ (b'"' | b'\'' | b'`') => Some((q, q)),
            _ => None,
        };
        match o {
            Op::Delete => {
                if tail.is_empty() {
                    return Parse::Incomplete;
                }
                let Some(_) = map(tail[0]) else {
                    return Parse::Invalid;
                };
                if tail.len() > 1 {
                    return Parse::Invalid;
                }
                return Parse::Complete(Command {
                    op,
                    register,
                    count,
                    target: Target::SurroundDelete(tail[0]),
                    keys: keys.into(),
                });
            }
            Op::Change => {
                if tail.len() < 2 {
                    return Parse::Incomplete;
                }
                let (Some(_), Some(_)) = (map(tail[0]), map(tail[1])) else {
                    return Parse::Invalid;
                };
                if tail.len() > 2 {
                    return Parse::Invalid;
                }
                return Parse::Complete(Command {
                    op,
                    register,
                    count,
                    target: Target::SurroundChange {
                        from: tail[0],
                        to: tail[1],
                    },
                    keys: keys.into(),
                });
            }
            Op::Yank => {
                // ys<motion><char>: the trailing char is the surround;
                // everything before it must parse as a complete motion/object
                if tail.len() < 2 {
                    return Parse::Incomplete;
                }
                let (motion_keys, ch) = tail.split_at(tail.len() - 1);
                let Some(_) = map(ch[0]) else {
                    return Parse::Incomplete;
                };
                let motion_str = format!("y{}", String::from_utf8_lossy(motion_keys));
                match parse(&motion_str) {
                    Parse::Complete(sub) => {
                        return Parse::Complete(Command {
                            op,
                            register,
                            count,
                            target: Target::SurroundAdd {
                                ch: ch[0],
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
            register,
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
                        register,
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
            register,
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
        b"W" => Motion::BigWordForward,
        b"B" => Motion::BigWordBackward,
        b"E" => Motion::BigWordEnd,
        b"%" => Motion::MatchPair,
        b"0" => Motion::LineStart,
        b"$" => Motion::LineEnd,
        b"gg" => Motion::FirstLine,
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
