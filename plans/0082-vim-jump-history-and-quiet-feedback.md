# 0082 — Vim-native jump history and quiet feedback

Status: **landed in 0.49.0** (2026-11-05).

## 1. Decisions

### Jump history is not undo/redo

Strop records the **origin immediately before a successful jump**, not every cursor
motion and not every destination as it lands. A destination becomes retained history
when a later jump leaves it. `Ctrl-O` walks to older origins and `Ctrl-I` walks back
toward newer ones; `Space j` exposes the same records plus a synthetic current-position
row. Ordinary motions and edits are not jumps, although edits remap retained records.

The canonical contract is Vim/Neovim `:help jumplist`:

- History is per window, copied when a window splits, then independent.
- At most 100 entries; a new entry removes an earlier entry on the same document line.
- The default is **not** a stack. A new jump while back in history retains newer
  entries. Discarding them is optional `jumpoptions=stack` behavior; Neovim defaults to
  `jumpoptions=clean`, not `stack`.
- Only the first `Ctrl-O` from below the newest entry adds the current position so
  `Ctrl-I` can return. Traversal itself must not invent entries after small motions.
- Existing full selection/viewport restoration is strop's deliberate equivalent of
  `jumpoptions=view`; retain it.

The previous global past/future stacks incorrectly truncated the future, injected live
positions during every traversal, mixed panes, had no bound, and deduplicated only an
exact newest offset. These are fidelity defects, not an alternative design.

### No fading notification popups

Do not add LazyVim/nvim-notify-style floating or fading toasts. Strop 0001 calls for one
slim statusline and no latency theater; 0081 §8 explicitly chooses durable actions over
more popups. The existing division is correct:

- ephemeral confirmations and immediate refusals stay in `Editor.message`;
- meaningful failures/unresolved decisions stay in owner-backed `FeedbackStore` and
  project through the modeline, `Space a`, and `:explain`;
- progress replaces in place; diagnostics and pickers retain their existing surfaces.

A second timed card owner would cover source, add animation/replay policy, and duplicate
presentation state without making recovery more durable. No popup configuration,
animation, queue, history, or input owner is added.

The actionable notification defect is narrower: ultra-narrow modelines retain an LSP
problem badge but can drop the durable feedback cue entirely. Preserve a compact textual
warning/failure marker through every width degradation, including the bare layout. This
is another projection of `presented_feedback()`, not new notification state.

## 2. Jump-list implementation

### One indexed history per pane

Move jump history into `Pane` as a `JumpList { entries, position }`. `position` is the
currently selected entry, or `None` below the newest entry. Keep the invariant
`position.is_none_or(|index| index < entries.len())`; splitting clones it with the rest
of the pane view. Remove the editor-global past/future fields and all stack swapping.

- A successful new jump captures the current full `JumpRecord`, removes same-line
  duplicates, appends the origin, caps the list at 100, and sets `position = None`.
  It does not clear newer entries. Initial file windows seed the same line-1 file
  origin Neovim exposes; scratch windows start empty.
- First back traversal from `None` snapshots the current position at the end only when
  older history exists, then selects the preceding entry. Subsequent back/forward
  traversal changes only the index and restores stored records; it never captures the
  live position.
- Dead documents are skipped without corrupting the index. Empty boundaries retain the
  existing `no jumps` / `at newest jump` messages.
- Picker past/current/future rows derive from the indexed list; acceptance remains a new
  jump through the same recording path.
- Edit mapping visits every pane's records plus the existing return/search/directory
  records. Replay seeds include pane histories and advance the semantic seed version.

### Record every supported successful Vim jump

Add missing recording for `{`/`}`, `H`/`M`/`L`, successful `:substitute`, buffer-picker
acceptance, `Ctrl-^`, and terminal launch. Validate mark existence/document liveness
before recording so failed marks leave history untouched. Retain existing recording for
`gg`/`G`, `%`, search/repeat, marks, `:N`, LSP/search/symbol/file dives, and collections.
No new sentence/section/tag motion is introduced by this plan. Fidelity work exposed by
the new differential journeys is included: `}` must not enter the trailing newline's
phantom line, `H`/`M`/`L` reveal the current caret and choose the upper middle line
for even-height views, and successful substitution lands on the last affected line.

### Persist the restored window's history

Extend the existing per-project session schema with the active window's indexed history.
Serialize file-backed records by path and remap them to fresh `DocumentId`s after all
session buffers open; restore offsets, selection, viewport, horizontal origin, and the
index with clamping. Skip records that have no persisted document and adjust the index
without reviving dead/scratch/read-only documents. Keep the existing local session and
undo trust/validation rules; do not add remote authority or split-layout persistence.
Older sessions without a `jumps` field load with empty history. Quit-all captures one
complete snapshot before closing any buffer; the last close must not overwrite it with
a subset containing only the final document.

## 3. Quiet feedback implementation

Gather the active `FeedbackRecord`'s severity once in `Modeline`. Under width pressure,
retain the shortest useful textual `!` warning/failure marker and severity color even
when transient prose and optional services disappear. The full summary/action cue keeps
its existing precedence at widths that can show it. No behavior changes in feedback
storage, dismissal, actions, `:explain`, terminal input, or timing.

## 4. Verification

- Unit/engine behavior: origin population, default future preservation, small motions
  during traversal, counts, same-line deduplication, 100-cap, failed-mark atomicity,
  per-pane isolation and split copying, closed-document traversal, and view remapping.
- Population regressions for paragraphs, `H/M/L`, successful/failed substitution,
  buffer switching, and terminal return journeys.
- Session round-trip with cross-document records and index restoration; old-session
  compatibility and changed-file clamping.
- Differential journeys against `nvim --clean` for back/forward, counted traversal,
  branching without stack truncation, paragraphs, `H/M/L`, substitution, and same-line
  deduplication. Compare observable cursor/text/register outcomes; do not fabricate a
  jumplist state dump that Neovim does not expose through the existing harness.
- Golden modeline grids proving warning/failure markers survive narrow and bare layouts,
  ordinary messages do not acquire durable markers, and existing terminal/LSP precedence
  remains intact.
- Real headless and interactive TUI smoke for jump traversal, session restoration and
  narrow feedback. Then run `cargo fmt --check`, strict locked Clippy, the locked
  workspace tests through `docker compose run --build --rm test`, release fixtures,
  changed-feature smoke, static artifact checks and freeze verification.

## 5. Documentation and release

Update `docs/vim-compat.md`, `docs/editor.md`, `docs/search.md` where its jump-history
description needs clarification, this plan's landed evidence, and `CHANGELOG.md`.
Release as 0.49.0 because indexed per-window history and session restoration change the
public session behavior in addition to fixing grammar fidelity.

Follow 0079 exactly: synchronize workspace/inter-crate versions and `Cargo.lock`, commit
and push the qualified source, tag `v0.49.0`, build/package the exact musl artifact once,
publish its draft assets publicly, verify latest catalog/public installation, vendor and
push the website sources, dispatch rebuild, and fetch the rendered site. The release is
not done until GitHub, installation, website, branch and tag all prove 0.49.0.

## 6. Landed evidence

- `docker compose run --build --rm test` passed formatting, strict locked Clippy and the
  locked workspace/SSH suite, including 949 engine tests, 168 binary tests and the real
  Neovim differential harness. Review-found dead-tail, no-op motion, same-line, picker,
  viewport and substitution regressions are included.
- The actual 0.49.0 headless binary returned line 1 after EOF-paragraph back traversal,
  line 9 after counted visible-line traversal, line 1 after substitution traversal, and
  retained the feedback `!` in an 8-cell frame.
- A real PTY created cross-file history and quit all. The persisted session contained
  both buffers and all jump records; after restart, the editor's state trace proved
  `Ctrl-O` switched to the other document and `Ctrl-I` returned to the original line 3.
- `python3 verification/check.py --release`, `sh tests/release-catalog.sh` and
  `sh tests/install.sh` passed: 28 boundaries, 165 claims and all evidence pins live.
  `local-fs-authority`, `worker-serve` and `worker-deployment` were force-stamped only
  for the reviewed manifest-version bump; `devcontainer-lifecycle` and
  `completion-lifecycle` were force-stamped because their behavior is unchanged by the
  shared terminal/replay files. Behavioral rows carry new named evidence instead.
- Modeline fitting moved to `render/statusline/layout.rs`; the main module is 538 lines
  and no over-ceiling follow-up is owed. Deep model/proof lanes were not run and are not
  claimed by this routine release qualification.
