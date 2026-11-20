---
rfc: "2026-09-06-incremental-treesitter-nucleo"
title: "Incremental syntax parsing and picker scoring"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-06"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/118"
milestone: "v0.10.1"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0022"
---

# RFC: Incremental syntax parsing and picker scoring

v0.10.1 retained edited tree-sitter trees, reparsed rope chunks incrementally and adopted nucleo-matcher for picker scoring ([d182377](https://github.com/stropdev/strop/commit/d1823771504281397f90c9acdc6e74a9b694124f)). v0.10.2 corrected the chunk callback for nonsequential parser reads, and v0.10.3 corrected Unicode picker input.

[Tracking issue #118](https://github.com/stropdev/strop/issues/118) · [Published v0.10.1](https://github.com/stropdev/strop/releases/tag/v0.10.1) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0022-incremental-treesitter-nucleo.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: accepted 2026-09-06. The 1.0-hardening remainder. Perf bench
suite stays skipped (per the author); this cycle is the two real
items left.

## 1. Incremental tree-sitter

Today: every revision reparses the whole visible window with no old
tree, and the source string is materialized wholesale. Tree-sitter
supports exactly what we already record:

- `History` ops are `{at, text, kind}` — that IS `InputEdit` (start /
  old_end / new_end byte offsets).
- The highlighter keeps the old `Tree`; on each edit transaction the
  document applies the ops as `InputEdit`s to the old tree, then
  re-parses from rope chunks with the old tree as the base.

Design:

- `strop-syntax::Highlighter` gains `tree: Option<Tree>` and
  `revision: u64` (the revision its tree covers).
- On `highlight(...)`: if `revision != self.revision`, apply the
  recorded edits between the revisions as `InputEdit`s (the history
  supplies them; a full replace falls back to a fresh parse), parse
  with `Some(old_tree)`, then run the spans query on the result.
- Parse input comes from rope chunks (`ropey::Rope::chunks`), never
  a materialized String of the document.

Correctness gate (this is the contract, speed is the side effect):
incremental results must equal a fresh full parse — property test over
a corpus of edit sequences (insert/delete/undo/redo/surround/paste)
comparing span sets.

## 2. The nucleo decision, with numbers

Not a swap on principle — a measurement:

- Build a small bench (dev-dependency `criterion` or a timed example):
  the picker's subsequence scorer vs `nucleo-matcher` on realistic
  workloads — 10k/50k/100k file-list items, query lengths 2–16,
  measuring full refilter time after one appended batch (the real
  hot path: refilter per keystroke over accumulated items).
- Compare correctness surface too: scoring parity on the cases the
  picker's own tests pin (camel/subsequence/boost-by-recency, match
  column outputs for highlighting).
- Decision rule: adopt nucleo only if it is meaningfully faster on
  the 100k refilter AND its match-column output is usable for our
  row highlighting. Otherwise keep the 60-line scorer and record why.

## Non-goals

- The perf bench suite (skipped per the author).
- Any user-visible feature change: highlighting looks identical, the
  picker feels identical — only the work behind them changes.

## Exit criteria

- Highlighter reuses the old tree across edit transactions; spans
  equal a fresh parse on a property corpus; no `to_string()` on the
  document in the highlight path.
- `docs/` or the changelog records the nucleo numbers and the
  decision; the roadmap/site reflect both.
- Gate green; no new file over the ceiling; types stay named
  (InputEdit bridge gets a struct, not tuples).
