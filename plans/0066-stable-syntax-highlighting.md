# 0066 — Stable syntax highlighting while typing

Status: **implemented** (2026-10-04) — the user authorized executing this plan
and cutting the release in the same session. The §2 baseline (98 flashed cells)
measures **0** on the resulting build; see §6 for the landed evidence.

## 1. Decision and user-visible contract

The report is credible: syntax colors changing across the buffer while typing
are distracting even when input remains fast. This investigation found and
exercised an avoidable whole-viewport color flash. Fix presentation continuity
before changing the palette, adding delays, or replacing the parser.

**Target:** ordinary local edits must not make unrelated, already-highlighted
text flash to the default foreground and back, or move an old color onto the
wrong characters. Text and caret echo remain immediate. Fresh analysis must
still reflect real changes to syntax, including changes with nonlocal effects.

This is a display-analysis/cache correction, not a new highlighting engine.
Keep the existing static grammars, queries, semantic classes, palette and worker.

Distinguish three kinds of color change:

1. **Delivery flicker:** colored text becomes plain while a worker catches up,
   then becomes colored again. Demonstrated below; required fix.
2. **Misplaced interim colors:** old byte spans paint new text without following
   edits. The current source permits this; exact visible instances depend on
   the edit/window. Required geometry and ownership correction.
3. **Actual syntax reinterpretation:** completing a call, opening a string or
   comment, or changing a Markdown fence can legitimately recolor other text.
   [INFERENCE] This may also contribute to the user's experience, but this
   investigation does not establish its contribution. Do not hide correct
   parser output or promise that colors outside the current line never change.

## 2. Evidence from the current implementation

Source inspected: workspace version **0.38.1**. Runtime exercised: installed
`strop --version` reported **0.38.1**. Matching version labels are not proof of
identical build contents; the source diagnosis and runtime observation are
separate evidence.

| Owner | Observed behavior | Consequence |
| --- | --- | --- |
| `crates/strop-engine/src/editor/analysis/mod.rs`: `AnalysisKey`, `document_analysis`, `document_analysis_cached` | Exact keys include revision and viewport byte bounds. The older-frame fallback still requires identical `first` and `last` byte offsets, tab width and search query. | An ordinary insertion/deletion changes the requested window bounds and defeats the intended no-flicker fallback. |
| `crates/strop/src/render/buffer/rows.rs`: `render_pane`; `buffer/content.rs`: `content_spans` | Missing analysis supplies an empty syntax slice; source cells start with the default text foreground. | A pending analysis frame can erase colors across the visible pane, not just the edited token. |
| `analysis/mod.rs`: `clip_stale_frame` | Clones the entire `FrameAnalysis`, drops spans starting beyond EOF and truncates ends. It does not transform spans through edits. Guides/search are carried along unchanged. | EOF clipping is not coordinate correctness. It also copies unrelated analysis data on a paint-time cache miss. |
| `analysis/mod.rs`: `handle_analysis` | Checks pending request identity and document revision before accepting a result; retains bounded cached windows, including older revisions. | Preserve freshness admission. The remedy is not accepting an obsolete completion as current. The “one-frame-stale” comments do not impose a frame/time bound on reuse. |
| `analysis/worker.rs`; `crates/strop-syntax/src/lib.rs` | One display-analysis actor owns native parser state; `Highlighter::apply_edits` edits kept trees, and `ensure_tree` reparses incrementally from rope chunks. | “Use incremental Tree-sitter” is already implemented and would not fix this delivery problem. |
| `editor/transact.rs`: `sync_document_positions`; collection mutation paths | Ordered `Change` records already reach `AnalysisState::edits` before their journal is cleared. | Reuse the mutation gateway and its geometry; do not infer changes by diffing buffer strings. |
| `crates/strop/src/render/mod.rs`: `syntax_style` | A shared class-to-style projection supplies panes and previews. | No competing theme or renderer-local stabilization algorithm is needed. |

The observed flash did not require a running language server: the isolated
runtime reported rust-analyzer unavailable. The inspected color path is
Tree-sitter spans, not an LSP semantic-token layer.

### Actual rendered-frame observation

A throwaway 228-byte Rust file was opened through the real binary's headless
driver at 100×24 cells. HOME and XDG config/state/cache roots were isolated;
PATH excluded the installed language servers. After `settle`, the script entered
Insert at byte zero and inserted **one space**. Full-content tracing recorded
actual rendered cells, including foreground/background/modifiers, rather than
only the text printed by `frame`.

Observed sequence:

- Analysis requests: revision 0, window `[0, 228)` → revision 1, `[0, 229)`.
- Last pre-edit frame: the `fn` in the untouched second function was keyword
  violet `#c58ae8`.
- Immediate post-edit frame: that same `fn` was default text `#e8e4da`.
- Fresh-analysis frame: `fn` returned to `#c58ae8`.
- **98 non-whitespace source cells on unchanged rows 2–11 changed foreground;
  all 98 returned to their original foreground.** Of those, 59 were in the
  untouched second function on rows 7–11. Gutter cells were excluded and only
  identical symbols at identical cells were compared.
- Script exit: 0. Undo was also exercised; no claim is made that every edit or
  undo produces the same visible sequence.
- Full replay of the captured trace with the same installed binary exited 0,
  including its recorded cell checks.

This proves a rendered color transition, not a human-perception duration or a
live-terminal latency measurement. No post-fix result is claimed. Raw capture
and fixture files were temporary; the reproducible inputs are below.

### Reproduction for the implementing agent

Create `typing.rs` with this exact text, including the final newline:

```rust
fn main() {
    let message = "hello";
    let count = 42;
    println!("{} {}", message, count);
}

fn untouched(value: usize) -> usize {
    // This function is not being edited.
    let doubled = value * 2;
    doubled + 1
}
```

Create `typing.keys`:

```text
resize 100 24
settle 5000
frame
keys gg0i
key {"Char":" "}
frame
settle 5000
frame
keys <esc>u
frame
settle 5000
frame
```

Run with fresh isolated HOME/XDG directories and a new trace path:

```sh
strop --headless typing.keys typing.rs --log-file typing.jsonl --log-content
```

Inspect the cell-bearing `replay` / `check` records emitted by
`render/frame_capture.rs::frame_observation`: each row run contains count,
symbol, foreground, background, underline color, modifiers and skip flag.
The explicit `frame` directive can run after the result arrives; inspect the
**automatic frame immediately after the input event** as well. Worker timing is
not a deterministic regression-test seam. The permanent test must control
completion delivery instead of sleeping or requiring a lucky scheduling race.

## 3. Proposed design: edit-mapped syntax presentation

Separate **exact analysis** from **temporary syntax presentation**. Keep one
owner in `strop-engine::editor::analysis`; paint remains a readonly consumer.
This refines presentation only, not the parser or grammar resolver.

Tradeoff: geometry can be current before token meaning is current. A retained
class may briefly reflect the previous parse after an upstream delimiter edit;
it must yield to the next accepted exact result. This is explicit provisional
syntax, never semantic authority.

This plan proposes a precise refinement of 0038's no-stale-result fallback and
0039's no-old-revision injection-span wording: raw old coordinates and stale
worker completions remain forbidden; journal-projected, provenance-bearing
syntax presentation is permitted while current analysis is pending. Amend those
contracts with the implementation rather than claiming that coordinate mapping
makes an old parse current. The pure operator-preview contract is unchanged.

### A. Own provenance and coverage explicitly

Retain the last accepted syntax presentation with:

- document/target and syntax identity;
- the revision actually analyzed;
- the revision its display coordinates have been mapped to;
- covered source intervals and sorted, nonoverlapping syntax spans;
- explicit exact versus projected freshness.

Use the smallest representation that expresses these facts. A projected
presentation is **not** a fresh `FrameAnalysis` and must not mark
`WindowBounds::Complete` or satisfy a consumer asking for current semantic
analysis. Keep analyzed revision distinct from mapped revision: coordinate
mapping does not establish semantic validity.

While awaiting analysis, present edit-mapped syntax on surviving text in covered
regions. New, replaced or otherwise unknown text may use the ordinary foreground.
An overlapping viewport reuses the covered intersection; newly exposed content
may be cold. Changing window bounds, pane width or the active search query must
not erase reusable syntax on the overlap.

Keep the cache bounded. Prefer reusing the existing window-retention owner over
adding an unbounded document-wide history. Bound retained projection data and
work by the displayed content, including extremely long lines; eight windows
alone do not bound their span count. Preserve the existing worker-prepared layout
and horizontal-clipping mechanisms rather than scanning a long line on input.

### B. Transform once through the real journal

Project at the mutation/preparation boundary, not by cloning/rebasing a complete
analysis frame on every paint. Consume ordered `Change`/`InputEdit` records from
**every existing** `AnalysisState::edits` caller, including collection edits,
projection updates, undo/redo, paste and completion acceptance through those
mutation paths.

For a half-open edit `[start, old_end)` replaced by `[start, new_end)`:

- Surviving spans strictly before the edit keep their coordinates.
- Surviving spans after it shift by the byte delta.
- Removed/replaced bytes must never retain a previous token's decoration.
- For intersected spans, retain provably surviving fragments in mapped
  coordinates and exclude the replacement interval. Do not drop an entire
  multiline string/comment span for a small internal edit: that would recreate
  the broad flash. Discard a whole span only when nothing survives or its mapping
  cannot be established. New bytes never inherit a class merely because they
  lie between mapped endpoints.
- For a pure insertion at a span boundary, text starting there moves right;
  a span ending there does not grow across the insertion. Make the affinity
  explicit and test it. Internal insertions must leave the new interval unknown.
- Transform coverage as well as spans, keep holes explicit, discard empty ranges,
  and preserve sorted/nonoverlapping, in-bounds UTF-8 byte geometry.

`strop-core/src/editmap.rs::map_position` is the verified **right-affinity point**
mapper. Reuse it where its contract fits, but do not blindly map both endpoints
of a syntax range with point affinity. It neither proves unchanged token meaning
nor handles the desired span-end insertion policy by itself.

Continue projecting through successive edits before a worker result arrives;
revision-minus-one is not an adequate freshness model for rapid typing. A missing
journal chain, full replacement or identity change invalidates projection for
the affected target/coverage rather than guessing. Source close/reopen and a
language/path change must not inherit another syntax owner's presentation.

Projection must not scan/materialize the document or copy all retained analysis
payloads per keystroke. Prepare/index the bounded visible presentation on the
worker where needed; the interactive update and paint must stay bounded. Measure
this explicitly for long lines and repeated edits, not only small fixtures.

### C. Publish exact results without a blank transition

Preserve cancellation, request ownership, target lifetime and current-revision
checks. A valid fresh result atomically replaces the relevant projected syntax
coverage; even an empty exact syntax result is meaningful and replaces old
colors. Do not clear a good display merely because another request starts.

Ensure the newest visible requests eventually run after cancellation and that
all visible windows converge, including two panes of the same document. Do not
turn this into a new scheduler unless a controlled delivery scenario proves the
existing admission path needs a targeted correction.

Do **not** reuse the entire stale `FrameAnalysis` as the continuity vehicle:
search hits/counts, guide geometry and line layouts have their own correctness
requirements. Keep them current-revision-only unless their existing owner proves
an applicable transformation. Matching-delimiter highlights, operator preview,
selection and diagnostics keep their existing ownership/precedence. Syntax
continuity must never authorize an edit, motion, search result or layout.

Cancellation is not failure. A real analysis failure remains visible through the
existing error path and invalidates failed provisional coverage; it must not
leave indefinitely “settled” old colors or be hidden by a success-shaped cache.

### D. Preserve legitimate syntax changes

Apply successful current-revision parser output even when its tree contains
`ERROR`/missing nodes. Incomplete code is normal while typing; a global
`root.has_error()` freeze can prevent correct colors from returning indefinitely.

Changing a quote/comment/fence can alter scope far beyond the caret, and matching
a fresh parse is the settled correctness oracle. Keep this distinct from the
no-blank-intermediate-frame contract for syntax-preserving edits.

No debounce, Insert-mode freeze, “update on Esc/save”, new timer, RGB transition,
new configuration knob or palette redesign is authorized by this plan. Those
would mask the demonstrated bug or add latency without fixing its source.
If significant noise remains after the delivery correction, record a separate
before/after case distinguishing parser recovery from palette contrast before
proposing a further policy. Do not silently add error-recovery heuristics here.

## 4. Implementation sequence and ownership map

### S1 — Establish deterministic regressions

Exercise the reproduction on the candidate binary, then turn its color-continuity
contract into a controlled pending-result renderer regression. Keep initial load,
edit-with-result-held and accepted-result frames separate. The test must fail on
the current behavior and assert exact unaffected cell styles, not merely that
some spans exist.

Delete the inadequate
`analysis/mod.rs::tests::an_edit_serves_the_previous_frame_instead_of_blanking`
test when replacing it: it manufactures a `[0, 0)` window containing a `[0, 2)`
span, requests the same bounds after edits and checks `is_some()`. It does not
exercise the real changing viewport or prove correct colors. Do not re-pin its
incidental clipped-stale behavior.

### S2 — Implement one presentation owner

Add the explicit projection/coverage model inside `editor/analysis`, feed it
through the existing journal boundary, and retain strict exact-result admission.
Split by concern as needed (for example `presentation.rs`, geometry logic and
focused tests). `analysis/mod.rs` is already about 650 lines; do not grow it past
the repository's source/test size ceiling.

Relevant journal callers found through Rust references:

- `crates/strop-engine/src/editor/transact.rs`;
- `crates/strop-engine/src/editor/collections/editing.rs`;
- `crates/strop-engine/src/editor/collections/projection.rs`;
- `crates/strop-engine/src/editor/collections/view_positions.rs`.

Keep `analysis/worker.rs` and `strop-syntax::{lib,injections,spans}.rs` responsible
for exact parsing/query/injection output. Change them only as required to deliver
bounded presentation data; do not introduce a parallel lexer.

### S3 — Cut every presentation consumer over

Refresh language-server references before exported API changes. Migrate both
admission and cached queries together; do not leave two fallback algorithms.
Known consumers to cover:

- `editor/prepare.rs`: normal panes, collection source windows, buffer-backed
  picker previews and pending/completeness reporting;
- `crates/strop/src/render/buffer/rows.rs`: ordinary panes and translated
  collection-source spans;
- `crates/strop/src/render/picker/preview.rs`: buffer-backed preview syntax and
  the separate immutable cached-resource preview path;
- `analysis/mod.rs` test-support fixtures, `editor/contract_probes.rs`, and
  `editor/tests/keybinds_tests.rs`: exact-analysis callers must explicitly await
  exact results, not accidentally accept a projected frame as a fresh parse.

Keep `render/mod.rs::syntax_style` as the shared palette projection. Remove
`clip_stale_frame`, duplicated stale lookup branches and obsolete “one frame
behind” comments after migration. Do not carry compatibility shims.

### S4 — Prove continuity, convergence and responsiveness

Run the acceptance matrix below, the real-binary frame capture and a live PTY
editing session. Record before/after results here. Only then update CHANGELOG and
0028 with the delivered behavior; this handover is not release evidence.

## 5. Acceptance matrix

| Scenario | Required observable result |
| --- | --- |
| Warm Rust file; insert/delete whitespace or extend an identifier; worker result held | Unaffected source text retains its syntax foreground/emphasis on every interim frame; no whole-pane default-color flash. Changed text/caret appears immediately. |
| Insert/delete before colored tokens; newline split/join; UTF-8 including multibyte characters | Surviving colors follow the same text, never adjacent bytes; intervals remain ordered, nonempty, in bounds and at valid boundaries. Check both edit edges. |
| Several edits, paste, undo/redo and completion acceptance before delivery | Every journal edit is accounted for in order; no cumulative drift; current exact syntax eventually replaces projection. |
| Same-length replacement with different syntax | No false cache hit from equal length/window; affected old styles are removed and settled spans equal a fresh parse. |
| Open/close multiline comments, strings and Markdown fences; TSX/HTML injections | Genuine scope changes are allowed; final classes/emphasis match a fresh parse, including injected regions. No permanent last-good-tree freeze. |
| Scroll, resize, horizontal scroll on a long line, two panes at different offsets | Covered overlap stays colored; cold content is not given fabricated spans; both panes converge without starvation or recoloring a different window. |
| Edit a collection excerpt while its source is visible; buffer-backed picker preview | Source-to-view translation preserves the same syntax policy and live text; cached previews never borrow another target's presentation. |
| Search/selection/operator preview/matching delimiters during typing | Syntax continuity leaves overlay precedence unchanged; stale search coordinates or counts do not become current merely because syntax was projected. |
| Delayed, cancelled, out-of-order or failed work; document close/reopen; syntax identity change | Wrong-owner results never paint; identity changes reset relevant presentation; real failure remains visible; the final current result is not lost. |
| Empty exact result, empty buffer, whole-buffer replacement | Old colors disappear when they should; no phantom tail spans or indefinite provisional state. |

Use deterministic journal/ownership tests plus **styled TestBackend cell-grid
snapshots across transitions**, not text-only snapshots or wall-clock sleeps.
Retain the incremental-versus-fresh parser oracle; add cases at boundaries not
already covered rather than duplicating static language-query tests. A settled
final screenshot alone cannot establish that intermediate flicker is gone.

Runtime qualification:

1. Repeat the real-binary reproduction and inspect the automatic post-key cells.
   For its unchanged rows, the foreground-change count must be **zero** between
   the warm, pending and fresh frames (the baseline is 98).
2. Type at ordinary and sustained rates in an actual terminal using Rust,
   TypeScript/TSX, Python and Markdown, with unfinished delimiters and undo/redo.
   Inspect styles as well as responsiveness. Keep genuine reclassification
   separate from delivery flicker in the evidence.
3. Exercise a large document and a pathological long line. Record input-to-echo
   and preparation/paint cost before/after using existing tracing/bench seams,
   plus retained presentation size. No waiting for analysis, per-key full-rope
   copy/scan, paint-time parser work, process spawn or new unbounded history.
4. Run the repository gate:
   `docker compose run --build --rm test` (fmt, locked workspace clippy with
   warnings denied, locked tests). Run affected protocol/replay or verification
   gates if their contracts change; do not claim unrelated assurance as proof.

**Done means:** the demonstrated color flash is eliminated, interim colors have
correct edit-mapped geometry and explicit provenance, all affected surfaces
converge to exact current analysis, and the editor retains its nonblocking input
path. (The handover phase itself changed no syntax implementation — §6
records the executing session's work.)

## 6. Landed implementation (2026-10-04)

Delivered exactly the §3 design, in three moves:

- **`strop-engine/src/editor/analysis/presentation.rs`** (new): the
  edit-mapped presentation owner — sorted disjoint spans, coalesced
  coverage, line-indexed rails, explicit provenance, bounded by
  `MAX_SPANS`/`MAX_COVERAGE_INTERVALS`/`MAX_RAIL_LINES` caps that reset to
  the freshly accepted window. Pure journal arithmetic (`fragments`,
  `project_ranges`, `project_rails`, `replace_window`, `cover`,
  `uncover`), a memoized interim `FrameAnalysis` (interior-mutable — the
  paint path stays readonly), and unit tests for every §5 geometry case
  including boundary-insertion affinity, straddled multiline tokens,
  revision-gap invalidation, cap resets, rail shifts and clip/tab guards.
- **`analysis/mod.rs`**: `AnalysisState` carries one presentation per
  document target; `edits()` folds the same journal (all four mutation
  callers) through it; `handle_analysis` publishes exact results into it
  (`accept`) and invalidates failed windows (`fail`); `document_analysis`
  and `document_analysis_cached` serve exact-hit-else-interim;
  `clip_stale_frame` and both stale-lookup branches are deleted;
  `analysis_fixture*` awaits the exact cache explicitly; the inadequate
  `[0,0)`-window test is replaced by three real ones (edit-mapped interim
  with held completion, scrolled-overlap, failure uncovers).
- **`prepare.rs`**: declared pane bounds probe the exact cache only
  (`exact_analysis_cached`), so an interim frame stays truthfully
  `WindowBounds::Loading` — satisfying this plan's own "must not mark
  Complete" clause.
- **`strop/src/render/buffer/tests/syntax_stability.rs`** (new): styled
  TestBackend cell-grid regressions — the §5 typing scenario (one space at
  byte zero; warm → held-interim → settled frames; zero foreground
  changes on unrelated rows, keyword color asserted on the untouched
  function and the shifted `fn`) and the scroll scenario (shared gutter
  line numbers paired across frames, covered overlap stays colored).

Search summaries, hit counts and line layouts remain current-revision-only
as §3C requires; preview surfaces are covered through the same document
query, and the immutable cached-preview path is untouched.

### Verification (this session)

- Reproduced §2's capture on the new build (debug and release): the
  98-cell metric reads **0** for both pre-edit→interim and
  pre-edit→settled; the untouched `fn` keeps `#c58ae8` throughout; the
  post-insert frame keeps indent rails (they re-seat one frame later when
  the inserted space re-indents the scope opener — the documented
  provisional tradeoff); undo restores 0 changes. Full trace replay of the
  capture exits 0 including cell checks.
- Render cost, same headless scenario on the same host (median/max µs,
  42 renders): 830 KB / 30 k-line file — release build 408/714 vs the
  shipped 0.38.1 release's 1046/3971; 73 KB single-line file — 343/611 vs
  506/1301. Analysis viewport requests identical (23) on the long-line
  file and fewer on the large file; input-to-echo stays immediate and no
  path waits on analysis.
- Live PTY session on the release build: typing in Insert mode, rails
  visible, clean exit — the real TUI loop drives the same engine.
- Suites: `strop-engine --lib` 773 passed, `strop-editor` bin tests 121
  passed (including the two new styled regressions), `strop-syntax` 35
  passed, `cargo fmt --check` clean, `cargo clippy --locked
  --workspace --all-targets -- -D warnings` clean. The Docker test gate
  (`docker compose run --build --rm test`) ran green before the release
  tag; see the changelog entry for the shipped version.

### Release follow-up (same version)

The 0.39.0 release workflow's completion qualification failed twice with
"dismissal retained obsolete query authority". Reproduced locally (2/30
 stress runs on this change's build, 0/30 on the 0.38.1 control), then
diagnosed to a `--ui-stdio` protocol defect this change's shifted
publish timing exposed: `serve.rs` sent the action acknowledgement
BEFORE publishing the action's view, and a state-only action (completion
dismissal — no revision change, so the view generation does not move)
could leave the pre-action state as the client's last word at the ack.
The publication now precedes both the action and viewport acks, with a
pipe-level regression (`acknowledged_state_only_actions_publish_before_
the_ack`) that fails on the old ordering and passes on the new one. The
separate TLAPS lane failure did not reproduce on the same tree locally
(`docker compose run --build --rm tlaps` green; specs untouched) —
prover-backend flakiness on the loaded runner, re-run.
