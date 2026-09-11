//! Occurrence selection (0049 §7): `gb`/`gB` semantics. With no live
//! session, the word under the caret (vim's own word classes —
//! strop-grammar's `word_run`, no second parser) or a nonempty
//! charwise visual selection (literal text, never pattern syntax)
//! seeds the session; repeating adds the next unselected literal match
//! in document order, wrapping at most once. Selections are REAL
//! stretched ranges riding the visual operator cascade, so motions,
//! c/d/y, insert and undo treat every occurrence as one logical edit.

use strop_core::id::{BufferRevision, DocumentId};
use strop_core::selection::Selection;
use strop_core::Range;

use super::{Editor, Mode};

/// A live occurrence session. `added` is in add order (seed first) and
/// mirrors the selection set exactly — staleness (edits, external
/// cursor moves, a buffer switch) is detected on the next call and the
/// session re-seeds instead of acting on ghosts.
#[derive(Debug, Clone)]
pub(crate) struct OccurrenceState {
    document: DocumentId,
    revision: BufferRevision,
    needle: String,
    added: Vec<(usize, usize)>,
    /// Candidates passed over by skip: never offered again this session.
    skipped: Vec<(usize, usize)>,
    /// Where the next scan starts; `wrapped` arms once at the end.
    next_from: usize,
    wrapped: bool,
}

/// One scan step's answer.
enum Candidate {
    /// The match and the scan cursor past it.
    Found((usize, usize), usize),
    Exhausted,
}

/// Message-safe needle: quoted, control characters made visible.
fn quoted(needle: &str) -> String {
    format!("\"{}\"", strop_core::layout::printable_text(needle))
}

impl Editor {
    /// `gb`: with no live session, select the word under the caret (or
    /// seed the literal visual selection); otherwise add the next
    /// unselected match in document order, wrapping at most once.
    pub fn occurrence_next_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let Some(mut state) = self.occurrence_session() else {
            // the first press seeds — it never adds in the same breath
            self.occurrence_seed();
            return;
        };
        match self.occurrence_candidate(&mut state) {
            Candidate::Found(range, next_from) => {
                state.next_from = next_from;
                state.added.push(range);
                let head = self.buf().clamp_boundary(range.1 - 1);
                self.sels_mut().plant_extra_selection(range.0, head);
                self.occurrence = Some(state);
                self.occurrence_message();
            }
            Candidate::Exhausted => {
                let message = format!("no more occurrences of {}", quoted(&state.needle));
                self.occurrence = Some(state);
                self.message = message;
            }
        }
    }

    /// `gB`: select every occurrence of the needle in the buffer. The
    /// seed stays the primary; ranges passed over by skip stay
    /// unselected.
    pub fn occurrence_all_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let mut state = match self.occurrence_session() {
            Some(state) => state,
            None => {
                // no session: seed the caret's word, then enumerate —
                // seeding IS half of select-all's job
                self.occurrence_seed();
                let Some(state) = self.occurrence.take() else {
                    return;
                };
                state
            }
        };
        // full enumeration from the top; advancing by match END keeps
        // hits non-overlapping, so no range is ever selected twice
        let mut all: Vec<(usize, usize)> = Vec::new();
        let mut from = 0;
        while let Some((start, end)) = self.occurrence_literal_from(from, state.needle.as_bytes()) {
            all.push((start, end));
            from = end;
        }
        let seed = state.added[0];
        state.added = std::iter::once(seed)
            .chain(
                all.iter()
                    .copied()
                    .filter(|range| *range != seed && !state.skipped.contains(range)),
            )
            .collect();
        state.next_from = self.buf().len_bytes();
        state.wrapped = true;
        self.sels_mut().collapse_extras();
        let extras: Vec<(usize, usize)> = state.added.iter().skip(1).copied().collect();
        for (start, end) in extras {
            let head = self.buf().clamp_boundary(end - 1);
            self.sels_mut().plant_extra_selection(start, head);
        }
        self.occurrence = Some(state);
        self.occurrence_message();
    }

    /// `:select-skip`: advance the candidate without selecting it — the
    /// passed-over match is never offered again this session.
    pub fn occurrence_skip_pub(&mut self) {
        if self.buf().readonly {
            self.message = "readonly buffer".into();
            return;
        }
        let Some(mut state) = self.occurrence_session() else {
            // the first press seeds — there is nothing to skip past yet
            self.occurrence_seed();
            return;
        };
        match self.occurrence_candidate(&mut state) {
            Candidate::Found(range, next_from) => {
                state.next_from = next_from;
                state.skipped.push(range);
                let selected = state.added.len();
                self.occurrence = Some(state);
                self.message = format!("occurrence skipped ({selected} selected)");
            }
            Candidate::Exhausted => {
                let message = format!("no more occurrences of {}", quoted(&state.needle));
                self.occurrence = Some(state);
                self.message = message;
            }
        }
    }

    /// `:select-pop`: drop the last-added occurrence. Popping the seed
    /// ends the session; a popped range becomes a candidate again.
    pub fn occurrence_pop_pub(&mut self) {
        if self
            .occurrence
            .as_ref()
            .is_some_and(|state| !self.occurrence_fresh(state))
        {
            self.occurrence = None;
        }
        let Some(mut state) = self.occurrence.take() else {
            self.message = "no occurrence selection".into();
            return;
        };
        let popped = state.added[state.added.len() - 1];
        if state.added.len() == 1 {
            // popping the seed ends the session: the caret stays on it
            self.sels_mut().collapse_primary(popped.0);
            self.mode = Mode::Normal;
            self.message = "occurrence selection cleared".into();
            return;
        }
        state.added.pop();
        state.next_from = popped.0;
        state.wrapped = false;
        // the session is fresh, so the popped extra is exactly as planted
        let head = self.buf().clamp_boundary(popped.1 - 1);
        self.sels_mut().remove_extra(Selection {
            anchor: popped.0,
            head,
        });
        self.occurrence = Some(state);
        self.occurrence_message();
    }

    /// The live session: Some when one exists and still mirrors the
    /// selections exactly; None when there is no session or it went
    /// stale (the caller decides between seeding and refusing).
    fn occurrence_session(&mut self) -> Option<OccurrenceState> {
        if self
            .occurrence
            .as_ref()
            .is_some_and(|state| self.occurrence_fresh(state))
        {
            return self.occurrence.take();
        }
        self.occurrence = None;
        None
    }

    /// The session survives only while the selections are exactly its
    /// ranges in the same buffer at the same revision — any edit,
    /// motion by other means, or document switch re-seeds the next call.
    fn occurrence_fresh(&self, state: &OccurrenceState) -> bool {
        if state.document != self.current() || state.revision != self.buf().revision() {
            return false;
        }
        let mut current: Vec<(usize, usize)> = std::iter::once(self.sels().primary())
            .chain(self.extra_selections().iter().copied())
            .map(|selection| self.selection_span(selection))
            .collect();
        current.sort_unstable();
        let mut added = state.added.clone();
        added.sort_unstable();
        current == added
    }

    /// Seed from a nonempty charwise visual selection (literal text) or
    /// the word under the caret. Selects the seed as the stretched
    /// primary in Visual mode — the operator cascade owns the edits.
    fn occurrence_seed(&mut self) {
        let seed = if self.mode == Mode::Visual && self.anchor() != self.head() {
            // visual seeds keep their direction — no re-stretch
            self.visual_range()
                .map(|range| (self.buf().slice_string(range), range, false))
        } else {
            strop_grammar::word_run(self.buf(), self.head()).map(|(start, end)| {
                (
                    self.buf().slice_string(Range::charwise(start, end)),
                    Range::charwise(start, end),
                    true,
                )
            })
        };
        let Some((needle, range, restretch)) = seed else {
            self.message = "no word under cursor".into();
            return;
        };
        if needle.is_empty() {
            self.message = "empty selection — nothing to match".into();
            return;
        }
        let range = (range.start.get(), range.end.get());
        // 0049 §7.2: in a collection the seed must land in an editable
        // excerpt body — chrome rows can't seed a selection.
        if let Some(collection) = self.collections.get(&self.current()) {
            let inside = collection
                .excerpts
                .iter()
                .any(|e| range.0 >= e.view_start && range.1 <= e.view_end);
            if !inside {
                self.message = "not on an editable excerpt".into();
                return;
            }
        }
        if restretch {
            let head = self.buf().clamp_boundary(range.1 - 1);
            self.sels_mut().stretch_primary(range.0, head);
        }
        // the session owns the whole selection set
        self.sels_mut().collapse_extras();
        self.mode = Mode::Visual;
        self.occurrence = Some(OccurrenceState {
            document: self.current(),
            revision: self.buf().revision(),
            needle: needle.clone(),
            added: vec![range],
            skipped: Vec::new(),
            next_from: range.1,
            wrapped: false,
        });
        self.message = format!("1 occurrence of {}", quoted(&needle));
    }

    /// The next unselected, unskipped literal match from the scan
    /// cursor, wrapping at most once (0049 §7.3). Every step advances
    /// `next_from` past a hit or wraps once, so this always terminates.
    fn occurrence_candidate(&self, state: &mut OccurrenceState) -> Candidate {
        loop {
            match self.occurrence_literal_from(state.next_from, state.needle.as_bytes()) {
                Some((start, end)) => {
                    state.next_from = end;
                    if state.added.contains(&(start, end)) || state.skipped.contains(&(start, end))
                    {
                        continue;
                    }
                    return Candidate::Found((start, end), end);
                }
                None if !state.wrapped => {
                    state.wrapped = true;
                    state.next_from = 0;
                }
                None => return Candidate::Exhausted,
            }
        }
    }

    /// The selection-count message every session mutation leaves behind.
    fn occurrence_message(&mut self) {
        let Some(state) = &self.occurrence else {
            return;
        };
        let n = state.added.len();
        self.message = format!(
            "{n} occurrence{} of {}",
            if n == 1 { "" } else { "s" },
            quoted(&state.needle)
        );
    }
}

impl Editor {
    /// Occurrence scan (0049 §7.2): in a collection, only editable
    /// excerpt bodies can match — titles, header rows and gaps are
    /// chrome, never selection targets.
    fn occurrence_literal_from(&self, from: usize, needle: &[u8]) -> Option<(usize, usize)> {
        let id = self.current();
        let Some(collection) = self.collections.get(&id) else {
            return strop_grammar::literal_from(self.buf(), from, needle);
        };
        let mut from = from;
        loop {
            let (start, end) = strop_grammar::literal_from(self.buf(), from, needle)?;
            let inside = collection
                .excerpts
                .iter()
                .any(|e| start >= e.view_start && end <= e.view_end);
            if inside {
                return Some((start, end));
            }
            if end <= from {
                return None; // no progress — never spin
            }
            from = end;
        }
    }
}
