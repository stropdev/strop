---
rfc: "2026-11-17-lean-and-correct"
title: "Harden editor operations and make release and proof gates block"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-11-17"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/204"
milestone: "v0.71.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0108"
---

# RFC: Harden editor operations and make release and proof gates block

PR [#45](https://github.com/stropdev/strop/issues/45) shipped regression fixes for editing, saves, terminal exit and Git reads; enforced LF-based line boundaries; expanded the differential coverage; and required local package qualification. The last TLC models gave way to tests against the Rust implementations; map_span/keep_disjoint and claim-specific calibration strengthened the Lean gate. The cleanup removed old flags, wire/session readers and dependency version pins, not the updater, grammar set or libgit2.

[Tracking issue #204](https://github.com/stropdev/strop/issues/204) · [Published v0.71.0](https://github.com/stropdev/strop/releases/tag/v0.71.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0108-lean-and-correct.md).

Follow-up issues: [#53](https://github.com/stropdev/strop/issues/53), [#54](https://github.com/stropdev/strop/issues/54), [#55](https://github.com/stropdev/strop/issues/55), [#56](https://github.com/stropdev/strop/issues/56), [#57](https://github.com/stropdev/strop/issues/57), [#58](https://github.com/stropdev/strop/issues/58), [#60](https://github.com/stropdev/strop/issues/60), [#69](https://github.com/stropdev/strop/issues/69), [#70](https://github.com/stropdev/strop/issues/70), [#71](https://github.com/stropdev/strop/issues/71), [#72](https://github.com/stropdev/strop/issues/72).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

- Status: **in force** (0.71).
- Source: the owner — "I need to make sure the editor is hardened as much as
  possible, see if there is any clutter or bloat floating around, we need to
  make it lean and correct." Three independent audits (proofs/specs, runtime
  hardening, clutter) are appended verbatim as §A–§C; this header is the
  decision record and wins where they differ.
- Builds on 0086 (ring design, survival rule), 0099 (proved caches), 0079
  (local release). Amends AGENTS.md (TLC probation ends, version bump is one
  line), 0086 (mutants calibrate properties, not only bridges).

## Decisions

### D1. Crashes and data hazards (fix + regression test each)
1. `registers/put.rs` linewise `p` below a last line without a break sliced
   the register by the prepended break: blanks are measured on the register
   text itself (`yyp` on `étude` panicked; ASCII landed one column off).
2. Insert-mode `Ctrl-W`/`Ctrl-U` floor: the insert origin is carried through
   every edit (or clamped to a char boundary) — `A <Left> é Ctrl-W` panicked.
3. New files honour the umask (0666 & !umask), not the stage's 0600.
4. The startup file goes through the same guarded open as `:e` (devices,
   FIFOs, non-UTF-8 refused or handled exactly like `:e`).
5. SIGTERM/SIGHUP restore the terminal and take the final checkpoint.
6. git child processes run with hostile-config overrides (no fsmonitor, no
   hooks, no textconv/external diff, no pager).
7. `:g` maps marks once per pass, not once per edit, and `Ctrl-C` stops it
   between lines.
8. `notify` reconcile does no `canonicalize` per hint on the event loop.
9. ropey's `unicode_lines` is off: a line break is `\n` (CRLF stays one
   break), as in vim and tree-sitter.
10. `transact.rs` mark deletion uses per-edit coordinates; an insertion at
    an excerpt's end does not grow the excerpt.
Deferred (recorded in the roadmap): in-place write for hard links and
read-only parents; macro-replay speed beyond `Ctrl-C`.

### D2. A key fuzzer that blocks
A fixed-seed, in-process random-key test (no panic; cursor on a char
boundary; undo to root restores the text) over multibyte, CRLF and
no-final-newline fixtures, run by `cargo test` with a bounded step budget.
The differential corpus gains the same three fixture shapes.

### D3. Gates
- `specs/` (both TLC models) and the `model` lane's TLC part are deleted: no
  recorded catch, 15 releases past probation. Replaced by a Loom test of the
  worker's cancel/finish exactly-once and an exhaustive sequence test of the
  picker generation guard.
- The Loom lane fails when it matches zero tests.
- `release-local.sh package` refuses unless `qualify` passed for the same
  commit.
- Tile premises are `debug_assert`ed at the call site.

### D4. Proofs
- `editmap::map_span` (proved) replaces the hand-rolled span mappers in the
  engine; the remaining position mappers route through `map_through_edits`.
  `editmap.rs`'s "single mapper" claim becomes true.
- `keep_disjoint` (multicursor plan filter) moves into the kernel, proved to
  produce a batch `validate_batch` accepts.
- Mutants calibrate property theorems, not only bridge lemmas: every claim
  in `Audit.lean` has a mutant that breaks it, and the audit refuses an
  extracted `pub fn` without a claim.
- Deferred: `operated_span`, viewport clamps, mark fold (roadmap).

Implementation record (D4, D1.10):
- `editmap::map_span` is the one span rule; `map_span_spec` states its
  properties over the extracted function (ordered, in bounds, shift before,
  untouched after, exact delta inside, start left-affine) and
  `map_span_keeps_order` that two ordered spans stay ordered unless the edit
  straddles the second one's start or the first claims an insertion there.
  Excerpt spans (`transact.rs`), view bodies (`collections/updates.rs`) and
  the completion journal's windows (`words/journal.rs`, whose `map_edge` was
  exactly `map_span` with the end claiming) map through it. Structural
  re-indent folds `map_span` over the edits that applied (a position in a
  re-indented prefix still lands after the new indentation); mirrored
  inserts fold `map_through_edits` over the inserts that applied (B5).
- `editmap::keep_disjoint` (`keep_disjoint_spec`): a range is kept exactly
  when it starts at or after the end of every range kept before it, and the
  kept non-empty ranges are `BatchOk`. The proof needs no sortedness:
  sorting only decides which cursor wins.
- D1.10, as built: marks are judged per edit (its own `start_point` column,
  the mark followed through the edits before it). An edit at an excerpt's
  end in the source does not grow it — typing or replacing at column 1 of
  the line below (B4) — except when the view itself wrote it through that
  excerpt (`o` on its last line, its undo restoring a line, typing there:
  `Collection::writing`) or the end is mid-line (a last line without a
  newline goes on). Read literally, "an insertion never grows it" would
  make the view's own `o` drift from its source. A clean view rebuilds from
  its sources for an insertion at an excerpt's end instead of deciding the
  rule a second time.
- Calibration: `Audit.lean` prints `CLAIM_DEPS` (the kernel theorems each
  claim rests on) and refuses a public kernel function absent from every
  claim's statement (`layout::empty` was deleted as weightless;
  `cache::vacant`, `holds`, `selects` gained claims). `mutants.py` refuses
  a claim no mutant breaks; for a property claim (not a `_spec`) the
  failure must reach a property lemma, so paired mutants patch the Rust
  and the Lean model together (cache, edit map, batch admission, excerpt
  translation, tiling). `total_cost_le` is a lemma about the cost model
  only, so its mutant patches the model alone.

### D5. Bloat
- `[profile.release]`: `lto = "fat"`, `codegen-units = 1`, `panic` unchanged.
- Delete: unreferenced pub items, dead `cfg(windows)`/`cfg(not(unix))`
  branches, the CLI `--flag` synonyms of subcommands (subcommands stay),
  legacy wire/session readers (alpha: old sessions may break), dead update
  channels, unused dev-deps, the duplicate `&self` preview ladder, path-dep
  version pins (a version bump is the workspace `Cargo.toml` + `cargo check`).
- Kept (owner questions defaulted conservative): `strop update`, every
  grammar, libgit2, the handoff/critique plans (marked superseded in STATUS).
- `STATUS.md` corrected; over-ceiling files split.

Implementation record (D5): no file is past the ceiling (the two the audit
named were split earlier this round). `strip` stays in the Dockerfile's
`release` stage. Beyond the audit's lists, the stored-query module (never
persisted, with its v1–v3 readers), the session/attach/DocPath compatibility
defaults and `--log=ALL` went too. Left for the roadmap (0028): the
`too_many_arguments` allows, the test PRNGs, retired-plan citations in
comments, the crossterm/ratatui bump.

---

## §A. Audit: proofs and specs


Read-only audit. Nothing was built or run: no `lake build`, no TLC, no cargo. Every statement about what a proof or gate does comes from reading the sources. Paths are relative to `/home/tarek/workspace/strop`.

Evidence labels: **[V]** = I read the code myself. **[A]** = reported by a sweep sub-agent and not re-read by me.

## Verdict in one paragraph

The Lean layer is honest but narrow. Its 34 claims are real theorems over the Aeneas translation, with no `sorry`, no project axioms and no `native_decide`, and production calls every kernel function. Three things are weaker than the documents say. First, the mutant calibration only tests the bridge lemmas ("code = model"), never the property theorems. Second, the kernel's "only way anchors move" claim is false: the engine has five hand-rolled span/point mappers, and one of them has a real bug. Third, the `proofs` lane blocks a release only by convention. The two TLA+ models have no recorded catch, are 15 releases past their probation date, and should be deleted. The sweep also turned up product bugs in exactly the kind of arithmetic the kernel is meant for; they are listed in section 2.0.

---

## 1. Soundness of what exists

### 1.1 What holds [V]

- **No escape hatches.** A grep of `proofs/Kernel` and `proofs/Extracted` for `sorry`, `axiom`, `admit`, `native_decide`, `implemented_by`, `ofReduceBool`, `opaque`, `partial` finds nothing. The only hits are the word "admits" in Excerpt.
- **Axiom audit is transitive.** `proofs/Audit.lean:68-80` runs `collectAxioms` on every theorem under `Strop.Kernel` and fails on anything outside `{propext, Classical.choice, Quot.sound}`. An Aeneas-emitted axiom for an unmodelled external would be caught as soon as a theorem depended on it.
- **Claims are backed.** All 34 names in `Audit.lean:21-56` exist as theorems in `proofs/Kernel/*.lean` (71 theorems total; cross-checked mechanically). A missing claim fails the audit (`Audit.lean:77-79`).
- **Theorems are about extracted code.** Every function-level spec is a Hoare triple over the extracted definition, for example:
  - `find_spec` (`Cache.lean:88`)
  - `place_spec` (`Cache.lean:369`)
  - `validate_batch_spec` (`EditMap.lean:226`)
  - `map_through_edits_composition` (`EditMap.lean:463`)
  - `source_edit_spec` (`Excerpt.lean:84`)
  - `tile_in_bounds` / `tile_exact` / `tile_minimum` / `tile_fair` (`Layout.lean:536-628`)
- **The translation cannot drift.** `proofs/gate.sh:31-37` regenerates the translation and diffs it against `proofs/Extracted`. `gate.sh:11-27` refuses a `pub mod` that Charon's `start_from` does not name.
- **Every public kernel function has a spec.** All 18 have a `_spec` theorem; only `vacant` and `empty` (constructors) do not.
- **Production calls the kernel (moved, not copied):**

  | Kernel function | Production caller |
  |---|---|
  | `generation::{next_generation, index_for_len}` | `crates/strop-core/src/id.rs:13` |
  | `editmap::validate_batch` | `crates/strop-core/src/buffer/mutation.rs:192`; `crates/strop-engine/src/editor/completion/acceptance.rs:374` |
  | `editmap::map_through_edits` | `editor/transact.rs:92`; `editor/snippet.rs:209`; `editor/collections/view_positions.rs:141` |
  | `cache::*` | `editor/cache.rs:12` |
  | `excerpt::source_edit` | `editor/collections/write.rs:58` |
  | `layout::{tile, minimum}` | `editor/panes/layout.rs:234,280` |

- **The cache wrapper is tight [A].** `get` is the only read path and goes through `find`. Values are swept after every retire or place, with a `debug_assert` that `live == value.is_some()`.
- **Hypotheses are satisfiable.** `TileBounded`, `TileFits`, `Chain3`, `Admitted` and the `hbound` premises are all inhabited by the kernel's own unit tests. I found no vacuous statement.

### 1.2 Weaknesses

**S1. Mutants calibrate the bridge, never the claims. [V]**
- `proofs/mutants.py:269-275` requires each mutant to fail exactly its named theorem and no other.
- All 27 mutants name a function-level spec or loop lemma. Those are the theorems that say "the code computes model X": `holds_spec`, `selects_spec`, `map_position_spec`, `map_through_edits_loop_spec`, `minimum_spec`, `share_spec`, `total_spec`, `tile_loop_body_spec`, and so on.
- The property theorems are stated over the models (`mapSpec`, `journalSpec`, `tileFrom`, `Placed`, `toSource`). By construction no mutant can reach them:
  - `mapSpec_monotone`, `mapSpec_in_bounds`
  - `map_through_edits_monotone`, `map_through_edits_composition`, `consecutive_of_batchOk`
  - `find_after_retire`, `find_after_place`, `place_evicts_oldest`, `placed_bounded`, `total_cost_le`
  - `tile_in_bounds`, `tile_exact`, `tile_minimum`, `tile_fair`, `tile_fair_leaves`
  - `toSource_in_span`, `source_edit_commutes`
- These are still sound: property-of-model plus code-equals-model gives property-of-code. But "one mutant per theorem family that must break it" (0086 §2, AGENTS.md) is not what the gate checks for them. If someone weakened `tile_exact` to a tautology, or changed `tileFrom` and `tile` together, every mutant would still be "rejected".
- Three models are near-restatements of the code: `mapSpec` (`EditMap.lean:39`), `journalSpec` (`:378`) and `tileFrom` (`Layout.lean:122`, described in its own header as "the loop itself over a list"). That is the pattern 0085 criticised in the Verus kernels. Here it is acceptable only because real properties are proved on top, which is exactly why those properties need calibration.
- 14 of the 34 claims are named by a mutant; 20 are not.
- Fix: for each property family, add one paired mutant that patches the Rust and the Lean model consistently and must fail the property. For example, patch `least(..., left)` to unclamped in both `tile` and `tileFrom`; that must fail `tile_in_bounds`. Alternatively, state two or three headline properties directly over the extracted function without the model.

**S2. Uncalibrated behaviours inside calibrated functions. [V]**
No mutant covers any of these:
- `find` starting at slot 1, or returning the last match.
- `victim` not preferring a dead slot.
- `place` writing a wrong `used` or `version`.
- `validate_batch` with the `Unsorted` test dropped.
- `touch` with the index guard dropped.

The specs would catch each one. The gate just never demonstrates it.

**S3. Nothing forces a new function to be claimed. [V]**
- A new `pub fn` in an existing kernel module is translated and drift-checked, but ships unproved with a green gate.
- `Audit.lean` could enumerate the `strop_kernel.*` definitions and require each non-instance one to occur in some claim's statement. That is roughly 15 lines of metaprogram.

**S4. Theorem premises are not enforced at the call site. [V]/[A]**
- `tile_exact`, `tile_minimum` and `tile_fair` need `TileFits`: every track ≥ 1, a non-empty list, and the minimum fitting.
- `editor/panes/layout.rs:55-70` `tracks()` can return 0 for a childless split (`each.sum()`).
- `well_formed` is only `debug_assert`ed after split and remove [A]. `Layout` derives `Deserialize` (`layout.rs:26`).
- The release profile has no `overflow-checks` (no `[profile]` section in `Cargo.toml`). Outside `TileBounded`, the Rust wraps where the Lean model fails.
- `cells` is safe by type (`u16`).
- Fix: `debug_assert!(tracks.iter().all(|t| *t >= 1) && !tracks.is_empty())` at `layout.rs:234`.

**S5. Doc comments overclaim. [V]**
- `crates/strop-kernel/src/editmap.rs:1-4` says "every selection/view remap call these exact functions". That is false; see section 2.1.
- `crates/strop-kernel/src/excerpt.rs:21-24` says "carets, marks and later excerpts agree in both buffers". `source_edit_commutes` (`Excerpt.lean:122-128`) assumes `keeps = true`, `e - vs ≤ se - ss` and `p - vs ≤ se - ss`. So it excludes every edit or anchor touching the added terminator, and it is a pure `Nat` lemma, not a statement over the extracted function.
- The cache header calls `length × limit` the memory bound. The only user passes `cost = symbols.len()` against 20,000 [A], so it is a count bound.
- `crates/strop-picker/src/source/lifecycle_traces.rs:6-7` calls `strop_core::searchguard` "the verified kernel". It is an unproved `==`.

**S6. Small duplicates and dead weight.**
- `crates/strop-core/src/id.rs:131-138` `insert_capacity` re-derives the retire predicate (`generation < u32::MAX`) and the index space next to the kernel. It is off by one against `index_for_len`, though only at about 2^32 slots [A].
- `crates/strop-core/src/buffer/mutation.rs:188-191`: the `debug_assert` on sort order runs immediately after the sort, so it can never fire [V].
- `searchguard`, `viewguard`, `mutguard`, `cohortguard` (124 lines in total, `crates/strop-core/src/*guard.rs`) are leftovers from the Verus era. They wrap `==`, `!=` and `<`. They are neither proved nor worth proving; inline them, per "delete weightless code".
- `crates/strop-core/src/selection.rs:171` `remap(at, delta)` is a dead shadow mapper with no non-test caller, and it maps deletions wrongly [A]. Delete it.
- `plans/0086` §4's ring-0 row still lists "frame-size admission", which was deleted in `0b670a9`.

---

## 2. Gaps

### 2.0 Bugs found during the sweep

| # | Bug | Evidence | Status |
|---|---|---|---|
| B1 | Linewise `p` below a last line with no trailing newline slices the register text by the length of the prepended newline. | `editor/registers/put.rs:60`: `&text[prefix.min(text.len())..]`, but `prefix` is not part of `text`. | [V] by reading; not run |
| B2 | Line-break disagreement between ropey and strop. | `ropey = "1"` with default features; ropey 1.6.1's default is `unicode_lines`, which implies `cr_lines`. `crates/strop-core/src/buffer.rs:322-327` strips only `\n` / `\r\n`. | [V] |
| B3 | Mark-deletion rule mixes coordinate systems. | `editor/transact.rs:150-164`: `line_starts` is computed on the post-batch buffer from each journal entry's own `start`, then compared with the mark's pre-batch offset. | [V] by reading |
| B4 | `map_excerpt_span` grows an excerpt when an edit starts exactly at its end. | `editor/transact.rs:254-262`: `excerpt.end == s < e` falls into the "covered" arm, so `end = s + new_len`. | [V] by reading |
| B5 | Mirrored insert remaps cursors even when an insert was refused. | `editor/insert.rs:314-317,328-331` ignore `insert`'s result, then `remap_after_mirrored_edit` shifts every cursor. Same pattern in `structural_indent.rs:403-432`. | [A] for the refusal path; the call shape is [V] |
| B6 | `gj`/`gk` under an operator or with extra cursors uses a display-cell column as a byte offset. | `normal/bind.rs:51` feeds `crates/strop-grammar/src/resolve/motion.rs:153`. | [A], medium confidence |
| B7 | Snippet placeholder deletion ignores the apply result. | `editor/snippet.rs:190-210`: `let _ = self.apply(…)`, then stops are remapped unconditionally. | [A] |

Details:
- **B1.** Buffer `"é\nabc"`: `yy` on line 0, `j`, `p`. `text = "é\n"`, `prefix = 1`, so the slice lands mid-character and panics. With `"abc"` and register `"  foo\n"`, the cursor lands one byte short of `f`.
- **B2.** `line_of` and `line_start` split at a lone CR, VT, FF, NEL, U+2028 or U+2029; `line_end`, `last_content_line` and the tree-sitter point math do not. The grammar test `lone_cr_is_a_plain_char` (`crates/strop-grammar/src/tests.rs:614`) shows CR is meant to be a plain character. The fix is configuration, not a proof: `ropey = { version = "1", default-features = false, features = ["simd"] }` in all nine `Cargo.toml` files, plus a test.
- **B3.** It is correct for a single edit and wrong for multi-edit batches (LSP format or rename, `:s`). Marks can be dropped or kept wrongly.
- **B4.** Replacing the first bytes of the line just below an excerpt pulls a partial line into it, breaking the whole-lines invariant.
- **B5.** In a collection view with one cursor on chrome, the text and cursors diverge.

### 2.1 The kernel's "single mapper" claim is false

Five hand-rolled mappers sit next to `editmap` [A table; the sites marked V I read]:

| Site | Rule | Differs from `map_position` |
|---|---|---|
| `editor/transact.rs:237` `map_excerpt_span` [V] | span; start is left-affine, end grows | by design, plus bug B4 |
| `editor/collections/updates.rs:19` `map_view_changes` | span; a straddling edit is left unmapped | by design; relies on admission |
| `editor/completion/words/journal.rs:193` `map_edge` | span edge (Start/End) | by design |
| `editor/structural_indent.rs:403-432` | `isize` running delta; interior maps to the end of the new text | yes: (5,4,8,6) gives 6, kernel gives 4 |
| `editor/cursor.rs:142` `remap_after_mirrored_edit` [V] | `old + delta·(below + own)`, `.max(0)` | arithmetically equal to the kernel fold; refusal-blind (B5) |
| `editor/ex/global.rs:37` `MarkedLines::map_edits` | line anchors | by design |

Spans legitimately need a different affinity from points. But the span rule is where the bugs are, and it exists in three unproved copies.

### 2.2 Ranked candidates

Ranking is (bug-risk × user impact) / proof cost. All are kernel-shaped: `usize` in and out, explicit loops, no dependencies.

**#1 — `editmap::map_span`: one span rule, proved. Cost M.**
- Move: `map_excerpt_span` (`transact.rs:237`), the span half of `map_view_changes` (`updates.rs:19`) and `map_edge` (`words/journal.rs:193`) into one function:
  ```rust
  pub fn map_span(start: usize, end: usize, s: usize, e: usize, new_end: usize,
                  grow_at_end: bool) -> (usize, usize)
  ```
  The caller decides `grow_at_end`. For excerpts it is "the inserted text ends in a newline and the edit is an insertion at `end`", which fixes B4.
- Also delete `remap_after_mirrored_edit` and route it through `map_through_edits` over the journal of inserts that actually applied (fixes B5; no new kernel code).
- Theorems, in words:
  - A span stays a span (`start' ≤ end'`) and stays inside the edited buffer.
  - Two ordered disjoint spans stay ordered and disjoint through any edit that lies inside one of them or between them. So excerpts can never come to overlap.
  - An edit strictly inside the span changes its length by exactly the edit's delta.
  - An edit strictly outside changes its length by 0.
  - With `grow_at_end = false`, an edit starting at `end` never changes `end`.
  - Interior points agree with `map_position`.
- Lean sketch:
  ```lean
  theorem map_span_spec (a b s e n : Usize) (g : Bool) (hab : a.val ≤ b.val) (hse : s.val ≤ e.val)
      (hsn : s.val ≤ n.val) (hfit : n.val + (b.val - e.val) ≤ Usize.max) :
      editmap.map_span a b s e n g ⦃ r => r.1.val ≤ r.2.val ∧
        (e.val ≤ a.val → a.val ≠ s.val → r.2.val - r.1.val = b.val - a.val) ∧
        (a.val ≤ s.val → e.val < b.val → (r.2.val : Int) - r.1.val = (b.val - a.val) + (n.val - e.val)) ∧
        (g = false → s.val = b.val → r.2.val = b.val) ⦄
  theorem map_span_keeps_order … (h : b₁ ≤ a₂) (hin : (a₁ ≤ s ∧ e ≤ b₁) ∨ (b₁ ≤ s ∧ e ≤ a₂) ∨ (a₂ ≤ s ∧ e ≤ b₂)) :
      (map_span a₁ b₁ …).2 ≤ (map_span a₂ b₂ …).1
  ```
- Mutants:
  - Treat `end == s` as covered (today's behaviour); must fail the `g = false` clause.
  - Use right affinity for the span start; must fail the interior-length clause.
- Oracle: the existing `collections/tests/convergence.rs` property test, plus a new case "replace the first bytes of the line below an excerpt".

**#2 — `editmap::keep_disjoint`: the multicursor plan filter, proved to feed `validate_batch`. Cost S.**
- Move: the overlap filter in `ActionPlan::from_resolved` (`crates/strop-grammar/src/resolve/mod.rs:442` [A]): sort by start, dedup, keep a range only if `start >= prev.end`.
- Kernel shape: `keep_disjoint(ranges: &[(usize, usize)], keep: &mut [bool])` over input already sorted by `(start, end)`. Use the same `debug_assert` that `mutation.rs:188` already uses.
- Theorem, in words: the kept ranges are a subsequence of the input, and the kept non-empty ranges satisfy the existing `BatchOk` predicate. So a plan built from the filter is never refused with `Overlap` or `Unsorted` by `validate_batch`. This composes two kernel claims end to end.
- Second theorem: the first range is always kept.
- Lean sketch:
  ```lean
  theorem keep_disjoint_spec (rs : Slice (Usize × Usize)) (keep : Slice Bool)
      (hin : ∀ r ∈ rs.val, InBounds len r) (hsorted : Sorted rs.val) (hlen : keep.length = rs.length) :
      editmap.keep_disjoint rs keep ⦃ k => BatchOk len (kept rs.val k.val) ∧
        (0 < rs.length → k.val[0]! = true) ⦄
  ```
- Mutants:
  - `start > prev.end`: touching ranges are wrongly dropped. Needs a maximality clause to catch it: "a dropped range overlaps the previously kept one".
  - Compare against the previous input range instead of the previous kept range; must fail `BatchOk`.
- User impact: every multicursor operator.

**#3 — `operated`'s decision table (0086 F3 / 0088 K2, now unblocked). Cost M.**
- 0086 §9.1 deferred K2 because the span decision was spread over the resolver arms. That is no longer true. `operated(buf, span, op, force)` at `crates/strop-grammar/src/resolve/mod.rs:277` (~85 lines) is the single normaliser every target feeds [A].
- Move the arithmetic into a kernel function over precomputed facts:
  ```rust
  pub fn operated_span(start, end, kind: u8 /*Excl|Incl|Line*/, force: u8, on_lines: bool,
      is_op: bool, start_in_indent: bool, end_line_start, end_line_end, prev_line_start,
      prev_line_last /*clamp(line_end-1).max(line_start)*/, ceil_after_end, after_end_line /*next line start or len*/,
      start_line_start) -> (usize, usize, u8, bool)
  ```
- The buffer-reading stays in the grammar; the decisions move.
- Theorems, in words:
  - The result is ordered and never passes `after_end_line`.
  - Linewise gives exactly `(start_line_start, after_end_line)`.
  - Inclusive gives `end' = ceil_after_end` unless `end` is already at the line end.
  - The exclusive-at-column-0 rule fires exactly when `is_op ∧ Excl ∧ multiline ∧ end = end_line_start`, and becomes Linewise exactly when `start_in_indent`.
  - Forcing `v` swaps Exclusive and Inclusive, and applying it twice is the identity.
  - A charwise result always contains `[start, end)`.
- Mutants:
  - Drop the `end < line_end` guard on the inclusive bump.
  - Bump by 1 instead of `ceil_after_end` (`dl` on a multibyte char).
  - Drop `start_in_indent`.
- Oracle: the differential corpus already has ≥ 600 cases (0095). Add `d$ dj d} dG v$d Vjd` and `dl` on `é` if they are missing, and assert `preview == execute` on them. That lands doctrine item 2 as a theorem-backed property.
- Ranked third, not first, because the Neovim oracle already covers this function heavily. The proof's marginal catch rate is lower than #1's.
- Same PR: move the satellites that do their own inclusive/exclusive conversion onto the kernel function:
  - the case-operator patch at `mod.rs:236-250`
  - `select_object` at `mod.rs:368`
  - `visual_range` at `strop-engine/.../visual/mod.rs:326`
  - surround's `end - 1` / `start + 1` at `normal/execute.rs:47-58`

**#4 — Viewport clamps (0087 K5). Cost S.**
- Move, from `editor/view.rs` and neighbours [A]:
  - `scroll_to_cursor` (`cursor.rs:219`)
  - `view_place` (`view.rs:61`, zz/zt/zb)
  - `scroll_lines` (`view.rs:25`)
  - `reveal_column` (`panes/mod.rs:45`)
  - the `max_top` clamp in `prepare.rs:250`
- All are functions of `(top, line, rows, last)`.
- Theorems, in words:
  - After `reveal`, the cursor row is visible (`top' ≤ line < top' + rows` when `rows ≥ 1`).
  - `reveal` is idempotent and moves `top` only when it must.
  - `place` results are `≤ max_top`.
  - After `scroll_lines`, the cursor is inside the window.
- Lean sketch:
  ```lean
  theorem reveal_spec (top line rows : Usize) (h : 1 ≤ rows.val) :
      view.reveal top line rows ⦃ t => t.val ≤ line.val ∧ line.val < t.val + rows.val ∧
        (top.val ≤ line.val ∧ line.val < top.val + rows.val → t = top) ⦄
  ```
- Mutant: `line > top + rows` instead of `>=`; must fail visibility.
- Low bug-risk today, since there is no wrap and no scrolloff. This is the cheapest way to make the scrolloff or soft-wrap work safe when it comes.

**#5 — Mark fate through a journal. Cost S–M.**
- Move: the mark-deletion rule (`transact.rs:150-164`) into a fold beside `map_through_edits`:
  ```rust
  pub fn map_mark_through_edits(position: usize, edits: &[(usize, usize, usize)],
                                starts_line: &[bool]) -> Option<usize>
  ```
- The rule is checked per step, in the coordinates that step addressed. That fixes B3.
- Theorems, in words:
  - `Some(p')` implies `p' = map_through_edits p edits`.
  - `None` holds exactly when some step, seen from the mark's position mapped through the earlier steps, is a pure deletion starting at a line start that covers it.
- Mutant: compare each step against the original position (today's code); must fail the `None` characterisation on a two-edit journal.
- The engine must compute `starts_line` at apply time (journal it as a fourth field), not after the batch.

### 2.3 Not worth moving

- **Undo/redo inversion**: an allocation-heavy tree, already property-tested by `transaction_conformance`.
- **Grapheme and width math**: needs Unicode tables.
- **Word and bracket scanners** (0087 K4): the rope is not a slice, and the Neovim oracle is the right check.
- **Count arithmetic**: saturating multiplies with a parser that never yields 0. Add a `debug_assert!(count >= 1)` at the four unchecked `count - 1` sites [A]: `resolve/mod.rs:102`, `motion.rs:163,247,260`.
- **The four `*guard` modules**: inline them.
- **Picker `move_by`** (`as i32` casts): fix in place.

---

## 3. TLA+

### 3.1 Verdict: delete both models

| Model | Lines | Recorded catch | Tie to the Rust | Verdict |
|---|---|---|---|---|
| `specs/EditorProtocol.tla` + `_Mutant` | 246 + 157 | none | comments in `editor/transaction_conformance.rs` and `…/traces.rs` | **delete** |
| `specs/SearchLifecycle.tla` + `_Mutant` | 352 + 312 | none | comments in `editor/picker/lifecycle_traces.rs` and `crates/strop-picker/src/source/lifecycle_traces.rs` | **delete** |

- **No catch.** 0085 H3 searched for a recorded catch and found none. Nothing in the plans or the changelog since then credits either model.
- **No change since probation began.** `git log -- specs` ends at `6aeded7` (2026-11-10), which only deleted WorkerSession. Probation ended at 0.55 (tagged 2026-11-10); the product is at 0.70.
- **The tie to Rust is naming only.** If the Rust changes, the model stays green.
- **The mutant files are whole-file copies** that must be hand-synced.
- **The lane does block.** `ci.yml:31-36,43-52`, the Dockerfile `model` stage (`RUN sh specs/gate.sh`) and `release-local.sh:59` all run it. It blocks on something that cannot fail for a product reason.
- **Survival rule (AGENTS.md):** a counterexample finder stays only "with a recorded catch". The Rust side already holds the real guards. `transaction_conformance*` (a seeded property test with shrinking on the real `Editor`) and the trace tests stay; strip their TLA references.

Deletion list:
- `specs/`
- the Dockerfile `model` stage
- the compose `model` service
- the `ci.yml` `model` job and its `needs` entry
- `release-local.sh:59`
- the AGENTS.md sentence about probation and the "model" mentions in Release gates
- the 0086 §8 row

### 3.2 What deserves a concurrency check instead

Existing Loom coverage [V]: 5 tests.
- `editor/events/channel.rs:481,536,558`: the event lane (never loses a semantic event, close wakes a sender, pop wakes a sender).
- `editor/picker/source_sink_tests.rs:9`.
- `crates/strop-lsp/src/client/queue/loom_tests.rs:69`.

Candidates, ranked [A for the inventory]:
1. **`crates/strop-core/src/worker.rs` cancel/finish exactly-once.** `CancelToken` is an `AtomicBool` plus `Mutex<Option<Resource>>`; the emitter is `Arc<Mutex<Option<FnOnce>>>`. Every job in the editor rests on it: rg, git, analysis, fs. It has no Loom test. Property: for any interleaving of cancel and finish, exactly one outcome is emitted and the resource is released exactly once. Cost S.
2. **Message-order races** (picker generation guard, analysis results vs edits, LSP workspace symbols). These are single-threaded ordering problems, so Loom does not apply. Use the pattern 0099 §5.1 already established: `editor/symbols/race_tests.rs` enumerates all 262,144 six-step sequences against the real `SymbolStore`. Do the same for `handle_picker_event` over {new query, partial from gen g, finish from gen g, close, reopen} with the invariant "rows shown belong to the current generation". That replaces SearchLifecycle with a check on shipped code. Cost S–M.
3. **`crates/strop-terminal/src/service/mod.rs` permit count and `try_send`** (`:59,124`): a small Loom test that the permit count never leaks on `Full` or disconnect. Cost S.
4. **`crates/strop-remote/src/pool.rs` actor** (`sync_channel(32)`, `StopSignal`, `StatusCell`): covered by the real-sshd journeys; no model needed.
5. **`strop-fs` notify**: no threads; nothing to do.

---

## 4. Gate integrity

| # | Finding | Evidence | Severity |
|---|---|---|---|
| G1 | The `proofs` lane blocks only by convention. | It runs only in `release-local.sh qualify` (`:58-60`). `package`, `draft` and `publish` (`:62-135`) check a clean tree, the tag and the checksum; none requires that `qualify` passed on this commit. `ci.yml` has no proofs job. | **high** |
| G2 | Kernel drift is invisible until release. | A PR that changes `crates/strop-kernel` without refreshing `proofs/Extracted`, or that breaks a proof, merges green; `ci.yml`'s `gate` needs only test, model and ssh-pythonfree. Raised as 0087 P3, deferred in 0088 §4, still open. | medium |
| G3 | The Loom lane can pass with zero tests. | Dockerfile `test` stage runs `cargo test … -p strop-engine -p strop-lsp loom`, a name filter. If the five `loom_*` tests are renamed or cfg'd out, cargo exits 0. | medium |
| G4 | Mutant calibration never exercises 20 of 34 claims (S1). The claims list is hand-maintained (S3). | `proofs/mutants.py:36-174`, `proofs/Audit.lean:21-56` | medium |
| G5 | The `model` lane blocks on nothing that can fail (section 3). | `specs/gate.sh` | low; delete |
| G6 | AGENTS.md is stale. | "on probation until 0.55" at 0.70; plan 0086 §8 still lists `container-test`. | low |
| G7 | `search-gate.sh:9` mktemp template has a stray `}` (`XXXXXX}`). | GNU mktemp accepts it; moot if `specs/` is deleted. | cosmetic |
| G8 | Non-hermetic test: uses `std::env::temp_dir()` with a fixed name. | `crates/strop-picker/src/source/lifecycle_traces.rs:20` | low |

Notes:
- **G1.** The only structural link is the `gh run list` check, which is inside `qualify`. A release can be published with `qualify` never run. The no-receipts rule forbids a marker file, so the fix is structural: make `package` call `qualify` first, or fold the two into one command.
- **G2.** A cheap hosted lane running Charon and Aeneas only (no mathlib) would catch translation drift. If the image cannot fit, accept the risk explicitly.
- **G3.** Fix by asserting the count, for example `cargo test … loom -- --list | grep -c ': test$'` equals 5, or by using a dedicated test target.

Checked and sound [V]:
- `test`, `model` and `ssh-pythonfree` run as `RUN` steps at image build, so a failure fails `docker compose run --build`.
- `STROP_REQUIRE_NVIM=1` and `STROP_REQUIRE_SSH_TESTS=1` turn skips into failures in the gate (`crates/strop/tests/differential/main.rs:301-306`).
- The CI `gate` job uses `if: always()` and tests each `needs.*.result`.
- The mutant runner refuses a mutation site that does not match exactly once (`mutants.py:246`), refuses errors outside the theorem files, and refuses a surviving build.
- The TLC gates reject parse errors as non-kills.

---

## 5. Ranked actions

| # | What | Why | Cost |
|---|---|---|---|
| 1 | Fix B1 (`registers/put.rs:60`: compute `blank` over `text`, not `text[prefix..]`) with tests for multibyte and indented first rows | panic on a common key sequence | S |
| 2 | Fix B2: ropey with `default-features = false, features = ["simd"]` in all nine `Cargo.toml`s, plus tests for lone CR and U+2028 | line index disagrees with all of strop's line math | S |
| 3 | Kernel `map_span` + route the three span mappers through it + replace `remap_after_mirrored_edit` with `map_through_edits` over applied inserts (fixes B4, B5) | the only place the kernel's stated contract is false, and where the bugs are | M |
| 4 | Delete `specs/`, the `model` lane and its references | survival rule; 15 releases past probation; no catch | S |
| 5 | Make `proofs` structurally blocking: `package` runs `qualify` (or merge them); decide on a Charon-only drift lane in CI | G1, G2 | S |
| 6 | Paired mutants for property theorems (one per family: tile in-bounds/exact/fair, editmap monotone/composition, cache find-after-retire/evicts-oldest, excerpt in-span) + `Audit.lean` requires every extracted function to appear in a claim | S1, S3: calibration currently covers 14 of 34 claims | M |
| 7 | Kernel `map_mark_through_edits` (fixes B3) | wrong marks after multi-edit batches | S–M |
| 8 | Kernel `keep_disjoint`, proved to yield `BatchOk` | composes with existing claims; every multicursor operator | S |
| 9 | Kernel `operated_span` + differential cases + `preview == execute` assertion (0086 F3) | the product's central decision; K2's blocker is gone | M |
| 10 | Assert `TileFits` premises at `panes/layout.rs:234`; assert the Loom test count | S4, G3 | S |
| 11 | Loom test for `worker.rs` cancel/finish exactly-once; exhaustive sequence test for the picker generation guard | replaces SearchLifecycle with checks on shipped code | S–M |
| 12 | Verify and fix B6 (`gj`/`gk` column unit) and B7 (snippet apply result) | sub-agent findings, not re-read | S each |
| 13 | Kernel viewport clamps | cheap; pays off when scrolloff or wrap lands | S |
| 14 | Cleanup: delete `Selections::remap`; inline the four `*guard` modules; derive `insert_capacity` from the kernel; remove the tautological `debug_assert` at `mutation.rs:188`; correct the overclaiming doc comments (S5); refresh the AGENTS.md verification and gates text | weightless code and stale claims | S |

## 6. Taste questions for the owner

1. **Restatement models.** `mapSpec`, `journalSpec` and `tileFrom` mirror the code line for line. Is that acceptable as long as properties sit on top and are calibrated (action 6), or should headline properties be stated directly over the extracted function, with harder proofs and no model to drift?
2. **Span affinity.** Should typing at column 0 of the line directly below an excerpt ever extend the excerpt? Today it does; that is B4. The proposed rule is: only when the insertion ends in a newline. This is a product decision the theorem will freeze.
3. **Line breaks.** Is "only LF and CRLF are line breaks" the intended contract for VT, FF, NEL and U+2028/2029 as well as lone CR? Vim treats all of them as plain characters, which suggests yes.
4. **Proofs in CI.** Is a release-time-only proof gate acceptable given roughly ten releases a week, or is a Charon-only drift lane on hosted CI worth its image cost?
5. **TLA+.** Delete `specs/` outright, or does the owner want one last attempt to make TLC earn a catch? The survival rule as written says delete.
6. **Claims list.** Hand-written, as today, or generated as "every theorem whose statement mentions an extracted function"? Generating removes the option of leaving a helper lemma unclaimed.
7. **Kernel growth.** #3 `operated_span` takes about 14 scalar facts as arguments. Is a wide-signature decision table an acceptable kernel shape, or should the kernel grow a byte-slice line model first?

## §B. Audit: runtime hardening


Scope: `/home/tarek/workspace/strop` at `ca6caa2` (0.69.0 tree), read-only. Static review of
the crates plus black-box fuzzing of the installed release binary (`~/.local/bin/strop`,
0.70.0) through `strop headless`, in a sandbox directory with a private `HOME`.
Nothing in the repo was edited or built.

Every "confirmed" finding below was reproduced on the 0.70.0 binary; the line numbers are
from the repo tree. "Static" findings were read, not executed.

## Verdict

The outer rings are in good shape. The save path (`strop-fs/src/local/store.rs`) is staged,
fsynced, digest-checked and renamed atomically; LSP edit coordinates are checked
(`strop-lsp/src/positions.rs`); the embedded terminal denies clipboard writes
(`strop-terminal/native/bridge.c:78`); ratatui 0.29 drops control characters before they reach
the tty; every `unsafe` block carries a comment; non-test `unwrap`/`expect` are few and
guarded; the main thread never calls `send_blocking`; recovery checkpoints run after every
event on a worker.

The defects are in the middle: the editor grammar on non-ASCII text, and operations whose
cost grows with the square of the buffer. About 25,000 random key scripts found two
distinct panics in under ten minutes. Both escape the differential harness because its
fixtures are ASCII-only, LF-only and always end in a newline
(`crates/strop/tests/differential/generated.rs:10-31`).

---

## Findings

### H1 — crash: `p` of a linewise register below a last line with no newline (confirmed)

- **Evidence:** `crates/strop-engine/src/editor/registers/put.rs:60`
  `let row = &text[prefix.min(text.len())..];`
  `prefix` is the length of the line break that will be *prepended* (1 for LF, 2 for CRLF),
  but it is used to slice the register text, which has not been prefixed yet.
- **Scenario:** a buffer whose last line has no trailing newline (every new unsaved buffer,
  many real files). `yyp` on a line that starts with a non-ASCII character panics:
  `buffer "étude"` + `keys yyp` → `start byte index 1 is not a char boundary`.
  Also a bracketed paste of whole lines in Normal mode (`buffer ""` + `paste "é日\r\n"`), and
  in a CRLF file any line whose second byte is inside a character (`"aé\r\nb"`, `yyGp`).
- **Also wrong for ASCII:** `buffer "a b"`, `yyp` lands the cursor on column 1, not on the
  first non-blank (column 0) — the blank count is taken from the shifted slice. Vim lands on
  column 0. So this is a fidelity bug as well as a crash.
- **Fix:** count leading blanks on `text` itself; `land = at + prefix + blank`.
- **Test:** state assertions for `yyp`, `yyP`, `]p` and Normal-mode paste on
  {LF, CRLF} × {final newline, none} × {ASCII, `é`, `日`, `🎉`} first characters; add the
  no-final-newline case to the differential corpus.

### H2 — crash: `Ctrl-W` / `Ctrl-U` in Insert after moving left and typing a non-ASCII character (confirmed)

- **Evidence:** `crates/strop-engine/src/editor/insert/keys.rs:61-63`
  (`floor = floor.max(origin)`), origin recorded once at `insert.rs:157-158` as a raw byte
  offset and never moved when text is inserted before it. The floor can land inside a
  character; `keys.rs:120` then slices the rope there
  (`byte_slice(): Byte range does not align with char boundaries`).
- **Scenario:** `A` (or `a`), `<Left>`, type `é`, `Ctrl-W`. Minimal script:
  `buffer "one two\nfour\n"`, `keys a<left>é<c-w>`. `Ctrl-U` panics the same way.
- **Also wrong for ASCII:** after `<Left>` the stale origin still acts as the "start of this
  insert" stop, at the wrong place.
- **Fix:** drop `insert_keys.origin` whenever the cursor moves in Insert other than by typing
  (Vim starts a new insert there), or keep it as an anchor that edits shift; clamp the floor
  with `ceil_boundary` as a belt.
- **Test:** the script above for `Ctrl-W` and `Ctrl-U` with 2-, 3- and 4-byte characters;
  a property test (see T1) would have found it.

After either crash the panic hook restores the terminal and the last completed recovery
checkpoint survives, so the loss is bounded to the edits since that checkpoint. Severity:
crash, not data loss.

### H3 — wrong behaviour: new files are created with mode 0600 (confirmed)

- **Evidence:** `crates/strop-fs/src/stage.rs:63` creates the stage file `0o600`.
  `store.rs` restores owner, mode and xattrs only when a file already exists
  (`if let (Some(metadata), Some(attributes)) = (&before, &attributes)`); for a new file the
  stage's mode is published as is. The crate's own stated policy is
  "create: 0666/0777 subject to umask" (`crates/strop-fs/src/context.rs:114`).
- **Scenario:** `umask 022`; `strop new.txt`, type, `:w` → `-rw-------`. Same for
  `:w name`. Every file strop creates is unreadable by group and others: scripts served by
  another user, shared directories, container build contexts. Git does not record it, so it
  goes unnoticed until something else cannot read the file.
- **Fix:** when the destination is absent, `fchmod` the stage to `0o666 & !umask` before
  publication (read the umask once at startup; `umask()` is process-global, do not toggle it
  from a worker).
- **Test:** store a new file under umask 022 and 077; assert 0644 and 0600.

### H4 — wrong behaviour: `:g` is quadratic and cannot be interrupted (confirmed)

- **Evidence:** `crates/strop-engine/src/editor/ex/global.rs:36-88`.
  `MarkedLines::map_edits` walks every remaining marked line for every edit (two
  `retain_mut` passes, the second with a `line_of` rope lookup per mark).
- **Measured (0.70.0, 4-word lines):** `:g/./norm Ax` — 5,000 lines 4.5 s, 10,000 lines
  19.9 s, 20,000 lines over 45 s. `:g/one/d` and `:g/one/s/o/0/` on 100,000 lines: over 45 s
  each (killed). `:%norm Ax` on 20,000 lines: 0.23 s, so the cost is the mark remapping.
- **Scenario:** `:g/DEBUG/d` on a 100,000-line log. The editor is frozen for minutes on the
  input thread; no key, including `Ctrl-C`, is read until it finishes. The user's only way out
  is `kill`, which loses everything since the last checkpoint.
- **Fix:** `:g` visits marks in order, so an edit confined to the current line only shifts the
  marks after it: keep one running delta for "everything ahead" and remap individually only
  when an edit reaches past the current line. Separately, poll the input lane for `Ctrl-C`
  between marked lines.
- **Test:** a bench or a bounded-work test: `:g/x/d` on 100,000 lines completes within a
  fixed step budget; a 2× larger input costs at most ~2× the steps.

### H5 — wrong behaviour: no interrupt for long input-thread work; macros run at ~1,600 keys/s on files over 4 KiB (confirmed)

- **Evidence:** counts are capped at 99,999 (`editor/input.rs:152`), which bounds memory but
  not time. `resolution_is_large` (`editor/resolution/mod.rs:249-254`) and
  `structural_indent.rs:177` send every resolution to a worker once the buffer or cursor
  offset passes 4,096 bytes; replayed keys then wait one worker round trip each.
- **Measured:** `qaxjq20000@a` on a 20,000-line buffer: 24.9 s. `qaox<esc>q99999@a`: not
  finished after 30 s. `5000ox<esc>` in a 2-line buffer: over 30 s (fine up to 2,046 lines,
  then each `Enter` crosses the 4 KiB line). `99999J`: 3.6–6.7 s. `99999>>`: 3 s.
  `*` or `n` on a 600 KB single line: ~2 s each.
- **Scenario:** a recorded macro applied to a large file with a big count, as Vim users do
  (`1000@q`). Input is queued behind it with no way to cancel.
- **Fix:** inside a macro or count replay keep resolution inline (as `compound_begin`
  already does for `:g`/`:normal`), and check for `Ctrl-C` between repetitions.
- **Test:** `20000@a` of `xj` completes inside a step budget; `Ctrl-C` queued behind a
  running macro stops it.

### H6 — wrong behaviour: the startup open bypasses the guarded reader (confirmed)

- **Evidence:** `crates/strop/src/main.rs:222` calls `Buffer::open`
  (`crates/strop-core/src/buffer.rs:200-214`): `File::open` + `Rope::from_reader` with no
  file-type check and no size bound. The in-editor path (`editor/io/open.rs`,
  `editor/namespace/local.rs:97`) refuses anything but regular files and directories.
- **Scenario:** `strop /dev/zero` grows by ~1.2 GB/s until the OOM killer (measured 3.2 GB
  after 3 s); `strop some.fifo` blocks forever before the screen is drawn. `:e /dev/zero`
  inside the editor is refused correctly. A Latin-1 file given on the command line exits with
  `strop: stream contained invalid UTF-8` instead of opening the editor with a message.
- **Fix:** start with an empty buffer and route the initial file through the same open job
  as `:e` (the remote operand already does this via `startup_open`), then delete
  `Buffer::open`'s use from `main`.
- **Test:** CLI test: `strop <fifo>` and `strop /dev/zero` reach the editor with a typed
  refusal within a deadline.

### H7 — async invariant: filesystem syscalls on the event loop per watcher hint (static)

- **Evidence:** `crates/strop-engine/src/editor/notify/reconcile.rs:152` —
  `std::fs::canonicalize(&absolute)` for every hint, inside `apply_notify_hints`, which runs
  on the event loop. It then scans every open document per hint (lines 153-185). One drain
  can hold 64 records × 512 hints (`notify/queue.rs:14-16`) and is one event, so
  `TURN_BUDGET` cannot split it.
- **Scenario:** `git checkout` or a build writes thousands of files under the project root
  while the user types. Up to ~32,000 `realpath` calls run between two keystrokes; on a slow
  filesystem (WSL `/mnt/c`, NFS, sshfs) that is a visible stall.
- **Fix:** canonicalise the root once at subscribe time (the watcher thread already does,
  `subscribe.rs:170`) and compare hints against a map of the open documents' stored
  identities; no syscall in the handler.
- **Test:** a handler test with a filesystem stub that fails on any call from the event
  thread; 32,000 hints against 50 open documents within a step budget.
- Smaller instance: `editor/io/save.rs:338` canonicalises on the event loop when a save
  completes (one call; same fix — carry the path in the worker's receipt).

### H8 — wrong behaviour: files that are hard-linked, or in a directory you cannot write, can never be saved (confirmed)

- **Evidence:** `crates/strop-fs/src/local/store.rs:90-95` and `prepare.rs:101,176` refuse
  `links != 1`; the stage directory is created inside the parent, so a read-only parent
  fails with `Permission denied`. `:w!` gets the same refusal. `attributes.rs:131-142` also
  refuses when owner or group cannot be preserved (a group-writable file owned by someone
  else).
- **Scenario:** files in a pnpm or Nix store, `cp -l` snapshots, a single file bind-mounted
  into a container, `/etc` files writable through a group. The refusal is safe — nothing is
  lost — but the user has typed edits and no command writes them to that path.
- **Fix:** for these cases write in place (open `O_WRONLY`, write, `ftruncate`, `fsync`)
  after first writing the recovery checkpoint, as Vim's `backupcopy=auto` does; keep the
  rename path as the default. Needs a plan amendment (0040/0054 record the refusal as
  deliberate).
- **Test:** store to a hard-linked file keeps both names on the new content; store into a
  0555 directory succeeds for a writable file.

### H9 — hostile repository: `git` runs with the repository's own config (static)

- **Evidence:** `crates/strop-git/src/exec.rs:147-149` — `git -C <workdir> <argv>` with no
  config overrides; callers include `blame` (`memory.rs:34,102`), `show` (`memory.rs:245-`),
  `log` (`log.rs:156`), `commit-graph write` (`graph.rs:44`). Only the commit-graph helper
  neutralises global config (`memory.rs:375-376`).
- **Scenario:** a project unpacked from an archive carries `.git/config` with
  `core.fsmonitor=<command>`, a `textconv`/`diff.external` driver, or `core.sshCommand`.
  Opening blame, a commit or the log in strop runs that command as the user. LSP commands
  from a project already need `:trust`; git does not. (libgit2 paths are not affected.)
- **Fix:** for read-only queries pass `-c core.fsmonitor=false -c core.hooksPath=/dev/null`,
  `--no-ext-diff --no-textconv` where diffs are produced, and set
  `GIT_OPTIONAL_LOCKS=0`; or gate git CLI use behind the same `:trust` answer.
- **Test:** fixture repo whose config sets `core.fsmonitor` to a script that touches a
  marker file; open blame/log/commit; assert the marker is absent.

### H10 — hygiene: SIGTERM and SIGHUP leave the terminal raw (static)

- **Evidence:** only `SIGWINCH` is registered (`crates/strop/src/terminal/reader.rs:25`).
  `restore_terminal` runs from `Drop` and the panic hook only (`terminal.rs:22-49`).
- **Scenario:** `kill <pid>`, a session manager, or a shutdown: the shell is left in raw mode
  on the alternate screen with the kitty keyboard flags still pushed. Drafts are safe up to
  the last checkpoint, but the final checkpoint (`recovery_on_finish`) does not run.
- **Fix:** add SIGTERM/SIGHUP to the existing `signal_hook` thread and send
  `AppEvent::QuitIntent`, which already takes the orderly path.
- **Test:** PTY test: send SIGTERM, assert the restore sequence is written and the recovery
  file holds the last edit.

### Minor / noted

- `Buffer::open` and the worker open mark a file writable when any write bit is set
  (`buffer.rs:209`, `io/open.rs:92-94`), not when *this user* may write. A root-owned 0644
  file opens editable and fails only at `:w`. Use `access(W_OK)` (`faccessat`).
- Save conflict detection is the mtime alone when no content baseline is carried
  (`io/save.rs:411-418`: `baseline_object: None, displayed: None`). A same-timestamp
  external write on a coarse-mtime filesystem (FAT, some NFS) is not seen. Carrying the
  length, or the digest the open already could compute, closes it.
- `strop-grammar/src/resolve/objects.rs:239-294` scans byte by byte through
  `Buffer::byte` (a rope descent per byte) to the start of the buffer for an unmatched
  bracket: `ci(` with no opener in a 100 MB file walks all of it on the resolver. Use a
  chunk iterator.
- `:`-line: `Esc` leaves the line open in its Normal mode (a second `Esc` closes it). This
  is the documented field design, not a defect, but it is how the fuzzer wrote two files in
  its sandbox (`:<esc>` … later `<cr>` ran `:w<text>`). Worth a line in `docs/vim-compat.md`.
- One fuzz script (`fz/fail-56132.keys`: splits, `:close` inside a macro, `@@`, `:e`) ends
  with `editor jobs did not settle` after 30 s. Not minimised; it may be H5 again or a job
  that never settles. Worth ten minutes with `--log`.

### Checked and found sound (no action)

- Atomic save, fsync of file and directory, digest check, symlink save (target is
  rewritten, link kept), mode and exec bit kept, CRLF and BOM and missing final newline
  round-trip byte-exact, external change refused until `:w!` — all confirmed on the binary.
- Swallowed errors: 51 candidates read; none hides a failed save, reload or git write.
- `unsafe`: 31 sites, all commented; FFI confined to `strop-terminal/src/vt`.
- Channels: bounded lanes with back-pressure from worker threads only; Loom models exist
  for the event channel and the LSP queue.
- Terminal output: no raw buffer, path or server text reaches the tty unsanitised; OSC 52
  payloads are base64; child-terminal clipboard writes are denied.
- LSP: frames bounded by a decode budget; edit positions validated for line, column and
  character boundary; navigation positions clamped.

---

## Test gaps and where a fuzzer pays fastest

Existing infrastructure: the Neovim differential harness (`crates/strop/tests/differential`,
ASCII fixtures only), a fixed-seed LCG property suite for the picker query language
(`strop-picker/src/query/properties.rs`), Loom models (`strop-engine`, `strop-lsp` under
`cfg(strop_loom)`), adversarial filesystem tests (`strop-fs/src/adversarial.rs`), the Lean
kernel. There is no `proptest`, `quickcheck`, `arbitrary` or `cargo-fuzz` in `Cargo.lock`, and
no random-key test of the editor itself.

- **T1 — random keys through the headless driver (found H1 and H2 in minutes).**
  A fixed-seed generator of grammar-shaped tokens (count, register, operator, motion, text
  object, Insert keys with cursor movement, Visual, a safe `:` list, paste, resize, frame)
  over fixtures that include multibyte, combining, CRLF, no final newline, empty, one very
  long line. Oracle 1: no panic. Oracle 2: after every step the cursor and every selection
  end are character boundaries inside the buffer. Oracle 3: `u` to the root restores the
  initial text and clears `dirty`; redo to the tip restores the final text. Shrink by
  deleting script lines. The scratch harness used here is
  `<audit scratch>/fz/fuzz.py`
  (generator) and `min.py` (line-deleting minimiser); in the repo it belongs in-process on
  `Editor` with the existing LCG, no new dependency.
- **T2 — widen the differential fixtures.** Add a multibyte fixture, a CRLF fixture and a
  no-final-newline fixture to `generated.rs`. H1's cursor error on plain ASCII (`yyp` on a
  last line with no newline) would then diff against nvim today.
- **T3 — work budgets.** For `:g`, `J`, `>>`, macro replay and search on one long line:
  assert steps (or wall time in the bench lane) grow at most linearly from n to 2n.
- **T4 — arbitrary bytes into the tty decoder** (`crates/strop/src/terminal/decode.rs`):
  random byte strings split at random read boundaries; oracle: no panic, and the
  concatenated decoded text equals the input for inputs without escape sequences.
- **T5 — arbitrary bytes into the embedded VT and git parsers** (`strop-terminal/src/vt`,
  `strop-git/src/{numstat,log,remote/parsers}.rs`): no panic, bounded output.
- **T6 — save matrix.** {new, existing} × {umask 022, 077} × {regular, symlink, hard link,
  read-only parent, foreign owner}: assert mode, link count, content.

---

## Ranked actions

| # | Action | Finding | Cost |
|---|---|---|---|
| 1 | Fix `put.rs:60` (blank count on unprefixed text) + tests | H1 | S |
| 2 | Reset or anchor the Insert origin on cursor movement; clamp floor | H2 | S |
| 3 | New files: apply `0666 & !umask` to the stage when the destination is absent | H3 | S |
| 4 | Land T1 in-process (LCG, boundary + undo oracles) and T2 fixtures | gaps | M |
| 5 | `:g` marks: running delta instead of full remap; `Ctrl-C` between lines | H4 | M |
| 6 | Route the startup file through the open job; drop `Buffer::open` from `main` | H6 | S |
| 7 | Inline resolution during macro/count replay; `Ctrl-C` between repetitions | H5 | M |
| 8 | Remove `canonicalize` from `apply_notify_hints`; identity map of open documents | H7 | S |
| 9 | Harden git CLI calls against repo config, or gate on `:trust` | H9 | S |
| 10 | SIGTERM/SIGHUP → `QuitIntent` | H10 | S |
| 11 | In-place write fallback for hard links, read-only parents, foreign owners (plan first) | H8 | L |
| 12 | `faccessat(W_OK)` for readonly detection; length/digest in the save baseline | minor | S |
| 13 | Minimise `fz/fail-56132.keys` (jobs did not settle) | noted | S |

## §C. Audit: clutter and bloat


Read-only audit. Nothing was built, edited or committed.

**Method and limits.** Evidence is from `rg`, `wc`, `cargo tree --offline --locked` and `Cargo.lock`.
Dead-code detection is a name-occurrence heuristic over all `crates/**/*.rs`: an item is "dead" when
its name occurs exactly once in the workspace (the definition), then re-checked by hand with `rg`
across `crates proofs .github docs tests`. It cannot see dead methods with common names (`len`,
`get`, ...), so the lists below are a floor, not a ceiling. No `cargo udeps`/`cargo bloat` run
(they would build). Binary size is from the newest local artifact, `dist/v0.62.0`, not 0.69.
Cost: S = under an hour, M = half a day, L = multi-day.

The tree is in better shape than most at this size: 205.9K lines of Rust, only 2 files past 800
lines, no openssl/native-tls in the lock, `git2` has `default-features = false`, golden files and
fixtures are all referenced, every script in `.github/scripts` and `tests/` has a caller.

---

## 1. Dead code

### 1.1 Pub items with zero references anywhere (confirmed, 17)

| Item | File |
|---|---|
| `Buffer::check_undo`, `Buffer::check_restore_revision` | `crates/strop-core/src/buffer/mutation.rs:448,454` |
| `History::last_committed_ops` | `crates/strop-core/src/history.rs:351` |
| `type ViewId`, `type PaneId` (plus their `ViewKind`/`PaneKind` markers), `checked_next` | `crates/strop-core/src/id.rs:51,52,346` |
| `diag_ranges_at` | `crates/strop-engine/src/editor/diagnostics.rs:96` |
| `action_labels` | `crates/strop-engine/src/editor/feedback/model.rs:241` |
| `indent_label` | `crates/strop-engine/src/editor/indent.rs:113` |
| `operator_register` | `crates/strop-engine/src/editor/input.rs:241` |
| `lsp_code_actions_pub` (a `_pub` forwarding wrapper with no caller) | `crates/strop-engine/src/editor/lsp.rs:343` |
| `terminal_launch_directory` | `crates/strop-engine/src/editor/terminal/mod.rs:179` |
| `keymap::completion::ARROWS` (a `Binding` in no table; see note) | `crates/strop-engine/src/keymap/completion.rs:28` |
| `revision_slug` | `crates/strop-git/src/revision.rs:54` |
| `RepoTarget::abs_of` | `crates/strop-git/src/target.rs:76` |
| `languages::specifications` | `crates/strop-syntax/src/languages.rs:469` |
| `strop_trace::take_failure` | `crates/strop-trace/src/lib.rs:315` |

Action: delete. Risk: low (clippy `-D warnings` will confirm). Cost: S.
Note on `ARROWS`: either the up/down completion binding is missing from `?` (a 0003 §5.7 gap) or
the constant is dead. Decide which; do not just delete.

### 1.2 Pub items reachable only from tests, not cfg-gated (compiled into the release binary)

`Editor::any_dirty` (`document/mod.rs:435`), `collection_row_kind` (`collections/projection.rs:222`),
`offscreen_count` (`ex/substitute/mod.rs:118`), `terminal_title` (`terminal/presentation.rs:15`),
`drain_terminals` (`terminal/lifecycle.rs:532`), `admission_probe` (`prepare.rs:164`),
`admit_document` (`document/mod.rs:283`), `History::can_undo`/`can_redo` (`strop-core/src/history.rs:317,321`),
`ThemeId::is_light` (`strop-core/src/theme.rs:243`), `Repo::head_content` (`strop-git/src/repo.rs:191`),
`require_full_stdout` (`strop-git/src/exec.rs:45`), `to_server_col` (`strop-lsp/src/protocol.rs`),
`is_unfiltered` (`strop-picker/src/query/plan/file.rs:157`), `StoredQuery::from_query`
(`strop-picker/src/query/store.rs:69`), `set_include_base`/`push_known_hosts`
(`strop-remote/src/hosts/mod.rs:182,189`), `notify::Manager::unsubscribe` (`strop-fs/src/notify/manager.rs:161`),
`keymap::completion::REQUEST`.

Action: per item, delete with its test (if the test only exercises the dead item) or gate under
`#[cfg(any(test, feature = "test-support"))]` like the `fixture_*` family already is.
Risk: low-medium (some may be intended API with a missing caller, e.g. `unsubscribe` suggests
watches are never released). Cost: S-M.

### 1.3 Dead platform branches

`crates/strop-fs/src/lib.rs:5` is `compile_error!` on anything but Linux, and `strop-remote`
depends on `rustix` unconditionally, so the product cannot compile off Linux. Yet ~30
`#[cfg(not(unix))]` / `#[cfg(windows)]` branches remain: `strop-core/src/process.rs` (3),
`strop-core/src/path_serde.rs` (5, including a `windows-utf16` wire variant),
`strop-remote/src/transport/error.rs` (3), `strop-remote/src/pool.rs:215`,
`strop-lsp/src/registry.rs:114,414`, `strop-workspace/src/addr/{uri,file,error}.rs` (4),
`strop-git/src/{numstat,permalink,remote/parsers}.rs` (3), `strop-fs/src/{context,observation}.rs` (3),
`strop-engine/src/editor/shell/process.rs:9`, `strop/src/cli/location.rs:119,144`, two test sites.
The `[target.'cfg(unix)'.dependencies]` tables in strop-core, strop-fs and strop-terminal are
equally meaningless.

Action: delete the non-unix arms and flatten the target tables. Risk: low. Cost: M.

### 1.4 Dead update channels

`crates/strop/src/update.rs:31-63`: `Channel::{Brew, Cargo, Mise}`. `install.sh:193` only ever
writes `"tarball"`, and AGENTS.md says crates.io and Homebrew are historical. Action: collapse to
tarball-or-unknown. Risk: low (old brew/cargo installs get "unknown", which is already handled). Cost: S.

### 1.5 Unused dependencies (zero `rg` hits in the crate)

- `sha2` dev-dependency of `strop-git` (`crates/strop-git/Cargo.toml:18`).
- `serde_json` dev-dependency of `strop-remote` (`crates/strop-remote/Cargo.toml:19`).

Action: delete. Risk: none. Cost: S.

### 1.6 Examples nobody runs

`crates/strop-lsp/examples/probe.rs`, `crates/strop-syntax/examples/probe.rs`,
`crates/strop-picker/examples/{score_probe,scorebench}.rs`. Only reference: `docs/nucleo-decision.md:8`
(a decision record from 2026-09-06 for a comparison whose loser, the custom scorer, no longer
exists). They are compiled by `clippy --all-targets` on every gate run. Action: delete all four and
`docs/nucleo-decision.md` (the CHANGELOG keeps the decision). Risk: low. Cost: S.

Not dead (checked): `editor/conformance.rs`, `contract_probes.rs` are `#[cfg(test)]` modules;
all 228 golden files and both `crates/strop/tests/fixtures/*.py` servers are referenced; config keys
are all consumed.

---

## 2. Shims and duplication

### 2.1 CLI flag/subcommand pairs (explicit old/new pair)

`crates/strop/src/cli.rs:187` literally says "subcommands (flags above remain the synonyms)".
Pairs: `--headless`/`headless`/`--script`, `--replay`/`replay`, `--remote`/`remote`,
`--bench`/`bench`, `--log`/`--log=ALL`, plus a no-op `edit` subcommand. `docs/cli.md:41-49`
documents them as "same as". This is what "Refactor aggressively" forbids.
Action: keep one spelling each (owner picks), delete the rest and the doc rows. Risk: breaks
scripts using the dropped spelling (`release-local.sh:91-94` and `docker-compose.yml:56` use
`--bench`; README uses `replay --script`). Cost: S.

### 2.2 Legacy deserialisation branches in an alpha with no compatibility promise

- `crates/strop-core/src/path_serde.rs:23-28,62-67`: `Wire::Legacy(String)` + `PathError::AmbiguousLegacy`.
- `crates/strop-terminal/src/model/wire.rs:57-58,258-266,359-362`: legacy per-cell row shape,
  legacy projected rows, `ProjectionWire`.
- `crates/strop-engine/src/editor/trace/seed.rs:270-276`: seeds without `semantic_version`.

Action: delete the legacy arms; old traces/sessions fail with a typed error. Risk: old session
files and recorded traces stop loading (owner's memory note says alpha: break things). Cost: S-M.

### 2.3 Picker preview resolved twice

`crates/strop-engine/src/editor/picker/preview.rs` (700 lines) holds `picker_preview_range`
(`&mut self`, line 82) and `RangeProbe::resolve` ("`&self` mirror ... for the paint path", line 169),
and again `PreviewProbe` ("`&self` mirror of the picker_preview resolution", line 268). Two
copies of the same branch ladder that must agree is the shape doctrine 2 exists to prevent.
Action: one `&self` resolver; the `&mut` caller does its cache write around it. Risk: medium
(paint path). Cost: M. This also brings the file back near 400 lines.

### 2.4 Two git back ends

`git2` is used in 3 files (`strop-git/src/repo.rs` 17 sites, `status.rs` 10, `graph.rs`/`exec.rs` 4);
everything else, and all remote work, shells out to `git` (`exec.rs`, `lib.rs`, `memory.rs`,
`remote/`), with a second status parser in `remote/parsers.rs`. libgit2 is a vendored C build that
costs compile time and binary size to serve hunks/status for local files only. AGENTS.md
requires libgit2 for per-keystroke paths, so this is a taste question (see end), not a defect.

### 2.5 Smaller

- Seven hand-rolled test PRNGs: `Rng` in `editor/conformance.rs:17`, `transaction_conformance.rs:62`
  ("same generator as conformance.rs"), `collections/tests/convergence.rs:11`,
  `strop-lsp/src/client/frame_limits/tests.rs:66`; `Lcg` in `panes/tests/tree.rs:188`,
  `strop-picker/src/query/properties.rs:11`; one more in `editor/cache.rs:162`. One 15-line
  helper in a test-support module. Cost: S.
- Warning suppressions against "no warning suppression": 6 `#[allow(clippy::too_many_arguments)]`
  (`strop/src/render/{picker/rows/search.rs:70, buffer/chrome.rs:24, buffer/rows/collection.rs:104,
  picker/preview.rs:142}`, `strop-engine/src/editor/{git_memory/jobs.rs:123, feedback/mod.rs:87}`)
  and 3 `#[allow(dead_code)]` in `strop-lsp/tests/decode_cost.rs:52,59,66`. Fix with a params
  struct each. Cost: M.
- Mixed module layout in `strop-engine/src/editor/`: both `x.rs + x/` (events, explain, git, input,
  insert, io, lsp, matching, namespace, normal, prepare, shell, structural_indent, transact,
  transaction_conformance) and `x/mod.rs` (actions, analysis, document, panes, ...). Pick one. Cost: S, churn only.

---

## 3. File size

### 3.1 Every `.rs` at or past ~750 lines

| Lines | File | State |
|---:|---|---|
| 995 | `crates/strop-engine/src/editor/picker/tests.rs` | over ceiling, not recorded in 0028 |
| 806 | `crates/strop-lsp/src/client/api.rs` | over ceiling, not recorded in 0028 |
| 790 | `crates/strop-engine/src/editor/resolution/mod.rs` | at ceiling |
| 788 | `crates/strop-picker/src/query/plan/tests.rs` | at ceiling (AGENTS.md cites it as the model split) |
| 785 | `crates/strop-remote/src/pool.rs` | at ceiling |
| 768 | `crates/strop-trace/src/tests.rs` | near |
| 753 | `crates/strop-engine/src/editor/analysis/presentation.rs` | near |

Next band (700-750): `analysis/mod.rs` 740, `tests/smartindent_tests.rs` 738, `editor/mod.rs` 729,
`jumps.rs` 728, `strop-fs/src/notify/manager.rs` 702, `transaction_conformance.rs` 700,
`picker/preview.rs` 700. A further 35 files sit between 600 and 700.

Suggested splits:
- `picker/tests.rs` (995): the directory already has `find_tests.rs`, `search_tests.rs`,
  `query_tests.rs`, `field_register_tests.rs`, `source_sink_tests.rs` beside it. Move the
  remainder into a `picker/tests/` submodule by concern (open/accept, modes/keys, recent/ranking,
  replace) and move those five siblings in with it. Cost: M, risk none.
- `strop-lsp/src/client/api.rs` (806): `api/{mod,navigation,edits,files}.rs` — requests
  (`prepare_request`, `launch_request`, `request`, `workspace_symbols`), edits (`format`, `rename`,
  `code_actions`), file operations (`will_rename_files`, `did_rename_files`). Cost: S-M.
- `strop-remote/src/pool.rs` (785): carries 12 cfg sites; deleting the non-unix arms (1.3) plus
  moving connection lifecycle from admission is the natural cut. Cost: M.
- `editor/resolution/mod.rs` (790): not read in depth; candidate for the next change that touches it.

### 3.2 Structural size

`strop-engine` is 102,880 lines, half the workspace, in one crate; `strop-kernel` (the proved core)
is 675. Largest engine subsystems: picker 8.1K, completion 8.0K, lsp 5.9K, git_memory 4.7K,
ex 4.6K, collections 4.5K, filesystem 3.7K. Any edit recompiles the whole crate and every test
binary linked against it. Not a ceiling violation, but the biggest compile-time lever in the tree.

### 3.3 Load-bearing if-chains

No strong offender found by `else if` density (max 9 in `strop-fs/src/notify/manager.rs`,
6 each in `strop-git/src/{status,memory}.rs`). The real ordering hazards are the `match` guard
ladder in `crates/strop/src/cli.rs:86-290` (each arm guarded by
`command.is_none() && !headless && replay.is_none() && operand.is_none()`; arm order decides
whether `config`/`update`/`edit`/`bench` is a subcommand or a filename) and the duplicated
ladders in `picker/preview.rs` (2.3). Removing the synonyms (2.1) shrinks the first.

---

## 4. Dependencies

293 packages in `Cargo.lock`; ~297 in the binary's normal tree. No openssl, native-tls or
openssl-sys. `git2 = { default-features = false }` holds.

### 4.1 Duplicate versions

| Crate | Versions | Pulled by | Fix |
|---|---|---|---|
| rustix, linux-raw-sys | 0.38 / 1.1; 0.4 / 0.12 | crossterm 0.28 | crossterm 0.29 + ratatui 0.30 |
| unicode-width | 0.1 / 0.2 | ratatui 0.29 (unicode-truncate) | same upgrade |
| syn | 2 / 3 | ratatui 0.29 (instability → darling) | same upgrade, or wait |
| bitflags | 1.3 / 2 | async-lsp → lsp-types 0.95 | none without leaving async-lsp |
| getrandom | 0.2 / 0.4 | ring (via rustls via ureq) | goes with `strop update` (4.2) |
| webpki-roots | 0.26 / 1.0 | ureq 2.12 | ureq 3, or goes with 4.2 |
| hashbrown, windows-sys | 2 and 3 versions | transitive | none |

Action: bump crossterm/ratatui together (clears three duplicates). Risk: medium, the goldens
catch regressions. Cost: M.

### 4.2 Heavy for what they do

- **`strop update` stack**: `ureq` + rustls + ring + webpki-roots ×2 + `indicatif` + `flate2` +
  `tar` + `sha2`, all used only by `crates/strop/src/update.rs` (684 lines). ring is a C/asm build.
  `install.sh` already does the same job and is the documented install path. Deleting the
  subcommand removes ~25 packages, one TLS stack and one network-capable code path from an editor.
  Risk: users lose `strop update` (print the `install.sh` one-liner instead). Cost: S-M. Taste question.
- **`async-lsp`** brings futures, tower-layer/service, tracing, lsp-types 0.95, url/idna,
  waitpid-any. The client already has its own queue, frame limits and decode budget
  (`strop-lsp/src/client/{queue,frame_limits}`), so the framework is used for framing and types.
  Replacing it is L and risky; record as a question, not an action.
- **ghostty-vt via Zig**: `crates/strop-terminal/vendor/` holds 7.2 MB of tarballs
  (ghostty-vt 3.9M, uucode 2.2M, arocc 0.9M, translate-c 0.15M); `build.rs` needs `flate2`, `tar`,
  `cc` and a Zig toolchain (`.github/scripts/install-zig.sh`, `.vscode/settings.json` sets `ZIG`).
  This is the single largest build-complexity item and sits awkwardly with "zero-config" for
  contributors (not for users). Taste question.
- **`imbl`** is in four crates but single-use in `strop-picker/src/catalog.rs` and
  `strop-syntax/src/guides.rs`. **`getrandom`** (one call, `strop-fs/src/context.rs:137`),
  **`unicode-normalization`** (one file, `strop-fs/src/guard.rs`), **`signal-hook`** (one call for
  SIGWINCH, `strop/src/terminal/reader.rs:25`), **`base64`** (one decode in
  `strop-picker/src/source/query.rs:39`), **`rustix-openpty`** (pinned `=0.2.0`, one call) are
  each a dependency for a few lines. `getrandom` could be `/dev/urandom` or rustix; `signal-hook`
  could be rustix/libc already in the tree. Low value individually; S each.
- `tempfile` is a normal dependency of strop-core, strop-engine and strop for atomic saves
  (`buffer/io.rs:203`, `session/persistence.rs:56`). Legitimate.

### 4.3 Path-dependency version pins are weightless

Every inter-crate dependency carries `version = "0.69.0"` (about 60 pins), and AGENTS.md makes
re-pinning them step two of every version bump. They exist for crates.io publishing, which is a
"historical channel", and publishing is already impossible: `strop-kernel` is `publish = false`
and `strop-core` depends on it by path with no version. Action: drop the pins, mark crates
`publish = false`, and the bump becomes one line plus `Cargo.lock`. Risk: none. Cost: S.

---

## 5. Repo-level clutter

### 5.1 Plans (105 files, 2.1 MB, 31,050 lines)

- **`plans/0084-project-indentation-policy.md` is untracked** (169 lines, "proposed, not implemented").
  Flagged by 0087 A15 at 0.53 and still untracked at 0.69. `plans/STATUS.md` does not list it
  (its history list ends "0080–0083"); 0086 and 0089 cite it. Action: commit it and add it under
  Proposed, or delete it. Cost: S.
- **STATUS.md contradicts itself**: 0101-0105 sit under "Proposed — not implemented" while each
  note begins "in force (0.6x)". Move them to "In force".
- **0093, 0094, 0096 have no row** in any table (only mentioned inside other notes).
- **Stale correction**: the 0037 line says devcontainers are "implemented in `strop-containers`
  and `editor/devcontainers`"; neither exists (cut by 0090).
- **Two 0083s** (Splicery, Weft): acknowledged in STATUS. Leave, or renumber one to the unused 0052.
- **AGENTS.md points at a fossil**: over-ceiling files must be recorded "in `plans/0028`", but
  0028 is a 662-line 0.14-0.18 review whose sections include the deleted Windows GUI arc (line 609)
  and the retired 0057 assurance sequence. Neither over-ceiling file is recorded there.
  Action: replace 0028 with a short live roadmap (or point AGENTS.md at STATUS.md) and move the
  old body to history. Cost: S-M.
- **Retired-plan citations in code**: 0058 ×50, 0057 ×28, 0055 ×9, plus 193 ticket tags
  (`VF11`, `WK04`, `R12`, ...) in comments and two `Cargo.toml` files. They point readers at
  retired documents. Rewrite to say what the code does. Cost: M, mechanical.
- Seven "handoff" plans (0041, 0049, 0050, 0065, 0078, 0080, 0081) and four critiques are history;
  fine under the ledger, but they are 15-20% of the directory. Taste question.

### 5.2 Survival-rule and gate issues

- **TLC probation expired.** AGENTS.md: "The two TLC models in `specs/` are on probation until
  0.55"; 0086 line 145: "probation ends 0.55 — delete any that caught nothing". At 0.69 both
  `EditorProtocol` and `SearchLifecycle` (11 tracked files, a CI job, a Docker stage, a
  release gate) are still there on probation. Either record their catch and drop the word, or
  delete `specs/`, the `model` service, the CI job and `transaction_conformance`'s TLA+ table.
  (The proofs auditor may cover whether a catch exists.) Cost: S either way.
- **`ssh-bench` and `bench` compose services** are "measurement, not a gate"
  (`docker-compose.yml:7-9`), and `Dockerfile` has an `ssh-bench` stage plus an unused `ship`
  stage. "Every lane blocks" says a lane that cannot fail a release is deleted. `release-local.sh`
  does run `--bench find/search/symbols/history`, so `bench` has a consumer; `ssh-bench` and
  `ship` have none. Action: delete those two. Cost: S.
- `crates/strop-syntax/grammars/{sovel,splicery,weft}/COVERAGE.md` (188 lines) are hand-kept
  rule-to-EBNF inventories, referenced by no code or test: an evidence inventory by the rule's
  wording. Action: delete or fold two lines into each README. Cost: S.
- `docs/nucleo-decision.md`: a benchmark receipt (see 1.6).

### 5.3 Untracked/ignored local debris (not in git, but in the working tree)

| Path | Size | Note |
|---|---:|---|
| `target/` | 9.9 G | host build output; AGENTS.md says no host `target/` |
| `.claude/worktrees/` | 1.2 G | 8 agent worktrees with 8 `worktree-agent-*` branches; one more worktree is "prunable" |
| `dist/` | 559 M | artifacts from 0.0.0-demo and 0.17.0 through 0.62.0 in three naming schemes |
| `.tlacache/` | 100 K | TLAPS proof cache; TLAPS lane was deleted by 0086 |
| `proofs/__pycache__/` | 20 K | flagged by 0087 A15 |
| `examples/`, `crates/strop/examples/`, `crates/strop-engine/examples/`, `crates/strop-grammar/tests/`, `crates/strop-syntax/tests/` | empty | leftover directories |

92 branches exist locally/remotely. Action: `git worktree prune`, delete merged branches, clear
`dist/` except the last release, remove `.tlacache` and its `.gitignore` line. Risk: the memory
note says packaging reuses `dist/`; keep the current version's directory. Cost: S.

### 5.4 Docs

- `docs/cli.md:27` documents `strop bench`; fine, but see 2.1 for the `--bench` twin.
- `CHANGELOG.md` is 204 KB and is vendored into the site on each release. Taste question.
- README/docs are otherwise consistent with the Linux-only scope.

---

## 6. Binary size and compile time

- **The release binary is 48.7 MB stripped** (`dist/v0.62.0`: 48,663,920 bytes; 12.7 MB tarball).
- **There is no `[profile.release]`** in the workspace `Cargo.toml`, so the release is built with
  defaults: no LTO, 16 codegen units, unwinding. The only size measure is `strip` in
  `Dockerfile:83`. Adding `lto = "thin"` (or fat), `codegen-units = 1` and `strip = true` is the
  cheapest large win in this audit; typical reduction is 20-40%, unmeasured here. `panic = "abort"`
  is a further option but needs a check that terminal restore does not rely on unwinding.
  Risk: longer release build, re-qualify with the perf gate. Cost: S.
- **Grammars dominate.** Generated parser sources in the registry: c-sharp 29 MB, cpp 18 MB,
  sequel (SQL) 18 MB, ruby 15 MB, bash 9.8 MB, rust 6.6 MB, plus typescript (two parsers), php
  (two), and the in-tree `sovel` (1.5 MB `parser.c`), `weft`, `splicery`. 22 crates.io grammars + 3
  in-tree. These are likely over half of the binary and a large share of cold compile time.
  Owner decision which languages earn their megabytes (see questions).
- Other compile-time weight: libgit2 (C), ring (C/asm, only for `strop update`), ghostty-vt
  (Zig), tokio in two crates, and the 103K-line `strop-engine` crate (3.2).
- `--bench` scenarios (1,445 lines in `crates/strop/src/bench*`) ship in the release binary.
  They are used by release qualification, so moving them out means qualifying a different binary;
  leave unless size matters more than that.

---

## Ranked action list

| # | Action | Cost | Risk | Payoff |
|---:|---|---|---|---|
| 1 | Add `[profile.release]` (LTO, codegen-units=1, strip) and measure | S | low | binary size, maybe speed |
| 2 | Delete the 17 dead pub items (1.1) and 2 unused dev-deps (1.5) | S | low | correctness of surface |
| 3 | Fix STATUS.md (0101-0105 section, 0093/0094/0096 rows, 0037 note) and track or delete 0084 | S | none | ledger tells the truth |
| 4 | Decide TLC probation: record the catch or delete `specs/` + `model` lane | S | low | doctrine holds |
| 5 | Remove CLI synonyms and the no-op `edit` (2.1) | S | low | one way to do each thing |
| 6 | Drop path-dep version pins; simplify the version-bump rule (4.3) | S | none | release procedure shorter |
| 7 | Delete legacy wire branches (2.2) and dead update channels (1.4) | S-M | low | less untested code |
| 8 | Delete non-unix cfg arms and target tables (1.3) | M | low | ~30 dead branches gone |
| 9 | Split `picker/tests.rs` and `lsp/client/api.rs`; replace 0028 with a live roadmap | M | none | ceiling rule holds |
| 10 | Merge the picker preview twins (2.3) | M | medium | one resolver |
| 11 | Delete examples, `nucleo-decision.md`, COVERAGE.md, `ssh-bench`/`ship` stages (1.6, 5.2) | S | low | less to compile and keep |
| 12 | Bump crossterm 0.29 / ratatui 0.30 (4.1) | M | medium | 3 duplicate crates gone |
| 13 | Gate or delete test-only pub items (1.2); params structs for the 6 `too_many_arguments` allows | M | low | no suppressions |
| 14 | Local hygiene: prune worktrees/branches, clear `dist/`, `target/`, `.tlacache`, empty dirs | S | low | 11.7 GB |
| 15 | Rewrite retired-plan/ticket tags in comments (5.1) | M | none | comments stand alone |
| 16 | Owner decisions below (update stack, grammars, libgit2) | M-L | varies | largest size/compile wins |

## Taste questions for the owner

1. **`strop update`**: keep a TLS client, ring and ~25 packages in a text editor, or print the
   `install.sh` one-liner? (It is the only network code in the binary.)
2. **Grammars**: which of the 25 are in scope? C#, SQL, Ruby, PHP, CMake and Fish cost tens of MB
   of generated C. Are Sovel/Weft/Splicery product features or personal ones, and should they ship
   to every user?
3. **libgit2**: which paths are truly per-keystroke? If only gutter hunks, is a vendored C library
   worth it when every other git feature, and all remote work, already uses the `git` CLI?
4. **CLI spelling**: subcommands (`strop headless`) or flags (`--headless`)? Pick one family.
5. **Session/trace compatibility**: may 0.70 refuse sessions, registers and traces written by
   older versions (deleting the legacy readers)?
6. **TLC models**: did either catch a real bug since 0.55? If not, the rule says delete.
7. **Plans directory**: keep 105 documents as history in-tree, or archive handoffs/critiques and
   keep only in-force contracts plus the ledger?
8. **ghostty-vt + Zig**: is the build-time Zig toolchain an accepted permanent cost for the terminal?
9. **`async-lsp`**: accepted, or a candidate for a small in-house JSON-RPC framer given the client
   already owns queueing and limits?
10. **`ARROWS` binding**: should up/down in the completion popup appear in `?`, or is the constant dead?
11. **Engine crate split**: is a 103K-line crate acceptable, or should picker/completion/lsp glue
    become crates to cut incremental build time?
