use super::classes::letter_class;
use super::*;

impl Parser<'_> {
    // -------------------------------------------------------------- atoms

    pub(super) fn parse_atom(&mut self) -> Result<Node, QueryError> {
        let at = self.at;
        let ch = self.char_at(at)?;
        match self.mode {
            Magic::Magic => self.parse_atom_magic(ch, at),
            Magic::VeryMagic => self.parse_atom_very(ch, at),
        }
    }

    pub(super) fn parse_atom_magic(&mut self, ch: char, at: usize) -> Result<Node, QueryError> {
        match ch {
            '\\' => self.parse_escape(at),
            '.' => {
                self.at += 1;
                Ok(Node::Any { nl: false })
            }
            '[' => self.parse_class(at),
            '*' => {
                // a star with nothing to repeat is a literal (vim)
                self.at += 1;
                Ok(Node::Literal(b"*".to_vec()))
            }
            '^' => {
                self.at += 1;
                // magic: ^ anchors only at a branch start, else literal
                if self.branch_start {
                    Ok(Node::LineStart)
                } else {
                    Ok(Node::Literal(b"^".to_vec()))
                }
            }
            '$' => {
                let anchor = self.rest_is_branch_end();
                self.at += 1;
                if anchor {
                    Ok(Node::LineEnd)
                } else {
                    Ok(Node::Literal(b"$".to_vec()))
                }
            }
            '~' => Err(QueryError::Unsupported {
                construct: "~ (last substitute pattern)",
                at,
            }),
            _ => {
                self.at += ch.len_utf8();
                Ok(Node::Literal(ch.to_string().into_bytes()))
            }
        }
    }

    /// Magic mode: `$` anchors before `\|`, `\)`, `\n` or the end.
    pub(super) fn rest_is_branch_end(&self) -> bool {
        let rest = &self.pat[(self.at + 1).min(self.pat.len())..];
        rest.is_empty()
            || rest.starts_with(b"\\|")
            || rest.starts_with(b"\\)")
            || rest.starts_with(b"\\n")
    }

    /// An atom whose match consumes a line break (`\n`, `\.`, `\_s`).
    pub(super) fn is_break_atom(n: &Node) -> bool {
        match n {
            Node::Set(s) => s.nl,
            Node::Any { nl } => *nl,
            Node::Break => true,
            Node::Rep { node, .. } => Self::is_break_atom(node),
            _ => false,
        }
    }

    pub(super) fn parse_atom_very(&mut self, ch: char, at: usize) -> Result<Node, QueryError> {
        match ch {
            '\\' => self.parse_escape(at),
            '.' => {
                self.at += 1;
                Ok(Node::Any { nl: false })
            }
            '[' => self.parse_class(at),
            '(' => {
                self.ngroups += 1;
                let idx = self.ngroups;
                self.at += 1;
                let inner = self.parse_alt()?;
                if !self.starts_with(b")") {
                    return Err(QueryError::UnbalancedGroup { at });
                }
                self.at += 1;
                Ok(Node::Group {
                    idx: Some(idx),
                    node: Box::new(inner),
                })
            }
            ')' => Err(QueryError::UnbalancedGroup { at }),
            '^' => {
                self.at += 1;
                Ok(Node::LineStart)
            }
            '$' => {
                self.at += 1;
                Ok(Node::LineEnd)
            }
            '<' => {
                self.at += 1;
                Ok(Node::WordStart)
            }
            '>' => {
                self.at += 1;
                Ok(Node::WordEnd)
            }
            '~' => Err(QueryError::Unsupported {
                construct: "~ (last substitute pattern)",
                at,
            }),
            '@' => {
                let which = match self.peek2() {
                    Some(b'=') => "\\@= (lookahead)",
                    Some(b'!') => "\\@! (negative lookahead)",
                    Some(b'<') => "\\@<= (lookbehind)",
                    _ => "\\@ (lookaround family)",
                };
                Err(QueryError::Unsupported {
                    construct: which,
                    at,
                })
            }
            '&' => Err(QueryError::Unsupported {
                construct: "& (branch conjunction)",
                at,
            }),
            '%' => self.parse_percent(at),
            '*' | '+' | '=' | '?' | '{' => Err(QueryError::BadRepeat { at }),
            _ => {
                self.at += ch.len_utf8();
                Ok(Node::Literal(ch.to_string().into_bytes()))
            }
        }
    }

    /// `\%...` (both modes; `self.at` is on the `\`) and bare `%...`
    /// in very magic (self.at on the `%`).
    pub(super) fn parse_percent(&mut self, at: usize) -> Result<Node, QueryError> {
        let lead_len = if self.mode == Magic::Magic { 2 } else { 1 };
        let item = self.pat.get(at + lead_len).copied();
        match item {
            Some(b'(') => {
                self.at = at + lead_len + 1;
                self.branch_start = true;
                let inner = self.parse_alt()?;
                let close: &[u8] = if self.mode == Magic::Magic {
                    b"\\)"
                } else {
                    b")"
                };
                if !self.starts_with(close) {
                    return Err(QueryError::UnbalancedGroup { at });
                }
                self.at += close.len();
                Ok(Node::Group {
                    idx: None,
                    node: Box::new(inner),
                })
            }
            Some(b'[') => {
                self.at = at + lead_len + 1;
                let items = self.parse_optional_seq(at)?;
                Ok(Node::PrefixOpt(items))
            }
            Some(b'^') => {
                self.at = at + lead_len + 1;
                Ok(Node::BufStart)
            }
            Some(b'$') => {
                self.at = at + lead_len + 1;
                Ok(Node::BufEnd)
            }
            Some(k @ (b'd' | b'x' | b'u' | b'U')) => {
                self.at = at + lead_len;
                self.parse_char_code(at, k)
            }
            Some(b'V') => Err(QueryError::Unsupported {
                construct: "\\%V (visual-area match)",
                at,
            }),
            Some(b'#') => Err(QueryError::Unsupported {
                construct: "\\%# (cursor match)",
                at,
            }),
            Some(b'=') => Err(QueryError::Unsupported {
                construct: "\\%= (regexp engine selector)",
                at,
            }),
            Some(b'l') | Some(b'c') | Some(b'v') => Err(QueryError::Unsupported {
                construct: "\\%l / \\%c / \\%v (line/column match)",
                at,
            }),
            Some(b'0'..=b'9') => Err(QueryError::Unsupported {
                // \%23l / \%23c / \%23v — a number-prefixed position
                construct: "\\%l / \\%c / \\%v (line/column match)",
                at,
            }),
            _ => Err(QueryError::Unsupported {
                construct: "\\% (special item)",
                at,
            }),
        }
    }

    /// `\%[abcd]` body: atoms until `]` (bare `]` in both modes —
    /// vim's E69 expects a plain close here).
    pub(super) fn parse_optional_seq(&mut self, at: usize) -> Result<Vec<Node>, QueryError> {
        let mut items = Vec::new();
        loop {
            match self.peek() {
                None => return Err(QueryError::UnclosedOptional { at }),
                Some(b']') => {
                    self.at += 1;
                    break;
                }
                _ => {
                    let atom = self.parse_atom()?;
                    items.push(self.parse_quantifier(atom)?);
                }
            }
        }
        Ok(items)
    }

    /// `\%d123` / `\%x2a` / `\%u20ac` / `\%U0001f600`. `self.at` is on
    /// the type letter.
    pub(super) fn parse_char_code(&mut self, at: usize, kind: u8) -> Result<Node, QueryError> {
        self.at += 1; // the type letter
        let n: u32 = match kind {
            b'd' => {
                let mut v = 0u32;
                let mut any = false;
                while let Some(c @ b'0'..=b'9') = self.peek() {
                    v = v
                        .checked_mul(10)
                        .and_then(|x| x.checked_add((c - b'0') as u32))
                        .ok_or(QueryError::BadCharCode { at })?;
                    self.at += 1;
                    any = true;
                }
                if !any {
                    return Err(QueryError::BadCharCode { at });
                }
                v
            }
            b'x' | b'u' | b'U' => {
                let width = match kind {
                    b'x' => 2,
                    b'u' => 4,
                    _ => 8,
                };
                let mut v = 0u32;
                for _ in 0..width {
                    let c = self.peek().ok_or(QueryError::BadCharCode { at })?;
                    let d = (c as char)
                        .to_digit(16)
                        .ok_or(QueryError::BadCharCode { at })?;
                    v = v * 16 + d;
                    self.at += 1;
                }
                v
            }
            _ => unreachable!("kind filtered by caller"),
        };
        let ch = char::from_u32(n).ok_or(QueryError::BadCharCode { at })?;
        Ok(Node::Literal(ch.to_string().into_bytes()))
    }

    /// Everything after a backslash, both magic modes; `self.at` is on
    /// the `\`.
    pub(super) fn parse_escape(&mut self, at: usize) -> Result<Node, QueryError> {
        let c = self.peek2().ok_or(QueryError::TrailingBackslash { at })?;
        // Escaped metachars are literals — but WHICH chars are meta
        // depends on the mode: in magic, `\( \) \| \{ \< \>` (and
        // `\@` lookaround) are structural, so their escapes here must
        // fall through to the structural arms below. In very magic,
        // escaping always removes the magic.
        let literal_escape = match self.mode {
            Magic::Magic => matches!(
                c,
                b'.' | b'*'
                    | b'+'
                    | b'='
                    | b'?'
                    | b'['
                    | b']'
                    | b'^'
                    | b'$'
                    | b'~'
                    | b'&'
                    | b'\\'
                    | b'/'
                    | b'#'
                    | b'"'
            ),
            Magic::VeryMagic => matches!(
                c,
                b'.' | b'*'
                    | b'+'
                    | b'='
                    | b'?'
                    | b'['
                    | b']'
                    | b'{'
                    | b'}'
                    | b'('
                    | b')'
                    | b'|'
                    | b'<'
                    | b'>'
                    | b'~'
                    | b'&'
                    | b'@'
                    | b'\\'
                    | b'/'
                    | b'#'
                    | b'"'
            ),
        };
        if literal_escape {
            self.at += 2;
            return Ok(Node::Literal(vec![c]));
        }
        // magic groups open with \( — very magic uses the bare `(`
        // handled in parse_atom_very, so an escaped \( here is magic's
        if c == b'(' {
            self.at += 2; // consume the \( before recursing
            self.ngroups += 1;
            let idx = self.ngroups;
            self.branch_start = true;
            let inner = self.parse_alt()?;
            if !self.starts_with(b"\\)") {
                return Err(QueryError::UnbalancedGroup { at });
            }
            self.at += 2;
            return Ok(Node::Group {
                idx: Some(idx),
                node: Box::new(inner),
            });
        }
        if c == b')' || c == b'|' {
            // a closer/separator reaching atom position is unbalanced
            return Err(QueryError::UnbalancedGroup { at });
        }
        if c == b'@' && self.mode == Magic::Magic {
            let which = match self.pat.get(at + 2).copied() {
                Some(b'=') => "\\@= (lookahead)",
                Some(b'!') => "\\@! (negative lookahead)",
                Some(b'<') => "\\@<= (lookbehind)",
                _ => "\\@ (lookaround family)",
            };
            return Err(QueryError::Unsupported {
                construct: which,
                at,
            });
        }
        // two-char specials
        self.at += 2;
        match c {
            b'<' => return Ok(Node::WordStart),
            b'>' => return Ok(Node::WordEnd),
            b'c' => {
                self.fold = Some(true);
                return Ok(Node::Seq(vec![]));
            }
            b'C' => {
                self.fold = Some(false);
                return Ok(Node::Seq(vec![]));
            }
            b'n' => {
                // one whole line break (\r\n or \n) — CRLF counts as one
                return Ok(Node::Break);
            }
            b'r' => return Ok(Node::Literal(vec![b'\r'])),
            b't' => return Ok(Node::Literal(vec![b'\t'])),
            b'e' => return Ok(Node::Literal(vec![0x1b])),
            b'b' => return Ok(Node::Literal(vec![0x08])),
            b'f' => return Ok(Node::Literal(vec![0x0c])),
            _ => {}
        }
        // \zs / \ze: strict — anything else after \z is a refusal
        if c == b'z' {
            return match self.peek() {
                Some(b's') => {
                    self.at += 1;
                    Ok(Node::SetStart)
                }
                Some(b'e') => {
                    self.at += 1;
                    Ok(Node::SetEnd)
                }
                _ => Err(QueryError::Unsupported {
                    construct: "\\z (only \\zs and \\ze exist)",
                    at,
                }),
            };
        }
        // backreference
        if c.is_ascii_digit() && c != b'0' {
            return Ok(Node::Backref((c - b'0') as usize));
        }
        // \_x newline-extended atoms
        if c == b'_' {
            return match self.peek() {
                Some(b'.') => {
                    self.at += 1;
                    Ok(Node::Any { nl: true })
                }
                Some(b'[') => {
                    let mut node = self.parse_class(self.at)?;
                    if let Node::Set(s) = &mut node {
                        s.nl = true;
                    }
                    Ok(node)
                }
                Some(b'^') => {
                    self.at += 1;
                    Ok(Node::LineStart)
                }
                Some(b'$') => {
                    self.at += 1;
                    Ok(Node::LineEnd)
                }
                Some(letter) => {
                    if let Some(f) = letter_class(letter) {
                        self.at += 1;
                        let mut s = f();
                        s.nl = true;
                        Ok(Node::Set(s))
                    } else {
                        Err(QueryError::Unsupported {
                            construct: "\\_ (newline-extended class)",
                            at,
                        })
                    }
                }
                None => Err(QueryError::TrailingBackslash { at }),
            };
        }
        // letter classes (\d \s \w …)
        if let Some(f) = letter_class(c) {
            return Ok(Node::Set(f()));
        }
        // \% specials: rewind onto the backslash and share the reader
        if c == b'%' {
            self.at -= 2;
            return self.parse_percent(at);
        }
        // magic levels: \v/\m only at the very start (handled there);
        // anywhere else — and \M/\V everywhere — are typed refusals
        if matches!(c, b'v' | b'm' | b'M' | b'V') {
            return Err(QueryError::Unsupported {
                construct: match c {
                    b'v' | b'm' => "magic-level prefix (only at pattern start)",
                    b'M' => "\\M (nomagic)",
                    _ => "\\V (very nomagic)",
                },
                at,
            });
        }
        // unknown escape: the literal char that followed the backslash
        // (vim's rule: `a\qb` matches "aqb" — corpus-pinned)
        let ch = self.char_at(at + 1).map_err(|_| QueryError::Unsupported {
            construct: "invalid UTF-8 in pattern",
            at,
        })?;
        self.at = at + 1 + ch.len_utf8();
        Ok(Node::Literal(ch.to_string().into_bytes()))
    }
}
