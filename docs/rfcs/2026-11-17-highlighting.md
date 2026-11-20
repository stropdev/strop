---
rfc: "2026-11-17-highlighting"
title: "Keep incremental highlighting correct across edits and windows"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-11-17"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/207"
milestone: "v0.71.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0111"
---

# RFC: Keep incremental highlighting correct across edits and windows

PR [#45](https://github.com/stropdev/strop/issues/45) shipped per-key failure retention, shared language queries, document-wide combined injections, a depth cap that only drops deeper colour, a 3,000-byte per-line budget, and re-detection on write. Child trees are reused only when their edited included ranges still agree; fresh/incremental equality is asserted for well-formed parses, not arbitrary recovery states. K1/K2 are proved in proofs/Kernel/Intervals.lean; the workspace's LF-only rope configuration settled the line-model choice in 0108 ([47c66db](https://github.com/stropdev/strop/commit/47c66db4b00b2895fbe72b837653baa0b976e2a0)).

[Tracking issue #207](https://github.com/stropdev/strop/issues/207) · [Published v0.71.0](https://github.com/stropdev/strop/releases/tag/v0.71.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0111-highlighting.md).

Follow-up issues: [#59](https://github.com/stropdev/strop/issues/59), [#60](https://github.com/stropdev/strop/issues/60), [#61](https://github.com/stropdev/strop/issues/61).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

- Status: **in force** (0.71).
- Source: the owner — "investigate our syntax highlighting, markdown
  treesitter injection … caching … make sure these are correct and have
  corresponding formalization." The audit is appended as §A; this header is
  the decision record and wins where it differs.
- Builds on 0022, 0066, 0073, 0099.

## Decisions
1. A failed analysis is remembered per key; it is not re-requested every frame.
2. Past the injection depth cap, the deeper layers go uncoloured; the rest
   of the window keeps its colour. Silent.
3. Compiled queries are shared per language.
4. Combined injections (Markdown, PHP) are collected document-wide, so a
   closing tag keeps its colour when its opener is off-screen.
5. A child tree survives a range change (edit, not reparse from nothing).
6. Long lines: like vim's `synmaxcol`, a line past a byte budget is not
   highlighted beyond it.
7. Language detection re-runs on write (a typed shebang takes effect at `:w`).
8. Small fixes: dead previous-revision retention, journal-gap guard in
   `apply_edits`, doc-comment range newline, presentation keyed on syntax
   identity.
9. **Proofs**: the presentation's span projection (`fragments`) and the
   coverage algebra (`cover`/`uncover`/`replace_window`/`clip_into`) move
   into `strop-kernel` on top of the proved edit map, with theorems and
   mutants (K1, K2). K3/K4 deferred to the roadmap.
   Implementation record (D9): `strop_kernel::intervals` holds `project`
   (K1, over `map_position`), `cover`, `uncover`, `replace_window` and
   `clip` (K2); `presentation.rs` keeps its signatures and calls them.
   `project_spec`: a byte after the edit is covered by source `i` exactly
   when it lies before the edit and was, or after the inserted text and its
   pre-image was; sorted stays sorted. `cover_spec` union, `uncover_spec`
   difference, `clip_spec` intersection, `replace_window_spec` fresh inside
   the window and old outside — each keeping lists sorted. `cover` and
   `replace_window` became three passes (before, merged or fresh, after),
   equal to the old merge for sorted input; `clip` clips the slice binary
   search finds (the search stays in the engine, from `std`).
10. **Properties that block**: seeded "incremental == from scratch" and
    "windowed == full, clipped" over rs/md/html/php/py, and an interleaving
    property on the analysis worker seam, with bounded step budgets.

### Implementation notes (decisions 1–8, 10)
- **5, narrowed.** A child keeps its tree only when the journal-edited
  tree already has exactly the new included ranges (typing inside or
  before a block). Reusing a tree whose included ranges differ from the
  new ones disagreed with a fresh parse under the seeded property and
  crashed tree-sitter 0.25 (SIGSEGV), so that case parses from nothing.
  Most of a keystroke inside a large Markdown fence is the host
  `tree-sitter-md` block grammar itself, whose incremental reparse of a
  10k-line fence costs as much as a cold parse (~60 ms on a string); the
  child's own share (~25 ms) is gone.
- **6.** `LINE_BUDGET` = 3000 bytes: vim's `synmaxcol` default in vim's
  unit (its columns are byte indices). Applied in `strop-syntax`, per
  line, so windowed == full still holds.
- **7.** The key carries the detected `LanguageId`; the shebang is read
  at open, write and reload.
- **10.** "incremental == fresh" holds only for well-formed parses:
  tree-sitter's error recovery may settle differently after an
  incremental reparse (reproduced on raw tree-sitter-rust, no injections).
  The property compares when the fresh layers have no error node. Exotic
  line breaks (L2) stay out of the alphabets until the line model is
  decided.

---

## §A. Audit


Read-only audit. Nothing in the repo was edited or built. Evidence marked **[RUN]** was
measured with two throwaway crates built outside the repo (path-deps on `strop-syntax`,
`strop-core`, `strop-engine[test-support]`, own target dir, `--offline`):

- `<audit scratch>/fuzz/src/main.rs` — differential fuzz + timing probe
- `<audit scratch>/eng/src/main.rs` — engine probe (real `Editor`, real worker)

Evidence marked **[READ]** is from reading the code only and was not executed.

Paths below are relative to `/home/tarek/workspace/strop/`.
`S/` = `crates/strop-syntax/src/`, `A/` = `crates/strop-engine/src/editor/analysis/`.

---

## 1. Architecture map

### 1.1 Scheduling: everything is on one background thread

- One `display-analysis` thread owns all parser state (`A/worker.rs:67-72`). Messages are
  `Analyze | Match | Edits | Forget` on a single `mpsc` channel (`A/worker.rs:31-36`), so
  order between an edit journal and the analysis of the resulting rope is the send order.
- The only `Highlighter` constructions in production are `A/worker.rs:50,58`. No
  tree-sitter parse for highlighting exists on the input→render path. Paint calls the
  readonly `document_analysis_cached` (`A/mod.rs:281`, used at
  `crates/strop/src/render/buffer/rows/mod.rs:35`); admission (`document_analysis`,
  `A/mod.rs:299`) is called from frame preparation (`editor/prepare.rs:313`). The async
  invariant holds for highlighting.
- One request in flight per target (`pending: HashMap<AnalysisTarget, Pending>`,
  `A/mod.rs:70`). A pending request for another revision or search is cancelled through an
  `AtomicBool` polled by the parser and query progress callbacks (`A/mod.rs:318-325`,
  `S/lib.rs:387,330`); a pending request for another *window* of the same revision is not
  cancelled, the new window waits for it.
- One job computes highlights, line layouts, indent guides and the search summary together
  (`A/worker.rs:110-189`), so highlight latency includes the other three.

### 1.2 Incremental edits

- `Buffer::replace_validated` builds the `InputEdit` (`crates/strop-core/src/buffer/mutation.rs:246-265`):
  bytes are exact; points are `(rope.byte_to_line, byte - line_start)` for start/old end
  (`buffer.rs:331-340`) and a `\n` count for the inserted text (`point_extent`, `buffer.rs:430+`).
  Columns are bytes (what tree-sitter wants). No UTF-16 anywhere on this path.
- `publish_change` bumps the revision by exactly one per change (`mutation.rs:310-326`);
  undo/redo replay through the same function (`mutation.rs:476-509`), so revisions are
  monotonic and never reused. `replace_rope` (reload) publishes one whole-buffer edit
  (`mutation.rs:402-428`).
- The journal reaches analysis from two places only: `editor/transact.rs:217` and
  `editor/collections/view_positions.rs:70`. `AnalysisState::edits` (`A/mod.rs:137-160`)
  cancels the pending job, folds the journal through the presentation, and forwards it to
  the worker.
- Worker: `Highlighter::apply_edits` (`S/lib.rs:207-238`) calls `tree.edit` per change and
  recurses into injected children. `ensure_tree` (`S/lib.rs:372-409`) reparses against the
  kept tree, and drops the tree when `tree_revision != revision` (`S/lib.rs:383`).
- CRLF: `\r\n` is one ropey break and one `\n` for tree-sitter, consistent. Other breaks
  are not, see L2.

### 1.3 Injections (`S/injections.rs`, `S/injections/documentation.rs`)

- Driven by each language's `injections.scm`. `collect` (`injections.rs:144-276`) runs the
  injection query **restricted to the window** (`:154`), reads `injection.language`
  (property or capture), `injection.filename`, `injection.shebang`, `include-children`,
  `include-unnamed-children`, `combined`, and strop's own `injection.documentation`.
- Language resolution: explicit name → `languages::for_name` (aliases, case-insensitive,
  `source.`/`text.` stripped, `languages.rs:451`), filename → `detect`, shebang →
  `for_shebang`. Precedence `Explicit > Filename > Shebang` (`:22-27`, `:238-247`); an
  explicit unknown language suppresses a shebang fallback on the same ranges (`:259-263`).
  Unknown language keeps host treatment.
- Markdown: fenced blocks (language or shebang), `inline` and table cells →
  `markdown.inline`, front matter → yaml/toml, `html_block` → html **combined**
  (`queries/markdown/injections.scm`); `markdown.inline` `html_tag` → html **combined**;
  PHP `text` → html **combined**.
- Doc comments: `Policy::{RustLine, Block, GoBlock, Python}` compute host-byte ranges with
  comment prefixes stripped (`documentation.rs:34-225`); Rust line docs are grouped by
  walking siblings on adjacent rows (`:56-97`).
- Ranges are canonicalized (sorted, merged, `:316-335`) and intersected with the parent
  parser's included ranges (`:338-370`) before parsing, so prefixes excluded at one level
  never become tokens at the next.
- Nesting: each injection is a full child `Highlighter` (own `Parser`, own compiled
  `Query` pair, own tree, own children), recursion capped at depth 8 with a hard error
  (`:75-78`).
- Children are kept only for injections found in the current window (`:79`, `:131`).
  Reuse: exact `(language, ranges)` match, else first child of the same language
  (`:85-94`); any range change does `invalidate()` (`:106-115`), i.e. a from-scratch parse.

### 1.4 Query execution and layering

- `highlight_cancellable` (`S/lib.rs:309-367`): `QueryCursor` with `set_byte_range(window)`,
  predicates read rope chunks (`RopeText`, `:131-171`), host captures are collected as
  `LayeredSpan{injected:false}`, child spans clipped to the child's ranges as
  `injected:true` (`injections.rs:117-130`).
- `spans::flatten` (`S/spans.rs:38-95`) sweeps start/end events; winner per byte is
  `max (injected, shortest, latest capture index)`; emphasis is OR-ed over all active
  captures; adjacent equal spans merge. Output is sorted, disjoint, clipped to the window.

### 1.5 Cache layers

| # | Layer | Where | Key | Invalidation | Version check |
|---|---|---|---|---|---|
| 1 | Parse tree | `Highlighter.tree` (`S/lib.rs:189-191`) | `tree_revision` | `apply_edits`, `invalidate`, mismatch at `:383` | `source_hash == revision` (`:379`) |
| 2 | Flattened spans | `Highlighter.spans` + `span_window` (`:187,192`) | window, implicitly revision | reset by every successful `ensure_tree` (`:407`) | `span_window == Some(window)` (`:325`) |
| 3 | Injected children | `Highlighter.children` (`:195`) | `(language, ranges)` | replaced every `injection_spans`; `invalidate` on range change | same as 1-2, recursively |
| 4 | Worker slot | `HashMap<AnalysisTarget, Slot>` (`A/worker.rs:25-30,72`) | target; highlighter rebuilt when `syntax_path` differs (`:55-60`) | `Forget` | none beyond 1-2 |
| 5 | Exact frame cache | `AnalysisState.cache` (`A/mod.rs:71`), ≤ 8 per target | full `AnalysisKey` = target, revision, first, last, tab, guides, left, right, syntax_path, search (`:23-36`) | `forget`; retain `revision >= current-1 && tab ==` (`:580`) | key equality (`:231-237`); acceptance requires request id and live revision (`:523-540`) |
| 6 | Presentation (0066 interim) | `AnalysisState.presentations` (`A/presentation.rs:32-49`) | `mapped_revision` | journal projection (`:115-136`), gap ⇒ clear, caps (`:28-30`), `fail` | `mapped_revision == buf.revision()` (`A/mod.rs:269`) |
| 7 | Interim memo | `Presentation.interim` (`presentation.rs:48`) | `MemoKey` (`:52-61`) | cleared on every mutation | memo equality |
| 8 | Layouts, guides | `A/layouts.rs`, `Slot.guides` | (target, revision, tab) | owner mismatch clears | — (not syntax) |

Preview targets use a constant revision 0 (`A/mod.rs:345,375`); correctness relies on
`forget` whenever a preview's rope is replaced or evicted (`editor/picker/drain.rs:203-214`,
`picker/mod.rs:360`, `filesystem/reconcile.rs:309`). I found no path that replaces a preview
rope without it.

### 1.6 Reaching the renderer

`handle_analysis` (`A/mod.rs:505-586`) drops a completion unless its request id is the
pending one and the document's revision still equals the key's; then installs layouts,
`Presentation::accept`s the frame and caches it. Paint asks for the exact key, else the
interim frame (accepted spans projected through the journal, clipped to the window,
`presentation.rs:163-210`). `content_spans` (`crates/strop/src/render/buffer/content.rs:44-80`)
walks graphemes and patches the style of the span containing the grapheme's first byte.

### 1.7 Kernel usage

The syntax path calls **nothing** in `strop-kernel`. `grep strop_kernel::` hits
`strop-core/buffer/mutation.rs`, `editor/cache.rs` (symbols only, `symbols/store.rs:88`),
`snippet.rs`, `transact.rs`, `collections/*`, `panes/layout.rs`, `completion/acceptance.rs`.

- `presentation.rs:250-257` (`fragments`) is a second, unproved edit map. Its after-part is
  `map_position` applied to `max(a, old_end)` and `b`, re-derived inline (`shift`).
- The exact frame cache (layer 5) decides validity itself, although `kernel/cache.rs:1-4`
  says "no flow decides validity for itself". Plan 0099 does not mention this cache.
- `excerpt.rs`, `layout.rs`, `generation.rs` are unrelated to syntax.

---

## 2. Correctness findings

Ordered by impact. B = bug with a reproduction, P = pathological input, L = latent/low.

### B1. A failed analysis is re-requested on every frame, forever  [RUN]

- **Where:** `exact_analysis` returns `None` for a cached failure (`A/mod.rs:236`,
  `.and_then(|entry| entry.value.clone())`), so `document_analysis` (`:308-328`) treats it
  as a miss and calls `request_analysis` again. The preview path does it right
  (`:385-391` returns the cached `None`).
- **Scenario:** open the file from B2 (or any input that makes the job fail or panic
  deterministically).
- **Measured:** engine probe, 20 prepare-then-drain rounds on an unchanged buffer →
  **20 requests**, message `analysis: syntax injection nesting exceeds eight levels` each
  time. With 6 or 7 levels: 1 request.
- **Expected:** one request; the failure is remembered for that key until the key changes.
- **Not verified:** that the real event loop turns each completion into a new prepared
  frame (I did not run the TUI). If it does, this is a busy loop on the worker and the
  event loop, and headless `settle` never settles, since `prepare.rs:472-479` reports
  "not settled" while `exact_analysis_cached` is `None`.
- **Also:** `request_analysis`'s start-failure branch pushes a `Cached{value: None}`
  without the 8-entry bound (`:443-449`), once per frame.
- **Fix:** make the exact probe return `Option<Option<Arc<_>>>` (hit-with-failure vs miss)
  and stop on a failure hit; bound the push at `:447`.
- **Regression test:** engine test with the completion seam used at `A/mod.rs:702-720`:
  deliver `Outcome::failed` for the current key, call `document_analysis` twice, assert
  `admissions.analysis` did not move and `analysis_pending_probe()` is false.

### B2. Nine nested languages blank the whole window  [RUN]

- **Where:** `injections.rs:75-78` returns `HighlightError::InjectionDepth` for the whole
  call; the worker maps it to `Outcome::Failed` (`A/worker.rs:196-198`).
- **Scenario:** a Markdown file with eight nested ` ```markdown ` fences (fence lengths
  11 down to 4 backticks) around a line of prose, followed by an ordinary ` ```rust `
  block. Generator: `nested(8)` in the engine probe.
- **Actual:** no syntax colour anywhere in the window, including the unrelated Rust block,
  plus B1's loop. Direct call returns `Err(InjectionDepth)`; levels ≤ 7 return spans.
- **Expected:** the ninth level keeps its host treatment (what an unknown language gets);
  everything else is coloured.
- **Fix:** at the cap, return `Ok(Vec::new())` for that level instead of an error. If the
  owner wants it reported, report once, not as a failed frame.
- **Regression test:** `strop-syntax` test: `nested(8)` highlights `Ok`, and the trailing
  Rust fence has a `Keyword` span on `fn`.

### B3. Combined injections depend on the viewport  [RUN]

- **Where:** `collect` restricts the injection query to the window (`injections.rs:154`),
  then merges `injection.combined` ranges among what it found (`:264-273`). A combined
  injection is by definition one parse over all its ranges in the document.
- **Scenario (Markdown):**
  ```
  <div>

  para

  </div>
  ```
  Scroll so `<div>` is above the top row. **Actual:** `div` in `</div>` loses its `Tag`
  colour; scroll back and it returns. Same for PHP: `</div>`/`</html>` closers after a
  `<?php ?>` island lose colour when the opener is off-screen.
- **Measured:** fresh windowed highlight ≠ fresh full-document highlight clipped to the
  same window: PHP 161 of 625 fuzz steps, Markdown 15 of 625, always a missing `Tag` on a
  closing tag. HTML, Python, Go: 0.
- **Expected:** a byte's colour does not depend on where the window starts (0066's target).
- **Fix:** for patterns with `injection.combined`, collect over the whole tree (cache the
  list per revision in the `Highlighter`), then highlight the child inside the window.
  Only md / md.inline / php pay for it.
- **Regression test:** the property in §4 (P-window); plus one golden: the Markdown above,
  window starting at the `para` line, assert `Tag` on `div`.

### P1. Every injected block compiles its own queries  [RUN]

- **Where:** `Highlighter::from_spec` runs `Query::new` for highlights and injections
  (`S/lib.rs:253,257`), and is called per injection, not per language
  (`injections.rs:99-100`). Children that leave the window are dropped (`:79,131`).
- **Measured (release):** cold highlight of `fn main() {}` 24 ms; a 9-line Rust file with
  one doc group containing a rust and a python fence 63 ms; Markdown with 5 fences in 5
  languages 131 ms; Markdown with 40 rust fences in one window 987 ms (≈ 24 ms per fence),
  after which a one-byte edit costs 1.7 ms.
- **Effect:** first colour on a documented Rust file or a README is delayed by tens to
  hundreds of ms; a jump (`G`, `<C-d>`) into a region with more blocks of one language
  than the previous window pays 24 ms per extra block. Not a blocking parse (worker
  thread), but it is latency the user sees as late colour.
- **Fix:** a process-wide `LanguageId → Arc<(Query, Option<Query>, Vec<CaptureStyle>)>`
  table (`OnceLock`); `Query` is `Send + Sync`. `Highlighter` keeps only parser + tree.
- **Regression test:** behaviour is unchanged, so guard with a count: a test-only counter
  of query compilations, assert it equals the number of distinct languages after
  highlighting the 40-fence document.

### P2. Typing inside an injected block reparses the block from scratch  [RUN]

- **Where:** any change in a child's ranges calls `invalidate()` (`injections.rs:106-115`),
  which drops the tree that `apply_edits` just edited (`S/lib.rs:234-237`). An edit inside
  a block changes its end; an edit anywhere before it shifts it. So the incremental path
  for children only survives same-length replacements and edits after the block.
- **Measured:** one keystroke inside a 10 000-line ` ```rust ` fence: 108–120 ms. Same
  text as a plain `.rs` file: 3.4 ms. Unterminated ` ```rust ` at the top of a
  20 000-line Markdown file (the rest of the file becomes the injection): 700 ms per
  keystroke.
- **Effect:** the 0066 interim frame keeps old colours on screen for that long; with fast
  typing every job is cancelled before it finishes, so colours stay stale until a pause.
- **Fix:** on a range change keep the edited tree: `set_included_ranges(new)` then parse
  with the old tree (tree-sitter diffs included ranges itself). Keep `invalidate()` only
  when the child is being rebound to a different injection (the language-only fallback at
  `:90-94`). Do exact matches for all injections before any language-only fallback, so one
  shifted block does not steal another block's tree.
- **Regression test:** §4 P-inc already covers correctness; add a bound: after one edit
  inside a 2 000-line fence, the child's reparse changed-range count is small (expose
  `Tree::changed_ranges` length in a test hook), or simply a time-free assertion that the
  child's tree was reused (`tree.is_some()` before the parse).

### P3. One huge line is highlighted whole, every keystroke  [RUN]

- **Where:** the window is line-granular (`prepare.rs:308-311`); nothing clips it to the
  visible columns, although the key already carries `left/right`.
- **Measured:** a 2.4 MB single-line `.js`: 1.42 s cold, 1.06 s per keystroke, 900 004
  spans = 21 MB per copy. Copies: `Highlighter.spans`, the clone at `S/lib.rs:366`, the
  cached frame, the presentation (`MAX_SPANS` at `presentation.rs:83-88` clears the old
  data but then stores all 900k), plus the previous revision's frames kept by `:580`.
- **Fix:** for lines over a byte budget, highlight only the byte range the layout
  checkpoint says is visible (plus a margin); or refuse syntax over a per-line budget and
  say so in the modeline. Owner's call (taste Q3).
- **Regression test:** highlight request for a 1 MB line with `left=0,right=120` returns
  fewer than N spans and all of them inside the visible byte range.
- 20 000-deep JSON nesting is fine (17 ms, 3 spans).

### B4. Language detection is frozen at first use  [RUN]

- **Where:** `slot_for` resolves `Highlighter::for_path(path, rope)` once and again only
  when `syntax_path` changes (`A/worker.rs:47-60`). Detection also depends on the rope's
  first line (shebang), which is not part of the slot's identity.
- **Scenario:** open an extensionless file `script` containing `echo "hi"`. Type
  `ggO#!/bin/bash<Esc>`. **Actual:** 0 spans, until the buffer is closed and reopened
  (saving does not help: same document, same path). A fresh open of the same text gives
  11 spans. Conversely, deleting the shebang keeps bash colours (11 spans).
- **Expected:** owner's call (taste Q2); at minimum re-detect after `:w` and reload.
- **Fix:** carry the detected language id in `AnalysisKey` instead of only the path, and
  re-detect on the engine side when line 0 changes or on write.
- **Regression test:** engine test with exactly those keys, assert non-empty spans after
  the next settled analysis.

### L1. Rust doc range swallows the next blank line's newline  [RUN, cosmetic]

- **Where:** `documentation.rs:82-87` appends `\r` and `\n` after the node "to keep exactly
  one newline", but tree-sitter-rust's doc comment node already ends after its newline, so
  when the next line is empty its `\n` joins the injection.
- **Evidence:** fuzz found windowed ≠ full at one byte (`Code` on a blank line's `\n`):
  the full parse includes the byte, a window starting at that blank line does not see the
  comment at all. Invisible on screen (the byte is a newline).
- **Fix:** append only when `rope.byte(end - 1) != b'\n'`.
- **Test:** the property P-window in §4 catches it.

### L2. Points handed to tree-sitter use ropey's line model, tree-sitter counts `\n`  [READ + RUN negative]

- ropey 1.6.1 default features (`unicode_lines`) treat VT, FF, lone CR, NEL, U+2028,
  U+2029 as breaks. `point_of` uses `rope.byte_to_line` (`buffer.rs:331-340`), while
  `point_extent` counts `\n` only, and `documentation.rs:233-240` uses `byte_to_line` for
  included-range points. So within one `InputEdit`, start/old-end rows and the inserted
  extent use different models, and both differ from tree-sitter's rows after such a byte.
- Highlight spans are byte-based, and the fuzz with those six characters in the token set
  (750 steps per language, six languages) found **no** incremental ≠ fresh divergence.
- Rows from the tree are read at `documentation.rs:63,93,109` (doc-group adjacency). After
  deleting a range containing one of those characters, reused nodes carry rows shifted by
  the wrong amount relative to freshly lexed ones. I could not produce a wrong highlight
  from it; treat as latent.
- **Fix:** compute points with a `\n`-only count for the syntax journal (one helper in
  `strop-core`), or build ropey without `unicode_lines` if the editor's line model is
  meant to be vim's (taste Q6).

### L3. Worker trusts the journal to be gap-free  [READ]

- `apply_edits` (`S/lib.rs:207-238`) applies whatever arrives and sets
  `tree_revision = revision`. `ensure_tree`'s guard (`:383`) only catches "last applied
  revision ≠ requested". A missing middle batch (5 → 7 with 6 lost) would pass the guard
  and reuse a tree that never saw edit 6.
- Today the journal is gap-free by construction (two publication sites, single channel),
  and the UI-side twin already guards the same hazard (`presentation.rs:117-124`). The
  worker has no such guard and no `debug_assert`.
- **Fix:** in `apply_edits`, `if self.tree.is_some() && revision != tree_revision + 1 { self.tree = None; }`.
- **Test:** unit test: parse at 0, apply edit for revision 2 only, highlight at 2 equals a
  fresh highlight.

### L4. Presentation does not know the syntax identity  [READ]

- `interim_analysis` (`A/mod.rs:261-274`) checks only the revision; `accept`
  (`presentation.rs:82-109`) replaces only its window. After the syntax path changes
  without an edit (`:saveas b.py` from `a.rs`), spans of the old language outside the
  first accepted window stay retained and are served as the interim frame on scroll until
  each window's exact result lands. Self-healing, wrong colours for a frame or two.
- `accept` also never checks `self.mapped_revision == key.revision` before merging; it
  holds today because every revision bump goes through `apply`.
- **Fix:** store `syntax_path` in `Presentation`; clear on mismatch in `accept` and refuse
  in `interim_frame`; `debug_assert_eq!(mapped_revision, key.revision)` when data is retained.

### L5. Dead retention and unbounded hidden state  [READ]

- `A/mod.rs:577-580` keeps the previous revision's frames "for the newest-first exact
  probes". Every probe compares the full key including the current revision
  (`:231-237`, `:398-404`), and revisions are never reused, so those entries can never be
  served. They cost up to 8 frames of spans + layouts per document. Delete (retain
  `== key.revision`).
- Worker slots hold a full tree per open document until close (`A/worker.rs:93-95`); there
  is no eviction for hidden buffers. Bounded by open documents; worth a cap only if
  sessions with hundreds of buffers are a target.

### Checked and found sound

- **Stale result painted?** No. An exact frame is served only on full key equality with
  the current revision; a completion is accepted only when its request id is the pending
  one and the live revision matches (`A/mod.rs:523-540`). A stale completion that won the
  race against the cancel flag is dropped.
- **Undo / redo / reload:** new revisions, journal published, tree edited. Incremental ==
  fresh held in 3 750 + 4 500 fuzz steps (plain + exotic) across rs, md, html, php, py, go
  including CRLF, multibyte, fence-language changes and multi-edit batches: **0
  divergences, 0 panics, 0 unstable repeats**.
- **Injection ranges after an edit:** never shifted, always recomputed from the reparsed
  host tree; children with changed ranges are invalidated. Correct, expensive (P2).
- **`fragments` / `project_ranges` / `cover` / `uncover` / `replace_window` / `clip_into`:**
  I checked the boundary cases by hand (insert at span start, at span end, inside;
  replacement straddling one or both ends; touching coverage) and found no off-by-one.
- **Blocking parse on input→render:** none for highlighting.

---

## 3. Formalization

### 3.1 Move into the kernel and prove (pure, small, load-bearing)

**K1. Edit-mapping of retained spans** — highest value: it is the one place where an old
colour is deliberately painted on a new buffer, and it is a hand-rolled copy of proved math.

- Move: `fragments` and `project_ranges` (`A/presentation.rs:250-300`) to
  `crates/strop-kernel/src/spanmap.rs` as
  `project(ranges: &[(usize, usize)], start, old_end, new_end) -> Vec<(usize, usize, usize)>`
  (third field = source index, so `Span` class/emphasis and coverage both reuse it; the
  `ByteRange` trait goes away). Express the after-part with `editmap::map_position` so the
  existing `map_position_spec` is reused instead of re-derived.
- Theorem, in words: for a well-formed edit and a sorted disjoint non-empty input, the
  output is sorted, disjoint and non-empty; a post-edit byte is covered **iff** it lies
  outside the inserted text and its pre-image was covered, and then it carries its
  pre-image's source index.
  ```lean
  theorem project_spec (rs : Slice (Usize × Usize)) (s oe ne : Usize)
      (hs : s.val ≤ oe.val) (hn : s.val ≤ ne.val) (h : SortedDisjoint rs) :
      ∃ out, spanmap.project rs s oe ne = ok out ∧ SortedDisjoint out ∧
        ∀ y i, CoveredBy out y i ↔
          (y < s.val ∧ CoveredBy rs y i) ∨
          (ne.val ≤ y ∧ CoveredBy rs (y - ne.val + oe.val) i)
  ```
  The `↔` gives both directions: inserted bytes never borrow a class, surviving bytes
  never lose one.
- Mutants that must break it: `a < start` → `a <= start` (empty fragment);
  `b.min(start)` → `b` (paints inserted text); `b > old_end` → `b >= old_end`;
  shift from `start` instead of `new_end`.

**K2. Coverage and window replacement** — `cover`, `uncover`, `replace_window`,
`clip_into` (`presentation.rs:333-458`) to `kernel/intervals.rs`.

- Theorems: `cover` = set union with `[first,last)`, `uncover` = set difference,
  `clip_into` = intersection, each preserving sorted-disjoint; `replace_window`: inside
  the window the result is exactly `fresh`, outside exactly the old spans.
- Corollary (the 0066 claim, currently only a `debug_assert` at `:232-240`): `accept`,
  `apply` and `fail` each preserve "every span lies inside coverage".
- Mutants: `b <= first` → `b < first` in `replace_window`; `a < first` → `a <= first` in
  `uncover`; drop the `flushed` push in `cover`. Note `cover`'s `<`/`<=` on touching
  intervals is not observable by a membership theorem; state "no two output intervals
  touch" if coalescing matters.
- Finding while reading: `fail` (`presentation.rs:140-152`) drops a span that straddles
  the failed window instead of clipping it. Safe direction, but the theorem should say
  which is intended.

**K3. Injection range algebra** — `canonicalize`, `intersect`, and the child-span clip
loop (`injections.rs:117-130`, `:316-370`), over byte pairs with the point donors returned
as indices.

- Theorems: `canonicalize` output is sorted, pairwise non-touching, same byte set;
  `intersect` = set intersection of two sorted disjoint lists, sorted disjoint; every
  emitted injected span is inside `span ∩ range` for one child range and their union is
  `span ∩ ⋃ranges`.
- Mutants: `end_byte <= range.start_byte` → `<` in the parent skip; `>=` → `>` in the
  merge test; `take_while(start_byte < span.end)` → `<=`.

**K4. Layering** — `spans::flatten` (`S/spans.rs:38-95`). Worth proving, most work: it
uses `sort_unstable`, `retain` and `max_by_key` closures, none kernel-shaped. Split it:
sorting stays outside; the kernel gets the sweep over pre-sorted events, checks sortedness
in a loop, and returns winners.

- Theorem: output sorted, disjoint, inside `[first,last)`; for each byte, class = class of
  the max `(injected, shortest, latest)` capture containing it, emphasis = OR over all
  captures containing it; bytes in no capture are absent.
- Mutants: drop `injected` from the key (host wins over injection); `usize::MAX - len` →
  `len` (outermost wins); emphasis from the winner only.

**K5. Not worth a proof:** the acceptance predicate (`request ids equal ∧ revisions equal`)
is one line of `==`; `strop-core` already has the same shape as plain functions
(`viewguard.rs:8`, `searchguard.rs:16`). Call one of those instead of a third inline copy.
Span splitting at line boundaries does not exist as code: spans are document-byte ranges
and the renderer indexes into them per row with `partition_point` (`content.rs:44-46`).

### 3.2 Protocol-shaped: test, not TLA+

The UI ↔ worker protocol (Edits / Analyze / Forget, cancel flag, one pending per target,
revision check on acceptance) is where B1 lives. I would not add a TLC model: the two in
`specs/` are on probation, and the state here is deterministic given an event order (one
`mpsc`, one `AtomicBool`). Loom adds nothing for the same reason. A model-based property
test over event interleavings, driving the real `Editor` through the completion seam
already used at `A/mod.rs:702-720`, is the artefact with a recorded catch:

Operations: `edit`, `undo`, `scroll`, `resize`, `deliver next completion`,
`deliver out of order (older request id)`, `fail completion`. Invariants after every step:

1. Any frame from `document_analysis_cached` is either an exact entry whose key revision
   equals the live revision, or equals `project(journal, accepted spans)` clipped to the
   window (K1's spec as the oracle).
2. Every painted span lies inside presentation coverage.
3. At quiescence (no pending, no input), the window has an exact frame **or** a recorded
   failure, and a further prepare issues **no** request. (Fails today: B1.)
4. At quiescence the exact spans equal a fresh `Highlighter` on the same rope.
5. Worker-side (unit level): the kept tree has seen exactly the journal
   `(parsed revision, requested revision]` in order (L3).

If the owner still wants a TLA+ statement, the invariants are 1 and 3:
`Painted(f) ∧ f.exact ⇒ f.rev = doc.rev` and
`Quiescent ⇒ (Cached(window, doc.rev) ∨ Failed(window, doc.rev)) ∧ pending = {}`,
with the mutant "serve a cached failure as a miss".

---

## 4. Tests

### What exists

- `S/tests.rs`: per-language class smoke tests; `window_and_cache_semantics_hold` (one
  window on one line); three incremental-vs-clean tests with hand-picked edits
  (`:161-222`), one on a multi-chunk rope (`:280`); backtracking-request survival (`:310`);
  one Markdown fence test and `editing_a_fence_language_retires_old_injected_spans` (`:381`);
  HTML script/style (`:454`).
- `S/tests/documentation.rs`: nine doc-injection tests including CRLF/unicode offsets,
  unclosed fences per group, nested ranges with a viewport, and one incremental test.
- `S/spans.rs`: one `flatten` test.
- `A/presentation.rs`: 19 unit tests of projection, coverage, caps, rails, memo.
- `A/mod.rs`: three engine tests (edit serves mapped spans; scroll serves overlap; failure
  uncovers).
- `crates/strop/src/render/buffer/tests/`: cell-grid tests `syntax_stability.rs` (typing a
  space does not flash; scrolling keeps the overlap), `documentation.rs` (one),
  `syntax_0083.rs` (weft, splicery heredoc across an edit, picker previews).

### Gaps

- No property test anywhere in the syntax path (no proptest/quickcheck in any manifest).
- No test that a windowed result equals the full result clipped, beyond one single-line
  case. B3 and L1 live exactly there.
- No test for a failed analysis being re-requested (B1), for the depth cap (B2), for
  shebang changes (B4), for a syntax-path change against the presentation (L4).
- No test with two panes on one document, or with out-of-order completions.
- No pathological-input test (huge line, giant fence, unterminated fence).

### The property test that finds bugs fastest

Three properties over a seeded xorshift (no dependency, hermetic), per language with
injections (rs with docs, md, php, html, py, go), token alphabets that include fence
markers, comment delimiters, `\r\n`, multibyte, and the six exotic breaks:

- **P-inc:** after each batch of 1–2 random replacements fed through
  `Buffer` → `apply_edits`, `highlight(window)` on the long-lived highlighter equals
  `highlight(window)` on a fresh one. (Holds today: 0 failures in 8 250 steps.)
- **P-window:** fresh `highlight(window)` equals fresh `highlight(0..len)` clipped to the
  window. **Fails today** within the first seed on md and php (B3) and within 13 seeds on
  rs (L1). This is the fastest bug finder.
- **P-repeat:** asking twice returns identical spans. (Holds.)

The scratch harness implements all three and can be lifted into
`crates/strop-syntax/src/tests/` nearly as is. It is slow until P1 is fixed: each fresh
highlighter recompiles every query (≈ 60 ms per step for the Rust case), so land P1 first
or the test costs minutes.

---

## 5. Ranked actions

| # | What | Why | Cost |
|---|---|---|---|
| 1 | Remember failed analyses per key (B1) | request per frame forever; probable busy loop | S |
| 2 | Depth cap degrades to host treatment (B2) | one crafted file removes all colour and triggers #1 | S |
| 3 | Share compiled queries per language (P1) | 24 ms per injected block; makes #6 affordable | S |
| 4 | Collect combined injections document-wide (B3) | colours change with scroll position in md/php | M |
| 5 | Keep the child tree across range changes; exact-match pass first (P2) | 108 ms vs 3 ms per keystroke in a big fence; 700 ms with an unterminated fence | M |
| 6 | Land P-inc / P-window / P-repeat in `strop-syntax` tests | only counterexample finder for this path; already has two catches | M |
| 7 | Kernel K1 + K2 with proofs and mutants | replaces a drifting copy of `map_position`; turns 0066's coverage invariant into a theorem | M |
| 8 | Engine interleaving property test (§3.2) | covers the protocol where B1 was | M |
| 9 | Long-line budget (P3) | 1 s per keystroke and ~100 MB on a 2.4 MB line | M |
| 10 | Delete previous-revision retention (L5) | weightless code, real memory | S |
| 11 | Journal contiguity guard in `apply_edits` (L3) | one line; closes a silent-misparse hazard | S |
| 12 | Doc-comment newline (L1) | cosmetic; needed for P-window to go green | S |
| 13 | Presentation carries syntax identity (L4) | wrong-language interim colours after `:saveas` | S |
| 14 | Shebang re-detection (B4) | needs an owner decision first | S–M |
| 15 | Kernel K3, then K4 | range algebra and layering as theorems | M, L |
| 16 | Point model (L2) | latent; depends on the line-model decision | M |

## 6. Taste questions for the owner

1. Depth overflow: silently keep host treatment, or keep host treatment and say so once
   in the modeline?
2. When should language detection re-run for a path without a recognised extension: on
   every first-line edit, on write/reload only, or never (vim's default)?
3. Huge lines: highlight only the visible columns, or refuse syntax past a per-line byte
   budget and say so?
4. Combined injections: pay a document-wide injection query per revision for md/php, or
   drop `injection.combined` from those three queries and accept per-block HTML?
5. Does the display-analysis frame cache fall under 0099's "one proved cache core" rule
   (move it onto `strop_kernel::cache`), or is it exempt because the key is a window, not
   a version?
6. Line model: should ropey's `unicode_lines` stay on? vim and tree-sitter break only on
   `\n`; FF/VT/NEL/LS/PS/lone CR are breaks in strop today.
7. Is a seeded differential fuzz acceptable as a blocking test lane, and with what step
   budget?
