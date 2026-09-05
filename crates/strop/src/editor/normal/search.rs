//! normal/search.rs — search: / ? * # n N and find candidates.

use strop_core::Range;
use strop_grammar::{self as grammar, Command};

use crate::editor::Editor;
use crate::editor::LastSearch;

impl Editor {
    pub(super) fn note_search(&mut self, cmd: &Command) {
        match &cmd.target {
            strop_grammar::Target::Motion(strop_grammar::Motion::Search(p)) => {
                self.last_search = Some(LastSearch {
                    pattern: p.clone(),
                    backward: false,
                    whole_word: false,
                });
            }
            strop_grammar::Target::Motion(strop_grammar::Motion::SearchBackward(p)) => {
                self.last_search = Some(LastSearch {
                    pattern: p.clone(),
                    backward: true,
                    whole_word: false,
                });
            }
            strop_grammar::Target::Motion(strop_grammar::Motion::FindChar {
                ch,
                till,
                backward,
            }) => {
                self.last_find = Some((*ch, *backward, *till));
            }
            _ => {}
        }
    }

    /// `;` / `,`: replay the last f/F/t/T (vim: `,` inverts direction),
    /// line-local like the original find, cascading over cursors.
    pub(crate) fn repeat_find(&mut self, reverse: bool) {
        let Some((ch, backward, till)) = self.last_find else {
            self.message = "no previous find".into();
            return;
        };
        let backward = backward ^ reverse;
        // char-honest: ; , on f é must land on é (0014)
        let seek = |buf: &strop_core::Buffer, c: usize| -> Option<usize> {
            let line = buf.line_of(c);
            let (ls, le) = (buf.line_start(line), buf.line_end(line));
            let text = buf.line_text(line);
            if backward {
                for (off, t) in text.char_indices().rev() {
                    let pos = ls + off;
                    if pos >= c.min(le) {
                        continue;
                    }
                    if t == ch {
                        return Some(if till { (pos + 1).min(le) } else { pos });
                    }
                }
                return None;
            }
            for (off, t) in text.char_indices() {
                let pos = ls + off;
                if pos <= c {
                    continue;
                }
                if pos >= le {
                    break;
                }
                if t == ch {
                    return Some(if till {
                        pos.saturating_sub(1).max(ls)
                    } else {
                        pos
                    });
                }
            }
            None
        };
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| seek(self.buf(), s.head).unwrap_or(s.head))
            .collect();
        self.sels_mut().set_extras(extras);
        match seek(self.buf(), self.head()) {
            Some(h) => {
                self.set_head(h);
                self.flash(Range::charwise(self.head(), self.head()));
            }
            None => self.message = "find: no more matches".into(),
        }
        self.normalize_cursors();
    }

    /// `n` / `N`: repeat the armed search, wrapping at the file edges.
    /// Cascades: every cursor seeks from its own position (0013 §3).
    pub(crate) fn repeat_search(&mut self, invert: bool) {
        let Some(ls) = self.last_search.clone() else {
            self.message = "no previous search".into();
            return;
        };
        let backward = ls.backward ^ invert;
        self.push_jump(); // n/N are jumplist entries
                          // a whole-word match has non-word bytes (or the edge) on both
                          // flanks — vim's \< \> without the regex layer
        let word_char = |b: u8| b.is_ascii_alphanumeric() || b == b'_';
        let boundary_ok = |buf: &strop_core::Buffer, at: usize, len: usize| {
            let before_ok = at == 0 || !word_char(buf.byte(at - 1));
            let after_ok = at + len >= buf.len_bytes() || !word_char(buf.byte(at + len));
            !ls.whole_word || (before_ok && after_ok)
        };
        let seek = |buf: &strop_core::Buffer, from: usize| {
            let len = ls.pattern.len();
            let mut hit = if backward {
                grammar::search_backward(buf, from, &ls.pattern)
            } else {
                grammar::search_forward(buf, from + 1, &ls.pattern)
            };
            // skip boundary-mismatched hits (whole-word searches)
            let mut guard = 0;
            while hit.is_some_and(|h| !boundary_ok(buf, h, len)) && guard < 64 {
                let h = hit;
                hit = if backward {
                    grammar::search_backward(buf, h.unwrap_or(0), &ls.pattern)
                } else {
                    grammar::search_forward(buf, h.map(|x| x + 1).unwrap_or(0), &ls.pattern)
                };
                guard += 1;
            }
            // vim wraps around the file ends
            hit.or_else(|| {
                let mut h = if backward {
                    grammar::search_backward(buf, buf.len_bytes(), &ls.pattern)
                } else {
                    grammar::search_forward(buf, 0, &ls.pattern)
                };
                let mut guard = 0;
                while h.is_some_and(|x| !boundary_ok(buf, x, len)) && guard < 64 {
                    let cur = h;
                    h = if backward {
                        grammar::search_backward(buf, cur.unwrap_or(0), &ls.pattern)
                    } else {
                        grammar::search_forward(buf, cur.map(|x| x + 1).unwrap_or(0), &ls.pattern)
                    };
                    guard += 1;
                }
                h
            })
        };
        let extras: Vec<usize> = self
            .extra_selections()
            .iter()
            .map(|s| {
                seek(self.buf(), s.head)
                    .map(|h| self.buf().clamp_boundary(h))
                    .unwrap_or(s.head)
            })
            .collect();
        self.sels_mut().set_extras(extras);
        match seek(self.buf(), self.head()) {
            Some(h) => {
                self.set_head(self.buf().clamp_boundary(h));
                self.clamp_cursor();
                self.flash(Range::charwise(self.head(), self.head()));
            }
            None => self.message = format!("pattern not found: {}", ls.pattern),
        }
        self.normalize_cursors();
    }

    /// `*` / `#` (vim): search the word under the cursor — whole-word,
    /// forward / backward, wrapping. `n`/`N` keep the same anchors.
    pub(crate) fn search_word_under_cursor(&mut self, backward: bool) {
        // char-classified (0017): identifiers in every script count —
        // é/fün/変数 are words. Walk CHAR boundaries via the line text.
        let word_char = |c: char| c.is_alphanumeric() || c == '_';
        let buf_len = self.buf().len_bytes();
        let head = self.buf().clamp_boundary(self.head());
        if head >= buf_len {
            self.message = "no word under cursor".into();
            return;
        }
        let char_at = |p: usize| -> Option<char> {
            let line = self.buf().line_of(p);
            let (s, e) = (self.buf().line_start(line), self.buf().line_end(line));
            self.buf()
                .rope
                .byte_slice(s..e)
                .to_string()
                .trim_end_matches('\n')
                .get(p - s..)
                .and_then(|t| t.chars().next())
        };
        if !char_at(head).is_some_and(word_char) {
            self.message = "no word under cursor".into();
            return;
        }
        let mut start = head;
        while start > 0 {
            let prev = self.buf().clamp_boundary(start.saturating_sub(4));
            let prev = if prev == start { start - 1 } else { prev };
            if char_at(prev).is_some_and(word_char) {
                start = prev;
            } else {
                break;
            }
        }
        let mut end = head;
        while end < buf_len {
            let line = self.buf().line_of(end);
            let (_, le) = (self.buf().line_start(line), self.buf().line_end(line));
            let next = char_at(end).map(|c| end + c.len_utf8());
            match next {
                Some(n) if n <= le && char_at(end).is_some_and(word_char) => end = n,
                _ => break,
            }
        }
        let pattern = self.buf().rope.byte_slice(start..end).to_string();
        self.last_search = Some(LastSearch {
            pattern,
            backward,
            whole_word: true,
        });
        // `#` seeks from the word's start so the current word isn't its
        // own "previous" match (vim semantics)
        if backward {
            self.set_head(start);
        }
        self.repeat_search(false);
    }

    /// Pending f/F/t/T awaiting its char: the leap-style candidates.
    pub fn find_candidates(&self) -> Option<(u8, bool)> {
        let b = self.pending.as_bytes();
        let (&pfx, _) = b.split_last()?;
        let backward = matches!(pfx, b'F' | b'T');
        if !matches!(pfx, b'f' | b'F' | b't' | b'T') {
            return None;
        }
        Some((pfx, backward))
    }

    /// Pending search pattern (incsearch highlight), if any.
    pub fn search_pattern(&self) -> Option<&str> {
        self.pending
            .find('/')
            .map(|i| &self.pending[i + 1..])
            .filter(|p| !p.is_empty())
    }
}
