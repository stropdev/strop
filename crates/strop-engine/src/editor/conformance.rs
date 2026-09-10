//! Conformance harness (0024): a reference model (plain String) and the
//! real editor driven by the SAME generated operation streams through
//! the production headless path. Every step asserts text equality plus
//! the protocol invariants the TLA+ spec proves at the model level:
//! panes reference live docs (NoStalePane), the revision never goes
//! back (RevisionTracksPublications — the epoch counts publications,
//! undo included), anchors stay inside the text on char boundaries.
//! The transaction/ticket boundary itself has its own oracle module:
//! transaction_conformance.rs (R12).
//!
//! Deterministic: a seeded xorshift, no wall clock, no I/O.

use super::*;
use strop_core::Buffer;

/// Seeded PRNG (xorshift64) — the stream is reproducible by seed.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        self.0 = x;
        x
    }
    fn below(&mut self, n: usize) -> usize {
        (self.next() % n.max(1) as u64) as usize
    }
}

/// The reference model: text as a String, cursor as a byte offset, one
/// undo stack of full-text snapshots (the model's history is exact).
struct Model {
    text: String,
    cursor: usize,
    undo: Vec<String>,
    /// Text as of the last session boundary (vim's undo target).
    boundary: String,
}

impl Model {
    fn clamp(&mut self) {
        if self.cursor > self.text.len() {
            self.cursor = self.text.len();
        }
        while self.cursor > 0 && !self.text.is_char_boundary(self.cursor) {
            self.cursor -= 1;
        }
    }
    fn insert(&mut self, s: &str) {
        self.clamp();
        self.text.insert_str(self.cursor, s);
        self.cursor += s.len();
    }
    fn backspace(&mut self) {
        self.clamp();
        if self.cursor > 0 {
            let prev = self.text[..self.cursor].chars().last().unwrap().len_utf8();
            self.text.replace_range(self.cursor - prev..self.cursor, "");
            self.cursor -= prev;
        }
    }
    /// vim's undo unit is the insert SESSION: at a boundary, the undo
    /// target is the text AS OF the previous boundary.
    fn mark(&mut self) {
        self.undo.push(self.boundary.clone());
        self.boundary = self.text.clone();
    }
    fn undo(&mut self) {
        if let Some(t) = self.undo.pop() {
            self.text = t;
            self.cursor = self.cursor.min(self.text.len());
            self.clamp();
            // the restored state IS the new session's start point
            self.boundary = self.text.clone();
        }
    }
    fn left(&mut self) {
        self.clamp();
        if self.cursor > 0 {
            self.cursor -= self.text[..self.cursor].chars().last().unwrap().len_utf8();
        }
    }
    fn right(&mut self) {
        self.clamp();
        if self.cursor < self.text.len() {
            self.cursor += self.text[self.cursor..].chars().next().unwrap().len_utf8();
        }
    }
}

/// One generated operation stream, applied to both. Invariants checked
/// every step on the real editor.
fn run_stream(seed: u64, ops: usize) {
    let mut rng = Rng(seed);
    let mut model = Model {
        text: String::new(),
        cursor: 0,
        undo: Vec::new(),
        boundary: String::new(),
    };
    let mut e = Editor::new(Buffer::from_text(""));
    e.feed(Key::Esc); // dismiss the welcome card (first key is a card key)
    e.feed_text("i");
    // the epoch counts publications (typing, undo moves included): it
    // never goes back within a document's life (RevisionTracksPublications)
    let mut last_revision = e.buf().revision().get();
    for step in 0..ops {
        let choice = rng.below(100);
        if choice < 40 {
            let c = (b'a' + (rng.below(26)) as u8) as char;
            model.insert(&c.to_string());
            e.feed(Key::Char(c));
        } else if choice < 55 {
            model.backspace();
            e.feed(Key::Backspace);
        } else if choice < 65 {
            model.left();
            e.feed(Key::Left);
        } else if choice < 75 {
            model.right();
            e.feed(Key::Right);
        } else if choice < 82 {
            // session boundary: Esc closes the unit in both
            e.feed(Key::Esc);
            model.mark();
            // then undo one unit
            e.feed_text("u");
            model.undo();
            e.feed_text("i");
        } else {
            let c = (b'a' + (rng.below(26)) as u8) as char;
            model.insert(&c.to_string());
            e.feed(Key::Char(c));
        }
        let _ = step;
        // text equality — through the public accessor, never the field
        let got = e.buf().text().to_string();
        assert_eq!(
            got, model.text,
            "seed {seed} step {step}: editor and model diverged"
        );
        // invariants: cursor in bounds on a char boundary
        let h = e.head();
        assert!(h <= e.buf().len_bytes(), "cursor past the text");
        assert!(e.buf().is_boundary(h), "cursor mid-char");
        // the revision is the publication count: monotonic, never back
        let revision = e.buf().revision().get();
        assert!(
            revision >= last_revision,
            "seed {seed} step {step}: revision went back ({last_revision} -> {revision})"
        );
        last_revision = revision;
        // every pane references a live document (NoStalePane)
        for p in &e.panes {
            assert!(e.docs.get(p.doc).is_some(), "pane holds a stale doc id");
        }
    }
}

#[test]
fn conformance_generated_streams_match_the_model() {
    for seed in [1, 7, 42, 1337, 99991] {
        run_stream(seed, 200);
    }
}
