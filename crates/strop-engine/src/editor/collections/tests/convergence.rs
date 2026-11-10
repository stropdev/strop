//! Convergence (0094 §4): one generated stream of edits is applied twice —
//! through a collection in one editor, and directly to twin source buffers
//! in another. After every step the sources agree byte for byte, and the
//! collection's view is exactly what rendering its sources again would
//! give. Deterministic: a seeded xorshift, no wall clock.
use super::write::assert_view_is_its_sources;
use super::*;
use strop_core::frontend_input::Key;
use strop_core::id::DocumentId;

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

/// Edits that mean the same thing at a caret in the view and at the same
/// caret in the source: none of them reaches past the caret's own line.
const EDITS: &[&str] = &[
    "x",
    "rZ",
    "~",
    "D",
    "iin <esc>",
    "A tail<esc>",
    "I<esc>",
    "onew line<esc>",
    "Oabove<esc>",
    "o<esc>",
    "dd",
    "yyp",
    "ccwhole<esc>",
];

const FILES: usize = 3;

struct Twin {
    /// Edits through the collection.
    through: Editor,
    view: DocumentId,
    /// Edits in the sources directly.
    direct: Editor,
    /// `(through, direct)` ids of each source.
    sources: Vec<(DocumentId, DocumentId)>,
    /// Sources of the collection edits its own history can still undo.
    undo: Vec<usize>,
    /// What the last step did, for a failure's message.
    last: String,
}

fn twin(seed: u64) -> Twin {
    let dir = tempfile::tempdir().unwrap().keep();
    let mut through = Editor::new_in(Buffer::from_text("scratch\n"), dir.clone());
    let mut direct = Editor::new_in(Buffer::from_text("scratch\n"), dir.clone());
    let mut sources = Vec::new();
    let mut items = Vec::new();
    let mut rng = Rng(seed | 1);
    for file in 0..FILES {
        let path = dir.join(format!("f{file}.txt"));
        let mut text: String = (0..24)
            .map(|line| format!("file{file} line {line} word\n"))
            .collect();
        if file == FILES - 1 {
            text.pop(); // one source's last line has no newline
        }
        std::fs::write(&path, &text).unwrap();
        sources.push((
            through.open_fixture(&path).unwrap(),
            direct.open_fixture(&path).unwrap(),
        ));
        for line in [1 + rng.below(4), 9 + rng.below(4), 24] {
            items.push(Item {
                badge: None,
                text: format!("f{file}:{line}"),
                payload: Payload::Grep {
                    location: strop_workspace::ResourceLocation::local(path.clone()),
                    line,
                    col: 1,
                    match_len: 4,
                    line_text: format!("file{file} line {} word", line - 1).into(),
                },
            });
        }
    }
    through.open_picker(Kind::Search);
    through.picker_items_fixture(items);
    through.feed(Key::CtrlO);
    let view = through.current();
    assert!(through.collections.contains_key(&view));
    Twin {
        through,
        view,
        direct,
        sources,
        undo: Vec::new(),
        last: String::new(),
    }
}

impl Twin {
    fn index_of(&self, through: DocumentId) -> usize {
        self.sources
            .iter()
            .position(|(id, _)| *id == through)
            .unwrap()
    }

    /// One edit at a random caret inside a random excerpt of the view, and
    /// the same edit at the same source byte in the twin.
    fn edit_through(&mut self, rng: &mut Rng) {
        let keys = EDITS[rng.below(EDITS.len())];
        let (start, end) = {
            let excerpts = &self.through.collections[&self.view].excerpts;
            let excerpt = &excerpts[rng.below(excerpts.len())];
            (excerpt.view_start, excerpt.view_end)
        };
        if start == end {
            return;
        }
        self.through.switch_to(self.view);
        self.through.set_head(start + rng.below(end - start));
        self.through.clamp_cursor();
        let Some((source, offset)) = self.through.source_position(self.view, self.through.head())
        else {
            return;
        };
        let index = self.index_of(source);
        self.last += &format!("\n{keys:?} through the view at source {index} byte {offset}");
        let before = self.through.doc(source).buf.revision();
        type_keys(&mut self.through, keys);
        if self.through.doc(source).buf.revision() != before {
            self.undo.push(index);
        }
        self.direct.switch_to(self.sources[index].1);
        self.direct.set_head(offset);
        self.direct.clamp_cursor();
        type_keys(&mut self.direct, keys);
    }

    /// One edit in a source buffer, the same in both editors. The
    /// collection's history starts over.
    fn edit_direct(&mut self, rng: &mut Rng) {
        let keys = EDITS[rng.below(EDITS.len())];
        let index = rng.below(self.sources.len());
        let (through, direct) = self.sources[index];
        let offset = rng.below(self.through.doc(through).buf.len_bytes().max(1));
        self.last += &format!("\n{keys:?} in source {index} at byte {offset}");
        for (editor, document) in [(&mut self.through, through), (&mut self.direct, direct)] {
            editor.switch_to(document);
            editor.set_head(editor.buf().clamp_boundary(offset));
            editor.clamp_cursor();
            type_keys(editor, keys);
        }
        self.undo.clear();
    }

    /// `u` in the collection is `u` in the source its last edit changed.
    fn undo(&mut self, redo: bool) {
        self.through.switch_to(self.view);
        if !self.through.buf().history().can_undo() {
            self.undo.clear();
            return;
        }
        let Some(index) = self.undo.pop() else {
            return;
        };
        self.last += &format!("\nu (redo {redo}) of source {index}");
        type_keys(&mut self.through, "u");
        self.direct.switch_to(self.sources[index].1);
        type_keys(&mut self.direct, "u");
        if redo {
            self.through.feed(Key::CtrlR);
            self.direct.feed(Key::CtrlR);
            self.undo.push(index);
        }
    }

    fn check(&self, step: usize, seed: u64) {
        for (index, (through, direct)) in self.sources.iter().enumerate() {
            let through = self.through.doc(*through).buf.text().to_string();
            let direct = self.direct.doc(*direct).buf.text().to_string();
            if through == direct {
                continue;
            }
            let line = through
                .lines()
                .zip(direct.lines())
                .position(|(a, b)| a != b)
                .unwrap_or_else(|| through.lines().count().min(direct.lines().count()));
            let steps: Vec<&str> = self.last.lines().rev().take(4).collect();
            panic!(
                "seed {seed} step {step}: source {index} diverged at line {line}: \
                 through {:?}, direct {:?}; last steps (newest first): {steps:#?}",
                through.lines().nth(line),
                direct.lines().nth(line),
            );
        }
        assert_view_is_its_sources(&self.through, self.view);
    }
}

#[test]
fn edits_through_a_collection_and_edits_to_its_sources_converge() {
    for seed in 1..=6u64 {
        let mut twin = twin(seed);
        let mut rng = Rng(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1);
        twin.check(0, seed);
        for step in 1..=100 {
            match rng.below(10) {
                0..=5 => twin.edit_through(&mut rng),
                6..=7 => twin.edit_direct(&mut rng),
                8 => twin.undo(false),
                _ => twin.undo(true),
            }
            twin.check(step, seed);
        }
    }
}
