//! The breadcrumb's index against the obvious answer (0107 §10.1): every
//! byte of thousands of random span sets, edits at span boundaries, and
//! a carried index against one rebuilt from the real outline.

use std::collections::HashMap;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;

use ropey::Rope;
use strop_core::id::{BufferRevision, LineIndex};
use strop_core::worker::{Ticket, WorkerId};
use strop_core::{Buffer, Range};
use strop_lsp::{LineSpan, ProtoSymbol, ServerColumn, ServerLocation, ServerPosition};
use strop_syntax::languages::LanguageId;

use super::outline::{self, OutlineKey, Work};
use super::race_tests::document;
use super::scope::{cells, Scopes, MAX_DEPTH, SCOPE_CAP};
use super::store::Producer;
use super::SymbolState;

/// splitmix64: seeded, so a failure names its case.
struct Rng(u64);

impl Rng {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        if n == 0 {
            0
        } else {
            (self.next() % n as u64) as usize
        }
    }
}

fn symbol(name: String, line: usize) -> ProtoSymbol {
    ProtoSymbol {
        name,
        container: String::new(),
        kind: "Function".into(),
        location: ServerLocation {
            doc: strop_workspace::ResourceLocation::local("/w/a.rs".into()),
            position: ServerPosition {
                line: LineIndex::new(line),
                column: ServerColumn::new(0),
            },
        },
        span: LineSpan::line(LineIndex::new(line)),
    }
}

fn named(count: usize) -> Arc<Vec<ProtoSymbol>> {
    Arc::new((0..count).map(|at| symbol(format!("s{at}"), 0)).collect())
}

fn index(ranges: &[(usize, usize)]) -> Scopes {
    Scopes::from_ranges(
        document(0),
        Producer::Outline,
        BufferRevision::new(0),
        named(ranges.len()),
        ranges.to_vec(),
    )
}

/// The innermost symbol at `byte`, by the index.
fn innermost(scopes: &Scopes, byte: usize) -> Option<usize> {
    let mut chain = [0; MAX_DEPTH];
    let len = scopes.path(byte, &mut chain);
    (len > 0).then(|| chain[len - 1] as usize)
}

/// The obvious answer: the smallest range that contains the byte.
fn smallest(ranges: &[(usize, usize)], byte: usize) -> Option<(usize, usize)> {
    ranges
        .iter()
        .copied()
        .filter(|&(start, end)| start <= byte && byte < end)
        .min_by_key(|&(start, end)| end - start)
}

/// A random nested set inside `lo..hi`: children share starts and ends
/// with their parent, siblings touch, some are empty.
fn grow(rng: &mut Rng, lo: usize, hi: usize, depth: usize, out: &mut Vec<(usize, usize)>) {
    if depth == 0 || out.len() >= 200 {
        return;
    }
    let mut at = lo;
    for _ in 0..rng.below(4) {
        if out.len() >= 200 || at > hi {
            return;
        }
        let start = (at + rng.below(3)).min(hi);
        let end = (start + rng.below(hi - start + 1)).min(hi);
        out.push((start, end));
        grow(rng, start, end, depth - 1, out);
        at = end;
    }
}

fn shuffled<T>(rng: &mut Rng, mut items: Vec<T>) -> Vec<T> {
    for at in (1..items.len()).rev() {
        items.swap(at, rng.below(at + 1));
    }
    items
}

#[test]
fn the_lookup_agrees_with_a_linear_scan_at_every_byte() {
    let mut rng = Rng(0x0107);
    for case in 0..2_000 {
        let len = 1 + rng.below(300);
        let mut ranges = Vec::new();
        let depth = 1 + rng.below(12);
        grow(&mut rng, 0, len, depth, &mut ranges);
        let ranges = shuffled(&mut rng, ranges);
        let scopes = index(&ranges);
        for byte in 0..=len {
            let found = innermost(&scopes, byte).map(|symbol| ranges[symbol]);
            assert_eq!(
                found,
                smallest(&ranges, byte),
                "case {case}, byte {byte}, ranges {ranges:?}"
            );
        }
    }
}

#[test]
fn a_half_overlapping_span_is_clamped_to_the_parent_it_starts_in() {
    let scopes = index(&[(0, 10), (5, 15), (12, 20)]);
    assert_eq!(
        scopes.spans(),
        [(0, 0, 10, None), (1, 5, 10, Some(0)), (2, 12, 20, None)]
    );
    let clamped = [(0, 10), (5, 10), (12, 20)];
    for byte in 0..=21 {
        assert_eq!(
            innermost(&scopes, byte).map(|symbol| clamped[symbol]),
            smallest(&clamped, byte),
            "byte {byte}"
        );
    }
    let mut chain = [0; MAX_DEPTH];
    assert_eq!(scopes.path(7, &mut chain), 2);
    assert_eq!(chain[..2], [0, 1], "outermost first");
}

#[test]
fn a_chain_deeper_than_the_walk_shows_nothing() {
    let ranges: Vec<(usize, usize)> = (0..=MAX_DEPTH).map(|depth| (depth, 1_000)).collect();
    let scopes = index(&ranges);
    assert_eq!(innermost(&scopes, MAX_DEPTH - 1), Some(MAX_DEPTH - 1));
    assert_eq!(innermost(&scopes, 500), None);
}

/// Spans are sorted, inside their parent, pairwise nested or disjoint,
/// and inside the buffer.
fn assert_valid(scopes: &Scopes, len: usize, context: &str) {
    let spans = scopes.spans();
    let by_symbol: HashMap<u32, (usize, usize)> = spans
        .iter()
        .map(|&(symbol, start, end, _)| (symbol, (start, end)))
        .collect();
    for pair in spans.windows(2) {
        assert!(pair[0].1 <= pair[1].1, "sorted by start: {context}");
    }
    for &(_, start, end, parent) in &spans {
        assert!(start <= end && end <= len, "inside the buffer: {context}");
        if let Some(parent) = parent {
            let (outer_start, outer_end) = by_symbol[&parent];
            assert!(
                outer_start <= start && end <= outer_end,
                "inside its parent: {context}"
            );
        }
    }
    for (at, &(_, a_start, a_end, _)) in spans.iter().enumerate() {
        for &(_, b_start, b_end, _) in &spans[at + 1..] {
            let nested =
                (a_start <= b_start && b_end <= a_end) || (b_start <= a_start && a_end <= b_end);
            let disjoint = a_end <= b_start || b_end <= a_start;
            assert!(nested || disjoint, "nested or disjoint: {context}");
        }
    }
}

/// Text of `lines` short lines, and a nested set of line spans over it.
fn fixture(rng: &mut Rng) -> (String, Vec<ProtoSymbol>) {
    let lines = 2 + rng.below(30);
    let text: String = (0..lines)
        .map(|line| format!("{}\n", "ab".repeat(rng.below(4) + line % 3)))
        .collect();
    let mut ranges = Vec::new();
    let depth = 1 + rng.below(6);
    grow(rng, 0, lines - 1, depth, &mut ranges);
    // few lines make many equal spans, and equal spans nest: keep the
    // chains under the walk's depth
    ranges.sort_unstable();
    ranges.dedup();
    let symbols = ranges
        .into_iter()
        .enumerate()
        .map(|(at, (first, last))| ProtoSymbol {
            span: LineSpan {
                first: LineIndex::new(first),
                last: LineIndex::new(last),
            },
            ..symbol(format!("s{at}"), first)
        })
        .collect();
    (text, symbols)
}

fn build(buffer: &Buffer, list: Arc<Vec<ProtoSymbol>>) -> Scopes {
    Scopes::build(
        document(0),
        Producer::Outline,
        buffer.revision(),
        list,
        buffer.text(),
    )
}

#[test]
fn edits_keep_the_index_sorted_nested_and_inside_the_buffer() {
    let mut rng = Rng(0x5c0e);
    for case in 0..500 {
        let (text, symbols) = fixture(&mut rng);
        let mut buffer = Buffer::from_text(&text);
        let mut scopes = build(&buffer, Arc::new(symbols));
        for step in 0..12 {
            let spans = scopes.spans();
            let len = buffer.len_bytes();
            // at a span boundary half of the time, anywhere otherwise
            let at = match spans.get(rng.below(spans.len() * 2)) {
                Some(&(_, start, end, _)) => {
                    [start, end, start + 1, end.saturating_sub(1)][rng.below(4)].min(len)
                }
                None => rng.below(len + 1),
            };
            let to = (at + rng.below(4)).min(len);
            let typed = ["", "x", "\n", "yy\n"][rng.below(4)];
            buffer.clear_changes();
            buffer
                .edit()
                .replace(Range::charwise(at, to), typed)
                .unwrap();
            // the byte typed inside a body is inside every span it was in
            let inside: Vec<u32> = spans
                .iter()
                .filter(|&&(_, start, end, _)| start < at && at < end && at == to)
                .map(|&(symbol, ..)| symbol)
                .collect();
            // a byte typed at a span's end (after its closing line) is not
            let after: Vec<u32> = spans
                .iter()
                .filter(|&&(_, start, end, _)| start < end && at == end && at == to)
                .map(|&(symbol, ..)| symbol)
                .collect();
            assert!(scopes.apply(buffer.changes()));
            let context = format!(
                "case {case} step {step}: {at}..{to} <- {typed:?}, {spans:?} -> {:?}",
                scopes.spans()
            );
            assert_valid(&scopes, buffer.len_bytes(), &context);
            // and the lookup still agrees with a scan of the mapped spans
            let mapped: HashMap<usize, (usize, usize)> = scopes
                .spans()
                .iter()
                .map(|&(symbol, start, end, _)| (symbol as usize, (start, end)))
                .collect();
            let ranges: Vec<(usize, usize)> = mapped.values().copied().collect();
            for byte in 0..=buffer.len_bytes() {
                assert_eq!(
                    innermost(&scopes, byte).map(|symbol| mapped[&symbol]),
                    smallest(&ranges, byte),
                    "byte {byte}: {context}"
                );
            }
            if typed.is_empty() {
                continue;
            }
            for (symbol, start, end, _) in scopes.spans() {
                if inside.contains(&symbol) {
                    assert!(start <= at && at < end, "typed inside: {context}");
                }
                if after.contains(&symbol) {
                    assert!(end <= at, "typed after: {context}");
                }
            }
        }
    }
}

const RUST: &str = "\
struct Fixture {
    a: u8,
}

impl Fixture {
    fn new() -> Self {
        let a = 1;
        Self { a }
    }

    fn get(&self) -> u8 {
        self.a
    }
}

fn free() {
    let x = 1;
}
";

/// The real outline of `text`.
fn outline(text: &str) -> Arc<Vec<ProtoSymbol>> {
    let mut extractors = HashMap::new();
    let work = Work {
        ticket: Ticket {
            request: WorkerId::new(1),
            key: OutlineKey {
                document: document(0),
                revision: BufferRevision::new(0),
            },
        },
        rope: Rope::from_str(text),
        language: LanguageId::Rust,
        location: strop_workspace::ResourceLocation::local("/w/a.rs".into()),
        cancel: Arc::new(AtomicBool::new(false)),
    };
    Arc::new(outline::extract(&mut extractors, &work).expect("an outline"))
}

fn names(scopes: &Scopes, byte: usize) -> Vec<String> {
    scopes.at(byte).names().map(str::to_owned).collect()
}

#[test]
fn a_carried_index_says_what_a_rebuilt_one_says_on_the_unedited_text() {
    let line_start = |text: &str, line: usize| -> usize {
        text.split_inclusive('\n').take(line).map(str::len).sum()
    };
    // a line above a function, a line inside it, a line after it
    for (line, typed) in [
        (4, "// above\n"),
        (7, "        let b = 2;\n"),
        (9, "    // after\n"),
    ] {
        let mut buffer = Buffer::from_text(RUST);
        let mut carried = build(&buffer, outline(RUST));
        let at = line_start(RUST, line);
        buffer.edit().insert(at, typed).unwrap();
        assert!(carried.apply(buffer.changes()));
        assert!(!carried.exact);
        let edited = buffer.text().to_string();
        let rebuilt = build(&buffer, outline(&edited));
        for old in 0..RUST.len() {
            let new = if old < at { old } else { old + typed.len() };
            assert_eq!(
                names(&carried, new),
                names(&rebuilt, new),
                "line {line}, byte {old} ({:?})",
                &RUST[old..=old]
            );
        }
    }
}

#[test]
fn the_slot_is_the_widest_chain_and_capped() {
    let chain = |ranges: &[(usize, usize)], names: &[&str]| {
        let list = names.iter().map(|name| symbol((*name).into(), 0)).collect();
        Scopes::from_ranges(
            document(0),
            Producer::Outline,
            BufferRevision::new(0),
            Arc::new(list),
            ranges.to_vec(),
        )
        .slot()
    };
    // the deepest chain is not always the widest: the widest wins
    assert_eq!(
        chain(&[(0, 10), (2, 5), (6, 9)], &["Outer", "inner", "innermost"]),
        cells("Outer › innermost")
    );
    let wide = "x".repeat(30);
    assert_eq!(chain(&[(0, 4), (1, 3)], &[&wide, &wide]), SCOPE_CAP);
    // a segment counts at most its cut width
    let long = "y".repeat(33);
    assert_eq!(chain(&[(0, 4), (1, 2)], &["a", &long]), 1 + 3 + 32);
}

#[test]
fn a_revision_gap_the_work_cap_and_a_close_each_drop_the_index() {
    let mut buffer = Buffer::from_text(RUST);
    // a gap: the index never saw the first edit
    let mut state = SymbolState {
        scope: Some(build(&buffer, outline(RUST))),
        ..SymbolState::default()
    };
    buffer.edit().insert(0, "a").unwrap();
    buffer.clear_changes();
    buffer.edit().insert(0, "b").unwrap();
    state.edited(document(0), buffer.changes());
    assert!(state.scope.is_none(), "a revision gap drops it");

    // too much work: changes × spans over the cap
    let wide: Vec<(usize, usize)> = (0..1_100).map(|at| (at, at + 1)).collect();
    let mut scopes = Scopes::from_ranges(
        document(0),
        Producer::Outline,
        buffer.revision(),
        named(wide.len()),
        wide,
    );
    buffer.clear_changes();
    for _ in 0..1_000 {
        buffer.edit().insert(0, "c").unwrap();
    }
    assert!(!scopes.apply(buffer.changes()), "the work cap drops it");

    // an edit of another document leaves it; closing its own drops it
    state.scope = Some(build(&buffer, outline(RUST)));
    state.edited(document(1), buffer.changes());
    assert!(state.scope.is_some());
    state.closed(document(0));
    assert!(state.scope.is_none(), "closing the document drops it");
}
