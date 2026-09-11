//! normal/search.rs — search: / ? * # n N and find candidates. The
//! query engine lives in strop-grammar (CompiledQuery); the prompt's
//! live resolution lives in `pending` (one owner, R7).

use strop_core::Range;
use strop_grammar::{self as grammar, Command};

use crate::editor::{Editor, FindPending, LastSearch};

impl Editor {
    pub(crate) fn note_search(&mut self, cmd: &Command) {
        match &cmd.target {
            grammar::Target::Motion(grammar::Motion::Search(query)) => {
                self.last_search = Some(LastSearch {
                    query: query.clone(),
                    backward: false,
                });
            }
            grammar::Target::Motion(grammar::Motion::SearchBackward(query)) => {
                self.last_search = Some(LastSearch {
                    query: query.clone(),
                    backward: true,
                });
            }
            grammar::Target::Motion(grammar::Motion::FindChar { ch, till, backward }) => {
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
        let seek = |buf: &strop_core::Buffer, cursor: usize| -> Option<usize> {
            let landing = |target: usize| {
                if !till {
                    target
                } else if backward {
                    buf.ceil_boundary(target + 1)
                } else {
                    buf.clamp_boundary(target.saturating_sub(1))
                }
            };
            let mut target = grammar::find_character(buf, cursor.into(), ch, backward, 1)?;
            while landing(target.get()) == cursor {
                target = grammar::find_character(buf, target, ch, backward, 1)?;
            }
            Some(landing(target.get()))
        };
        let extras: Vec<strop_core::selection::Selection> = self
            .extra_selections()
            .iter()
            .map(|s| strop_core::selection::Selection {
                anchor: s.anchor,
                head: seek(self.buf(), s.head).unwrap_or(s.head),
            })
            .collect();
        self.sels_mut().set_extra_selections(extras);
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
    /// The query is the one compiled engine — whole-word and the regex
    /// dialect live inside it, never re-filtered here.
    pub(crate) fn repeat_search(&mut self, invert: bool) {
        let Some(search) = self.last_search.clone() else {
            self.message = "no previous search".into();
            return;
        };
        let motion = if search.backward ^ invert {
            grammar::Motion::SearchBackward(search.query.clone())
        } else {
            grammar::Motion::Search(search.query.clone())
        };
        let command = grammar::Command {
            op: None,
            register: None,
            count: None,
            target: grammar::Target::Motion(motion),
            keys: if invert { "N" } else { "n" }.into(),
        };
        if self.defer_resolution(
            &command,
            self.all_cursors(),
            super::super::resolution::ResolutionPurpose::RepeatSearch(invert),
        ) {
            return;
        }
        self.move_cursor(&command);
        // N reverses this movement, not the saved search's direction.
        self.last_search = Some(search);
    }

    /// `*` / `#` (vim): search the word under the cursor — whole-word,
    /// forward / backward, wrapping. `n`/`N` keep the same anchors.
    pub(crate) fn search_word_under_cursor(&mut self, backward: bool) {
        // char-classified (0017): identifiers in every script count —
        // é/fün/変数 are words. Walk scalar boundaries directly on the rope.
        let word_char = |c: char| c.is_alphanumeric() || c == '_';
        let buf_len = self.buf().len_bytes();
        let head = self.buf().clamp_boundary(self.head());
        if head >= buf_len {
            self.message = "no word under cursor".into();
            return;
        }
        let char_at = |position: usize| -> Option<char> {
            (position < buf_len).then(|| {
                self.buf()
                    .text()
                    .char(self.buf().text().byte_to_char(position))
            })
        };
        if !char_at(head).is_some_and(word_char) {
            self.message = "no word under cursor".into();
            return;
        }
        let mut start = head;
        while start > 0 {
            let prev = self.buf().clamp_boundary(start - 1);
            if char_at(prev).is_some_and(word_char) {
                start = prev;
            } else {
                break;
            }
        }
        let mut end = head;
        while end < buf_len {
            match char_at(end) {
                Some(ch) if word_char(ch) => end += ch.len_utf8(),
                _ => break,
            }
        }
        let pattern = self.buf().text().byte_slice(start..end).to_string();
        let query = match grammar::CompiledQuery::compile(&pattern, true) {
            Ok(query) => query,
            Err(error) => {
                self.message = error.to_string();
                return;
            }
        };
        self.last_search = Some(LastSearch { query, backward });
        // `#` seeks from the word's start so the current word isn't its
        // own "previous" match (vim semantics)
        if backward {
            self.set_head(start);
        }
        self.repeat_search(false);
    }

    /// Pending `f/F/t/T` awaiting its char: the leap-style candidates.
    /// The WALKER owns that state — the old check read the free-text
    /// line's last byte, so any pattern ending in `f`/`t` (`/const`)
    /// lit candidates over the cursor line (issue 13's "stale match").
    pub fn find_candidates(&self) -> Option<FindPending> {
        let m = self.walker.pending_motion();
        let ch = m.chars().next()?;
        (m.chars().count() == 1 && matches!(ch, 'f' | 'F' | 't' | 'T')).then_some(FindPending {
            ch,
            backward: matches!(ch, 'F' | 'T'),
        })
    }

    /// The active query's identity. Painting consumes its revision-matched
    /// worker summary; only grammatical test oracles enumerate all matches.
    pub fn current_search_query(
        &self,
    ) -> Result<Option<grammar::CompiledQuery>, grammar::QueryError> {
        if let Some(pattern) = self.search_pattern() {
            grammar::CompiledQuery::compile(pattern, false).map(Some)
        } else {
            Ok(self.last_search.as_ref().map(|search| search.query.clone()))
        }
    }
}
