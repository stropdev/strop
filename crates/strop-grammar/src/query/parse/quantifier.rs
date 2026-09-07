use super::*;

pub(super) struct Quantifier {
    minimum: u32,
    maximum: Option<u32>,
    greedy: bool,
}

impl Parser<'_> {
    /// Apply a trailing quantifier to `atom` when one follows; a
    /// second quantifier after the first is E871 (multi follow multi).
    pub(super) fn parse_quantifier(&mut self, atom: Node) -> Result<Node, QueryError> {
        let Some(Quantifier {
            minimum: min,
            maximum: max,
            greedy,
        }) = self.quant_at()?
        else {
            return Ok(atom);
        };
        let after_first = self.at;
        if self.quant_at()?.is_some() {
            // vim's E871: a multi cannot follow a multi
            return Err(QueryError::BadRepeat { at: after_first });
        }
        if matches!(
            atom,
            Node::LineStart
                | Node::LineEnd
                | Node::BufStart
                | Node::BufEnd
                | Node::WordStart
                | Node::WordEnd
                | Node::SetStart
                | Node::SetEnd
        ) {
            // a quantifier on a zero-width assertion is vim's E871 too
            return Err(QueryError::BadRepeat { at: self.at });
        }
        Ok(Node::Rep {
            node: Box::new(atom),
            min,
            max,
            greedy,
        })
    }

    /// Consume a quantifier if one starts here; Ok(None) = none.
    pub(super) fn quant_at(&mut self) -> Result<Option<Quantifier>, QueryError> {
        match self.mode {
            Magic::Magic => {
                if self.peek() == Some(b'*') {
                    self.at += 1;
                    return Ok(Some(Quantifier {
                        minimum: 0,
                        maximum: None,
                        greedy: true,
                    }));
                }
                if self.starts_with(b"\\+") {
                    self.at += 2;
                    return Ok(Some(Quantifier {
                        minimum: 1,
                        maximum: None,
                        greedy: true,
                    }));
                }
                if self.starts_with(b"\\=") || self.starts_with(b"\\?") {
                    self.at += 2;
                    return Ok(Some(Quantifier {
                        minimum: 0,
                        maximum: Some(1),
                        greedy: true,
                    }));
                }
                if self.starts_with(b"\\{") {
                    return self.parse_braces().map(Some);
                }
            }
            Magic::VeryMagic => {
                if self.peek() == Some(b'*') {
                    self.at += 1;
                    return Ok(Some(Quantifier {
                        minimum: 0,
                        maximum: None,
                        greedy: true,
                    }));
                }
                if self.peek() == Some(b'+') {
                    self.at += 1;
                    return Ok(Some(Quantifier {
                        minimum: 1,
                        maximum: None,
                        greedy: true,
                    }));
                }
                if self.peek() == Some(b'=') || self.peek() == Some(b'?') {
                    self.at += 1;
                    return Ok(Some(Quantifier {
                        minimum: 0,
                        maximum: Some(1),
                        greedy: true,
                    }));
                }
                if self.peek() == Some(b'{') {
                    return self.parse_braces().map(Some);
                }
            }
        }
        Ok(None)
    }

    /// `\{n,m}` (magic) / `{n,m}` (very magic); a `-` right after the
    /// brace marks lazy (`\{-1,3}`, `\va{-}`). Reversed ranges swap
    /// (vim: `a\{2,1}b` matches "ab" — corpus-pinned).
    pub(super) fn parse_braces(&mut self) -> Result<Quantifier, QueryError> {
        let start = self.at;
        self.at += if self.mode == Magic::Magic { 2 } else { 1 };
        let greedy = if self.peek() == Some(b'-') {
            self.at += 1;
            false
        } else {
            true
        };
        let mut min: Option<u32> = None;
        let mut cur = String::new();
        let mut stage = 0; // 0 = min, 1 = max
        loop {
            let Some(c) = self.peek() else {
                return Err(QueryError::BadRepeat { at: start });
            };
            self.at += 1;
            match c {
                b'0'..=b'9' => cur.push(c as char),
                b',' if stage == 0 => {
                    min = Some(if cur.is_empty() {
                        0
                    } else {
                        self.num(&cur, start)?
                    });
                    cur.clear();
                    stage = 1;
                }
                b'}' => break,
                b'\\' if self.mode == Magic::Magic && self.peek() == Some(b'}') => {
                    self.at += 1; // tolerated escaped close
                    break;
                }
                _ => return Err(QueryError::BadRepeat { at: start }),
            }
        }
        let (min, max) = match stage {
            0 => {
                if cur.is_empty() {
                    // `\{}` / `{}` — same as star
                    (0, None)
                } else {
                    let n = self.num(&cur, start)?;
                    (n, Some(n))
                }
            }
            _ => {
                let hi = if cur.is_empty() {
                    None // `\{2,}` unbounded
                } else {
                    Some(self.num(&cur, start)?)
                };
                (min.unwrap_or(0), hi) // `\{,3}` — min omitted
            }
        };
        // vim swaps reversed ranges rather than erroring
        let (min, max) = match max {
            Some(hi) if hi < min => (hi, Some(min)),
            _ => (min, max),
        };
        Ok(Quantifier {
            minimum: min,
            maximum: max,
            greedy,
        })
    }

    pub(super) fn num(&self, s: &str, at: usize) -> Result<u32, QueryError> {
        s.parse::<u32>()
            .ok()
            .filter(|n| *n <= MAX_REPEAT)
            .ok_or(QueryError::BadRepeat { at })
    }
}
