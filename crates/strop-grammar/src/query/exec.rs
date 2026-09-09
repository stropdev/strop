//! The bounded backtracking engine. Operates directly on the rope via
//! random byte access — no line or file materialization on the hot
//! path. Every attempt runs under a step budget; exhaustion is a typed
//! [`QueryError::TooComplex`], never a hang.

use ropey::Rope;
use strop_core::id::ByteOffset;

use super::parse::{Class, Inst, Program};
use super::{QueryError, SearchMatch};

/// Steps per match attempt (vim's E363 family — complex patterns
/// error instead of freezing the editor).
pub const STEP_BUDGET: u64 = 100_000;

/// Runs one program against one text. All positions are byte offsets;
/// UTF-8 decoding is manual (the rope is always valid UTF-8).
pub(crate) struct Matcher<'a> {
    rope: &'a Rope,
    len: usize,
    query: &'a super::CompiledQuery,
}

enum Mark {
    Slot(usize, usize),
    Guard(usize, Option<usize>),
}

struct Frame {
    pc: u32,
    pos: usize,
    mark: usize,
}

const UNSET: usize = usize::MAX;

impl<'a> Matcher<'a> {
    pub(crate) fn new(rope: &'a Rope, query: &'a super::CompiledQuery) -> Self {
        Self {
            len: rope.len_bytes(),
            rope,
            query,
        }
    }

    pub(super) fn len(&self) -> usize {
        self.len
    }

    fn byte(&self, i: usize) -> u8 {
        debug_assert!(i < self.len, "matcher reads stay in bounds");
        self.rope.byte(i)
    }

    /// One full line break at `pos`: 2 for `\r\n`, 1 for `\n`.
    fn break_at(&self, pos: usize) -> Option<usize> {
        match self.byte(pos) {
            b'\n' => Some(1),
            b'\r' if pos + 1 < self.len && self.byte(pos + 1) == b'\n' => Some(2),
            _ => None,
        }
    }

    fn char_len(&self, pos: usize) -> usize {
        let lead = self.byte(pos);
        let mut n = match lead {
            0x00..=0x7f => 1,
            0xc0..=0xdf => 2,
            0xe0..=0xef => 3,
            _ => 4,
        };
        // clamp at the text end (a truncated final char still advances)
        while pos + n > self.len {
            n -= 1;
        }
        n.max(1)
    }

    fn char_at(&self, pos: usize) -> char {
        let len = self.char_len(pos);
        let mut buf = [0u8; 4];
        for (i, slot) in buf.iter_mut().enumerate().take(len) {
            *slot = self.byte(pos + i);
        }
        std::str::from_utf8(&buf[..len])
            .ok()
            .and_then(|s| s.chars().next())
            .unwrap_or('\u{fffd}')
    }

    /// The char that ENDS at `pos` (for word flanks).
    fn prev_char(&self, pos: usize) -> Option<char> {
        if pos == 0 {
            return None;
        }
        let mut start = pos - 1;
        let mut steps = 0;
        while start > 0 && (self.byte(start) & 0xC0) == 0x80 && steps < 3 {
            start -= 1;
            steps += 1;
        }
        Some(self.char_at(start))
    }

    fn word(c: char) -> bool {
        c.is_alphanumeric() || c == '_'
    }

    /// The editor's word model — same classifier as w/b/e and `*`.
    fn word_flanked(&self, start: usize, end: usize) -> bool {
        let before_ok = start == 0 || !Self::word(self.prev_char(start).unwrap_or('\u{0}'));
        let after_ok = end >= self.len || !Self::word(self.char_at(end));
        before_ok && after_ok
    }

    fn line_start_here(&self, pos: usize) -> bool {
        pos == 0 || self.byte(pos - 1) == b'\n'
    }

    fn line_end_here(&self, pos: usize) -> bool {
        pos >= self.len
            || self.byte(pos) == b'\n'
            || (self.byte(pos) == b'\r' && pos + 1 < self.len && self.byte(pos + 1) == b'\n')
    }

    /// `\<`: a word char starts at pos with no word char before it.
    fn word_start_here(&self, pos: usize) -> bool {
        pos < self.len && {
            let c = self.char_at(pos);
            Self::word(c) && (pos == 0 || !Self::word(self.prev_char(pos).unwrap_or('\u{0}')))
        }
    }

    /// `\>`: a word char ends at pos with no word char after it.
    fn word_end_here(&self, pos: usize) -> bool {
        (pos > 0 && Self::word(self.prev_char(pos).unwrap_or('\u{0}')))
            && (pos >= self.len || !Self::word(self.char_at(pos)))
    }

    fn consume(&self, class: &Class, pos: usize) -> Option<usize> {
        if pos >= self.len {
            return None;
        }
        match class {
            Class::Str { bytes, fold } => {
                if pos + bytes.len() > self.len {
                    return None;
                }
                if !*fold {
                    for (i, b) in bytes.iter().enumerate() {
                        if self.byte(pos + i) != *b {
                            return None;
                        }
                    }
                    Some(pos + bytes.len())
                } else {
                    self.consume_fold(bytes, pos)
                }
            }
            Class::Any { nl } => match self.break_at(pos) {
                Some(w) if *nl => Some(pos + w),
                Some(_) => None,
                None => Some(pos + self.char_len(pos)),
            },
            Class::Break => self.break_at(pos).map(|w| pos + w),
            Class::Set(s) => {
                if s.empty {
                    return None;
                }
                if let Some(w) = self.break_at(pos) {
                    return if s.nl { Some(pos + w) } else { None };
                }
                let c = self.char_at(pos);
                let mut in_set = (c as u32) < 128 && (s.ascii >> c as u32) & 1 == 1;
                if !in_set {
                    in_set = s.unicode_ranges.iter().any(|range| range.contains(&c));
                }
                if !in_set && s.fold {
                    for flip in c.to_lowercase().chain(c.to_uppercase()) {
                        if ((flip as u32) < 128 && (s.ascii >> flip as u32) & 1 == 1)
                            || s.unicode_ranges.iter().any(|range| range.contains(&flip))
                        {
                            in_set = true;
                            break;
                        }
                    }
                }
                let hit = if s.negated { !in_set } else { in_set };
                if hit {
                    Some(pos + self.char_len(pos))
                } else {
                    None
                }
            }
        }
    }

    fn consume_fold(&self, bytes: &[u8], pos: usize) -> Option<usize> {
        let pat = std::str::from_utf8(bytes).ok()?;
        let mut at = pos;
        for pc in pat.chars() {
            if at >= self.len {
                return None;
            }
            let tc = self.char_at(at);
            if !chars_eq_fold(pc, tc) {
                return None;
            }
            at += self.char_len(at);
        }
        Some(at)
    }

    /// Byte-equal range compare (optional case fold) for backrefs.
    fn ranges_eq(&self, a: usize, b: usize, len: usize, fold: bool) -> bool {
        if !fold {
            for i in 0..len {
                if self.byte(a + i) != self.byte(b + i) {
                    return false;
                }
            }
            return true;
        }
        let mut x = a;
        let mut y = b;
        let end = a + len;
        while x < end {
            let (cx, lx) = (self.char_at(x), self.char_len(x));
            let (cy, ly) = (self.char_at(y), self.char_len(y));
            if !chars_eq_fold(cx, cy) {
                return false;
            }
            x += lx;
            y += ly;
        }
        true
    }

    /// One match attempt at exactly `start`; errors only on budget.
    pub(super) fn find_at(
        &self,
        prog: &Program,
        start: usize,
    ) -> Result<Option<(usize, usize)>, QueryError> {
        if self.query.cancelled() {
            return Err(QueryError::Cancelled);
        }
        let mut saves = vec![UNSET; prog.n_slots];
        let mut guards = vec![None; prog.n_loops];
        let mut trail: Vec<Mark> = Vec::new();
        let mut stack: Vec<Frame> = vec![Frame {
            pc: 0,
            pos: start,
            mark: 0,
        }];
        let mut steps = 0u64;
        let mut empty_fallback: Option<(usize, usize)> = None;
        while let Some(frame) = stack.pop() {
            while trail.len() > frame.mark {
                match trail.pop() {
                    Some(Mark::Slot(s, v)) => saves[s] = v,
                    Some(Mark::Guard(g, v)) => guards[g] = v,
                    None => {}
                }
            }
            let (mut pc, mut pos) = (frame.pc as usize, frame.pos);
            loop {
                steps += 1;
                if steps > STEP_BUDGET {
                    return Err(QueryError::TooComplex);
                }
                if steps.is_multiple_of(512) && self.query.cancelled() {
                    return Err(QueryError::Cancelled);
                }
                match &prog.insts[pc] {
                    Inst::Consume(class) => match self.consume(class, pos) {
                        Some(next) => {
                            pos = next;
                            pc += 1;
                        }
                        None => break,
                    },
                    Inst::Split { prefer, alt } => {
                        stack.push(Frame {
                            pc: *alt,
                            pos,
                            mark: trail.len(),
                        });
                        pc = *prefer as usize;
                    }
                    Inst::Jmp(x) => pc = *x as usize,
                    Inst::Save(slot) => {
                        trail.push(Mark::Slot(*slot, saves[*slot]));
                        saves[*slot] = pos;
                        pc += 1;
                    }
                    Inst::Guard(id) => {
                        if guards[*id] == Some(pos) {
                            break; // zero-width pass: force the exit arm
                        }
                        trail.push(Mark::Guard(*id, guards[*id]));
                        guards[*id] = Some(pos);
                        pc += 1;
                    }
                    Inst::LineStart => {
                        if self.line_start_here(pos) {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::LineEnd => {
                        if self.line_end_here(pos) {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::BufStart => {
                        if pos == 0 {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::BufEnd => {
                        if pos == self.len {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::WordStart => {
                        if self.word_start_here(pos) {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::WordEnd => {
                        if self.word_end_here(pos) {
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Backref { group } => {
                        let (s, e) = (saves[2 * group], saves[2 * group + 1]);
                        if s == UNSET || e == UNSET || pos + (e - s) > self.len {
                            break;
                        }
                        if self.ranges_eq(pos, s, e - s, prog.fold) {
                            pos += e - s;
                            pc += 1;
                        } else {
                            break;
                        }
                    }
                    Inst::Fail => break,
                    Inst::Match => {
                        let rep_start = if saves[0] != UNSET { saves[0] } else { start };
                        let rep_end = if saves[1] != UNSET { saves[1] } else { pos };
                        if prog.whole_word && !self.word_flanked(rep_start, rep_end) {
                            break; // keep backtracking: another path may fit
                        }
                        if rep_start == rep_end && empty_fallback.is_none() {
                            // vim's buffer search prefers a non-empty
                            // match at the same position (`a\{-}` takes
                            // one char, `^$` still matches); keep going
                            empty_fallback = Some((rep_start, rep_end));
                            break;
                        }
                        return Ok(Some((rep_start, rep_end)));
                    }
                }
            }
        }
        Ok(empty_fallback)
    }

    /// The next char boundary strictly after `pos` (len + 1 ends scans).
    pub(super) fn next_boundary(&self, pos: usize) -> usize {
        if pos >= self.len {
            return self.len + 1;
        }
        pos + self.char_len(pos)
    }
}

/// Unicode-simple case equality for `\c` matching.
fn chars_eq_fold(a: char, b: char) -> bool {
    if a == b {
        return true;
    }
    let mut la = a.to_lowercase();
    let mut lb = b.to_lowercase();
    loop {
        match (la.next(), lb.next()) {
            (None, None) => return true,
            (Some(x), Some(y)) if x == y => {}
            _ => return false,
        }
    }
}

/// All matches left to right, non-overlapping; empty matches advance
/// one char (the `cpo+=c` walk the corpus pins).
pub(crate) fn visit_matches(
    m: &Matcher<'_>,
    prog: &Program,
    mut visit: impl FnMut(SearchMatch) -> std::ops::ControlFlow<()>,
) -> Result<(), QueryError> {
    let mut at = 0usize;
    while at <= m.len() {
        match m.find_at(prog, at)? {
            Some((start, end)) => {
                if (start < m.len() || start < end)
                    && visit(SearchMatch {
                        start: ByteOffset::new(start),
                        end: ByteOffset::new(end),
                    })
                    .is_break()
                {
                    return Ok(());
                }
                at = if end > start {
                    end
                } else {
                    m.next_boundary(start)
                };
            }
            None => at = m.next_boundary(at),
        }
    }
    if m.query.cancelled() {
        Err(QueryError::Cancelled)
    } else {
        Ok(())
    }
}

/// First match from a scan start; attempts advance one char.
pub(crate) fn first_from(
    m: &Matcher<'_>,
    prog: &Program,
    from: usize,
) -> Result<Option<SearchMatch>, QueryError> {
    let mut at = from.min(m.len());
    while at <= m.len() {
        if let Some((s, e)) = m.find_at(prog, at)? {
            return Ok(Some(SearchMatch {
                start: ByteOffset::new(s),
                end: ByteOffset::new(e),
            }));
        }
        at = m.next_boundary(at);
    }
    if m.query.cancelled() {
        Err(QueryError::Cancelled)
    } else {
        Ok(None)
    }
}
