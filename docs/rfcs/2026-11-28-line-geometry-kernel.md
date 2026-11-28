---
rfc: "2026-11-28-line-geometry-kernel"
title: "Prove line geometry: selection spans, change regions and line maps across a diff"
status: "accepted"
implementation: "in-progress"
authors: ["Fable"]
created: "2026-11-28"
updated: "2026-11-28"
issue: "https://github.com/stropdev/strop/issues/286"
milestone: "v0.78.0"
supersedes: []
superseded_by: []
blocked_on: ["Refuse a permalink or line-history range that contains an uncommitted change, or map its two ends?"]
---

# RFC: Prove line geometry: selection spans, change regions and line maps across a diff

## Problem and observable outcome

Tracking issue: [#286](https://github.com/stropdev/strop/issues/286).

Every git feature that names a line does its own arithmetic over rows, and none of that arithmetic is in the kernel. Reviewing v0.77.0 (`07732b1`) found eight user-visible defects that are all the same missing decision: *which line of another version is this line?*

| What the user does | What happens today | Where |
|---|---|---|
| `V` over lines, `Space g y` | "command unavailable in visual mode"; the Visual branch of the builder is unreachable through keys | [`visual/mod.rs`](../../crates/strop-engine/src/editor/visual/mod.rs) `dispatch_visual_action`, [`permalink.rs`](../../crates/strop-engine/src/editor/permalink.rs) |
| Insert three lines at the top, go to the last line, `Space g y` | `#L5` on the HEAD commit, where the file has two lines | `permalink.rs` builds `(first + 1, last + 1)` from buffer rows and pins `head_sha` |
| Same buffer, `V`, `Space g h` | `git log -L5,5:f.rs` against HEAD: "file f.rs has only 2 lines", or silently the history of other lines | `visual/mod.rs` passes buffer rows to [`log.rs`](../../crates/strop-git/src/log.rs) |
| Hunk preview (`Space g p`), `Space g y` on a context row | worktree line number with the HEAD sha | `permalink.rs` takes `new_lineno` for every diff surface |
| Stage a hunk, insert lines above it, `Space g u` on the staged line | "no staged hunk here"; the staged mark sits on an unrelated row | [`git.rs`](../../crates/strop-engine/src/editor/git.rs) looks a buffer row up in the HEAD→index hunk set |
| Status view, `Enter` on a staged or deleted diff row | lands on the wrong row | [`git_work/keys.rs`](../../crates/strop-engine/src/editor/git_work/keys.rs) uses an index or old-side number as a buffer row |
| Delete two lines, `Space g X` | text restored, cursor three rows low | [`diff.rs`](../../crates/strop-git/src/diff.rs) `changed_region` reports the end of the hunk for a pure deletion |

The first row is a dispatch defect and is fixed on its own ([#289](https://github.com/stropdev/strop/issues/289)). The rest need two pure decisions that do not exist yet:

1. **The change regions of a diff.** Today three functions walk a hunk body independently: `push_run`, `Hunk::signs` and `Hunk::changed_region`. One of them is wrong at a boundary (last row of the table).
2. **A line's position on the other side of a diff**, and whether a *range* of lines exists unchanged on the other side.

The outcome: a permalink, a line history, a staged mark and a status jump name the line the user is looking at, in the version the command talks about, or refuse and say why. Each of those is decided by a kernel function with a theorem about the shipped Rust.

Non-goals: forge-specific URL shapes, the "HEAD is not pushed" notice, Visual line-range staging (deferred by [the git RFC](2026-11-11-git.md) and [#89](https://github.com/stropdev/strop/issues/89)), and the conditional claims already listed in [#282](https://github.com/stropdev/strop/issues/282).

## Proof and test obligations

Stated before the design, per the registry rules. New module `crates/strop-kernel/src/lines.rs`: plain safe Rust, explicit loops over slices, no closures, no iterator adaptors. All arithmetic is compare-before-subtract or `saturating_add`, so every function except `runs` is total in the Aeneas model and [#282](https://github.com/stropdev/strop/issues/282)'s classification lists it as total. `runs` appends to a `Vec`, which Aeneas models as failing at `usize::MAX` elements, so it is listed as conditional (as-built note below).

### The functions

```rust
/// One change run between two versions of a file, in 0-based lines: old
/// lines `old..old + old_len` became new lines `new..new + new_len`. A pure
/// insertion has `old_len == 0`, a pure deletion `new_len == 0`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Region { pub old: usize, pub old_len: usize, pub new: usize, pub new_len: usize }

/// A unified hunk body line, by which sides it exists on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Origin { Context, Addition, Deletion }

/// The ordered inclusive rows a selection covers, whichever end is the anchor.
pub fn span(anchor: usize, head: usize) -> (usize, usize);

/// Append the change runs of one hunk body whose first line is old line
/// `old_start` and new line `new_start` (0-based): one region per maximal
/// stretch of non-context lines.
pub fn runs(old_start: usize, new_start: usize, body: &[Origin], out: &mut Vec<Region>);

/// Whether `regions` describes a diff: each region starts at or after the
/// previous one's end on both sides, at the same distance on both, and no
/// region is empty on both sides.
pub fn coherent(regions: &[Region]) -> bool;

/// Where new-side line `line` is on the old side; `None` when a region
/// added or replaced it.
pub fn old_line(line: usize, regions: &[Region]) -> Option<usize>;

/// Where old-side line `line` is on the new side; `None` when a region
/// deleted or replaced it.
pub fn new_line(line: usize, regions: &[Region]) -> Option<usize>;

/// The old-side lines of new-side lines `first..=last` when all of them are
/// kept and no deletion falls between them; `None` otherwise.
pub fn old_span(first: usize, last: usize, regions: &[Region]) -> Option<(usize, usize)>;
```

`old_line`, `new_line` and `old_span` are one loop each: walk the regions keeping `(old_base, new_base)`, the first line after the previous region on each side; stop at the first region that starts past the queried line; refuse when the line lies inside a region's lines on the queried side; otherwise answer `other_base + (line - this_base)`.

### The model

```lean
/-- A region as plain numbers: (old, old_len, new, new_len). -/
abbrev Quad := Nat × Nat × Nat × Nat
def quad (r : lines.Region) : Quad := (r.old.val, r.old_len.val, r.new.val, r.new_len.val)

/-- New line `n` is a kept line and was old line `o`: `n` lies in a gap
between regions and `o` sits at the same offset in the matching old gap.
`ob`/`nb` are the first lines after the regions already passed. -/
def Kept (ob nb : Nat) : List Quad → Nat → Nat → Prop
  | [], o, n => nb ≤ n ∧ o = ob + (n - nb)
  | (ro, rol, rn, rnl) :: rest, o, n =>
      (nb ≤ n ∧ n < rn ∧ o = ob + (n - nb)) ∨ Kept (ro + rol) (rn + rnl) rest o n

/-- The diffs `coherent` accepts. -/
def Coherent (ob nb : Nat) : List Quad → Prop
  | [] => True
  | (ro, rol, rn, rnl) :: rest =>
      ob ≤ ro ∧ nb ≤ rn ∧ ro - ob = rn - nb ∧ 0 < rol + rnl ∧
      Coherent (ro + rol) (rn + rnl) rest

/-- The regions a hunk body denotes, read off the body directly: one region
per maximal stretch of non-context lines; its lengths count that stretch's
deletions and additions; its starts are `os`/`ns` plus the lines before it
on each side (context and deletions on the old side, context and additions
on the new side). -/
def RunsOf (os ns : Nat) (body : List lines.Origin) : List Quad
```

### The claims

All are added to `claims` in `proofs/Audit.lean`. Bridges end in `_spec`; the others are properties.

| Claim | Statement over the extracted Rust | Rust mutant that must break it |
|---|---|---|
| `span_spec` | `lines.span a h ⦃ r => r.1.val = min a.val h.val ∧ r.2.val = max a.val h.val ⦄` | `lines-span-keeps-direction`: return `(anchor, head)` in both arms |
| `runs_spec` | for `out.val = []`: `lines.runs os ns body out ⦃ r => r.val.map quad = RunsOf os.val ns.val body.val ⦄` (no hypothesis beyond room for the pushes, as in `project_spec`) | `lines-runs-merge-across-context`: do not close the run on a context line. `lines-runs-deletion-at-hunk-end`: take a region's `new` from the counter at the *end* of the run (today's `changed_region` defect, kept as a mutant) |
| `runs_coherent` | `Coherent os ns (RunsOf os ns body)` for every body: what `runs` produces for one hunk is always a diff | paired, see below |
| `coherent_spec` | `lines.coherent rs ⦃ b => b = true ↔ Coherent 0 0 (rs.val.map quad) ⦄` | `lines-coherent-accepts-skew`: drop the equal-distance test |
| `old_line_spec` | with `qs := rs.val.map quad` and `Coherent 0 0 qs`: `lines.old_line n rs ⦃ r => ∀ o : Nat, (∃ u, r = some u ∧ u.val = o) ↔ Kept 0 0 qs o n.val ⦄` | `lines-old-line-admits-added`: skip the "inside this region's new lines" refusal. `lines-old-line-forgets-deletions`: advance `old_base` to `r.old` instead of `r.old + r.old_len` |
| `new_line_spec` | the mirror: `lines.new_line o rs ⦃ r => ∀ n : Nat, (∃ u, r = some u ∧ u.val = n) ↔ Kept 0 0 qs o.val n ⦄` | `lines-new-line-admits-deleted`, `lines-new-line-forgets-additions` |
| `old_span_spec` | under `Coherent 0 0 qs`: `lines.old_span f l rs ⦃ r => (∀ a b, r = some (a, b) → b.val - a.val = l.val - f.val ∧ ∀ i, f.val ≤ i → i ≤ l.val → Kept 0 0 qs (a.val + (i - f.val)) i) ∧ (r = none → l.val < f.val ∨ ¬ ∃ a, ∀ i, f.val ≤ i → i ≤ l.val → Kept 0 0 qs (a + (i - f.val)) i) ⦄`: a range is answered exactly when every selected line is kept and they are consecutive on the old side | `lines-old-span-straddles-region`: drop the `first < r.new` refusal, so a range across a replaced block maps its two ends |
| `kept_respects_regions` | under `Coherent ob nb qs`: `Kept ob nb qs o n → ∀ (ro, rol, rn, rnl) ∈ qs, ¬ (ro ≤ o ∧ o < ro + rol) ∧ ¬ (rn ≤ n ∧ n < rn + rnl)`, and `Kept ob nb qs o n → Kept ob nb qs o' n' → (n < n' ↔ o < o')`: a kept line is never a deleted or an added line, and the pairing preserves order | paired, see below |
| `lines_total` | folded into the bridges as built: `span_spec`, `coherent_spec`, `old_line_spec`, `new_line_spec` and `old_span_spec` have no hypothesis (the coherence premise sits inside the postcondition), so the audit prints each as `TOTAL` | `lines-old-line-raw-add`: replace a `saturating_add` by `+` (breaks `old_line_loop_spec`) |

**Paired mutants** (Rust and model patched together, so the bridge still proves and the property must fail):

- ~~`lines-kept-forgets-deletions`~~ — not buildable: `new_line` stops on the old side, so no patch of `Kept` matches all three patched functions and `new_line_spec` breaks with it. Replaced as built by `lines-coherent-forgets-old-order` — `coherent` and `Coherent` both drop the old-side order test; a region may then start inside an earlier deletion, a kept line can be a deleted one, and `kept_respects_regions`' lemmas (`kept_old_ge`, `coherent_starts`) fail. The coherence bridge and the `runs_coherent` construction follow the old predicate and break with it.
- `lines-runs-new-start-at-run-end` — `runs` takes a region's `new` from the counter at the end of the run, and `RunsOf` is patched alike. `runs_spec` still proves; `runs_coherent` fails, because the region now starts `new_len` lines further from the previous one on the new side than on the old side.

### Outside the proof

- **That libgit2's hunks describe the two texts.** Assumed, and tested: a property test in `strop-git` generates two random line lists, diffs them with the production diff, builds regions with `runs`, and checks `old[old_line(n)] == new[n]` for every kept `n`, plus `coherent`.
- **That the region list is the right one for the command.** Tested at engine level, one test per row of the table above, driven through keys.
- **Neovim** has no say here: none of this is Vim grammar.
- **Row vs line.** `Buffer::line_of` and the 1-based conversion stay in the engine; the kernel sees 0-based lines only. The `+ 1` appears once, where a URL fragment or a `-L` argument is written.

## Decision and alternatives

### Ownership and data flow

1. `strop-git` gains a path dependency on `strop-kernel`. `Hunk` gets `fn regions(&self, out: &mut Vec<Region>)`, which maps its typed `LineOrigin`s to `lines::Origin` and calls `lines::runs`. The header conversion happens once, there: a side with a non-zero count starts at `start - 1`; a side with a zero count names the line *before* the gap, so its 0-based position is `start` itself (`@@ -0,0 +1,3 @@` is position 0). `push_run`, `Hunk::signs` and `Hunk::changed_region` are deleted; signs are derived from regions (`new_len > 0 && old_len == 0` → `Add`, both → `Change`, `new_len == 0` → `DeleteAfter` at `new`).
2. `HunkSet::new` (worker side, [`hunk_set.rs`](../../crates/strop-engine/src/editor/git_memory/hunk_set.rs)) concatenates the regions of its hunks and checks `lines::coherent` once. An incoherent list is kept as "no line map" and `debug_assert`ed: the gutter still paints, every mapping consumer refuses.
3. `HunkData` already holds both edges for the current buffer revision: `unstaged` (index → buffer) and `staged` (HEAD → index). Consumers compose:
   - buffer row → HEAD row: `old_span` through `unstaged`, then through `staged` (permalink, line history, hunk-preview permalink);
   - index row → buffer row: `new_line` through `unstaged` (staged marks, `Space g u`, status `Enter` on a staged row);
   - a deleted row's position: the region's `new` (status `Enter` on a deletion, cursor after `Space g X`).
4. `selected_lines` in `visual/verbs.rs` and the duplicate min/max in `permalink.rs` both become `lines::span`.

The hunk snapshot is keyed by buffer revision, so when it is present it describes exactly the text on screen. When it is still in flight the permalink waits for it the way it already waits for an SSH alias (`PendingPermalink`): nothing is copied until the snapshot for *this* revision lands, and an edit in between drops the request with a message. Nothing on the input path reads git.

### Refusals

`old_span` answering `None` is a visible refusal that names the reason: "lines 4–9 are not in HEAD (uncommitted changes in the selection)". A file with no HEAD version (untracked, or renamed in the worktree) refuses with "file is not in HEAD". No guessed line ever reaches a URL.

### Alternatives

- **Do nothing / test more.** The defects are boundary errors in three hand-written walks. Tests found one of them only now, after two releases of gutter work.
- **Map the two ends only.** `old_line(first)` and `old_line(last)` would let a range across a replaced block through; the link would show different text from the selection without saying so. `old_span` refuses instead. This is an owner taste question (see the tracking issue): the lenient rule is one line of engine code over the same proved functions.
- **Ask git** (`git blame -L` reverse mapping, `git diff -U0` parsing per request). A process per keystroke-level action, and a second source of truth beside the hunks already on screen.
- **Keep `changed_region` and fix it.** It would remain a third walk of the same body.

## Compatibility and delivery

No key, file format, session or trace changes. `HunkSet` serialization keeps `(hunks, rows)`; regions are rebuilt on load. Replay `SEMANTIC_VERSION` moves once because permalink and line-history results change for dirty buffers.

Independently deliverable, in order:

1. [#287](https://github.com/stropdev/strop/issues/287) — `Region`, `Origin`, `runs`, `coherent` with their claims; `strop-git` consumes them; `changed_region`/`signs`/`push_run` deleted; cursor after `Space g X` fixed.
2. [#288](https://github.com/stropdev/strop/issues/288) — `span`, `old_line`, `new_line`, `old_span` with their claims and the paired mutant.
3. Consumers, one issue each: permalink ([#290](https://github.com/stropdev/strop/issues/290)), line history ([#291](https://github.com/stropdev/strop/issues/291)), staged rows ([#293](https://github.com/stropdev/strop/issues/293)), status `Enter` ([#294](https://github.com/stropdev/strop/issues/294)).

Finish line for the kernel slices: `sh proofs/refresh.sh` committed, `docker compose --profile proofs run --rm proofs` green, every new claim broken by a mutant in `proofs/mutants.py`, the audit listing six more public functions as claimed.

Later, not in this RFC's slice: the sign clamp and row lookups of `HunkSet` ([#305](https://github.com/stropdev/strop/issues/305)), the second span rule in `symbols/scope.rs` ([#306](https://github.com/stropdev/strop/issues/306)), and the remaining edit geometry in [#55](https://github.com/stropdev/strop/issues/55).

## Decision log and as-built notes

- 2026-11-28: drafted from the v0.77.0 git review. Awaiting the maintainer's decision; milestone assignment is not acceptance.
- 2026-11-28 (as built, #287/#288, `proofs/Kernel/Lines.lean`): the Lean statements above were never compiled; the proved ones differ where the drafts were false or unprovable.
  - `Coherent` also requires every region to end inside `usize` (`ro + rol ≤ Usize.max`, same for the new side) and `coherent` checks it: without it the saturating bases of the walks differ from the model.
  - `RunsOf os ns body` is `runsFrom (os, 0, ns, 0) body`, a recursion over a pending run: a context line closes the run (`closeRun`, which drops an empty one) and opens an empty one on the next line of both sides.
  - `runs_spec` appends to any `out` (`r.val.map quad = out.val.map quad ++ RunsOf …`) under `os + |body| ≤ Usize.max`, `ns + |body| ≤ Usize.max` and `out.len + |body| < Usize.max`; `runs_coherent` carries the first two. `runs` is listed `CONDITIONAL` in the audit.
  - The line-map bridges put `Coherent 0 0 qs →` inside the postcondition and bound the answer by `Usize.max` (an answer past `usize` is refused): `old_line_spec` is `old_line n rs ⦃ r => Coherent 0 0 qs → ∀ o, (∃ u, r = some u ∧ u.val = o) ↔ Kept 0 0 qs o n.val ∧ o ≤ Usize.max ⦄`, `new_line_spec` its mirror, and `old_span_spec`'s refusal side reads `¬ ∃ a, a + (l - f) ≤ Usize.max ∧ ∀ i …`.
  - `old_span` refuses at the first region the range reaches (`last ≥ r.new`) unless the range starts at or after that region's added lines (`first ≥ r.new + r.new_len`); `lines-old-span-straddles-region` narrows that refusal to `first ≥ r.new`.
  - `lines-runs-deletion-at-hunk-end` breaks `close_spec`, the push bridge `runs_spec` rests on; the paired `lines-runs-new-start-at-run-end` breaks `coherent_closeRun_append` (behind `runs_coherent`) and `close_spec`.
- 2026-11-28 (as built, #290/#291/#293): `editor/git_memory/line_maps.rs` maps 0-based buffer rows to 1-based HEAD lines (`old_span` through `unstaged`, then `staged`); the refusal reads "line N is not in HEAD (uncommitted change)" for one line and "lines A–B are not in HEAD (uncommitted changes in the selection)" for a range. The held request lives in `GitWork` (the `Editor` field ratchet), keyed by document and revision, and is released by the hunk completion. A hunk preview's `HunkOrigin` captures its origin's line maps at the preview's revision, so the preview needs no live snapshot. `open_line_history` maps its 1-based buffer lines before `git log -L`. The staged set is indexed on the worker over buffer rows (`HunkSet::staged`, `new_line` through the unstaged regions); its own regions stay in index lines for the HEAD map. Status `Enter` (#294) is not part of this slice.
