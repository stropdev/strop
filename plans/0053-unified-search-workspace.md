# 0053 — One search workspace, replacement on demand

Status: implemented for 0.30.0, following the published and fully verified
0.29.0/0051 release. S01–S10 implementation and acceptance evidence are in §11.

## 1. Decision and scope

**Grep and project replacement are one task with an optional replacement field,
not two pickers.** Keep the large workspace the user likes in Replace. Toggling
replacement changes presentation and edit intent in place; it never throws away
the query or starts another search merely to change modes.

This follows [0051](0051-whole-editor-polish-and-query-language.md), especially
R01–R04, R06, R07 and R10. Its parser, suggestions, hidden/ignored controls,
source authority, exact replacement planning, review/apply/save separation and
whole-editor quality requirements remain binding. This plan changes the search
family's interaction and presentation, not those guarantees. It does not delay
0051 or replace its acceptance ledger with a smaller release.

[0054](0054-unified-filesystem-workspace.md) supplies directory browsing and file
operations. Files remains a quick recursive finder; Search remains a content
workspace; a collection remains the durable, editable source-backed working set.
Unifying search does not mean turning all three into one overloaded popup.

### Required delivery ledger

All S requirements are part of this change. Implementing the toggle while leaving
two renderers, destructive Enter behavior or session loss is not completion.

| ID | Required result |
| --- | --- |
| S01 | One search model and entry path; delete the separate Replace kind and its duplicate presentation/dispatch branches. |
| S02 | In-place replacement toggle preserving query, draft, results, selection, viewport, exclusions and scope; no search relaunch on a presentation-only change. |
| S03 | One large, stable, responsive card in both modes, with deliberate file identity, source line information and surface backgrounds. |
| S04 | Explicit keyboard ownership, truthful contextual hints and query suggestions; ordinary document Vim bindings remain unchanged. |
| S05 | One included workset shared by collection and replacement, with visible exclusions in either mode. |
| S06 | Exact replacement preview and explicit prepared review; Apply changes buffers, Save persists them according to 0051. |
| S07 | Source opening and search re-entry retain the investigation instead of rebuilding an empty picker. |
| S08 | Typed namespace/root scope, independent query/draft/review lifetimes and bounded owned workers/caches. |
| S09 | Shared row/preview components, clean callsite migration, no oversized mixed-responsibility modules. |
| S10 | Real-surface evidence, behavioral regressions for state/safety boundaries, help/docs/changelog and the integrated gate. |

An implementation owner records paths, exercised behavior and limitations against
every S ID. Removing an S requirement needs explicit user approval and a roadmap
entry, not an assertion that the rest is sufficient for an initial version.

## 2. Evidence, with the baseline kept honest

The installed/frozen executable used for the walkthrough was **strop 0.28.0**:
SHA-256 `db347d0e49b6754970dc6b5d6661101d53b07e53e0d44ec5695df741c5c71398`.
The source tree at inspection already contained active 0051 query-language edits.
Those unbuilt edits were inspected, not treated as shipped behavior or formatted
and tested underneath the other session.

A private two-file fixture was exercised through the real executable's headless
input/event/render path, at 140×40 cells:

```text
resize 140 40
keys <space>/needle
settle
state
frame
keys <esc><esc><space>R
state
frame
```

Observed: Grep finished with `picker_input="needle"`, two items and no streaming.
After closing it and entering Replace, the input was empty and there were zero
items. The Grep card occupied a smaller centered rectangle; Replace occupied
almost the entire frame. This establishes the session reset and geometry split
in that baseline, not a finding about the eventual 0051 result.

Source inspection explains the split:

- `strop-picker::Kind::{Grep, Replace}` share `Picker` and its catalog/ranking
  machinery. Replacement input, focus and exclusion sets already live there.
- `editor/picker::picker_input_changed` uses the same GrepWorker path for both.
  The new `strop-picker/src/query/` parser is also shared. Do not build another
  parser, filtering convention or search provider for this change.
- `set_picker`/`close_picker` destroy the glue, query/draft, catalog, selection,
  exclusions and request ownership. Reopening a different kind cannot be a
  correct toggle implementation.
- `render/picker/mod.rs` chooses different card geometries; `rows.rs` has a
  separate replacement renderer. `accept_current_picker` changes Enter from
  opening a source to replacement application based on Kind.
- Ctrl-R is already decoded and currently a no-op inside the picker host.
  Ctrl-X is advertised for row exclusion but was unwired in the inspected
  in-flight dispatcher. Reconcile that with 0051's final implementation rather
  than preserving a dead hint or treating this temporary state as a release claim.

The code anchors below are symbols/responsibilities, not permanent line numbers.
0051 is allowed to split their current files.

## 3. Entry, toggle and keyboard contract

### One invocation, two useful entry intents

- **`Space /` — Search workspace.** First invocation starts with replacement
  hidden. Re-entering a retained investigation restores its state, including
  whether the user enabled replacement. It does not silently clear a draft.
- **`Space R` — Search with replacement.** Opens the same workspace and ensures
  the With row is visible. Reuse a compatible retained session, not another
  catalog. An empty query starts in Find; an existing query can focus With.
- **`Ctrl-R` inside Search — toggle replacement.** Works while editing either
  field and in the field's Normal mode. Showing With focuses its parked editor;
  hiding it returns focus to Find. Keep the replacement text, caret and edit
  state while hidden. Scope this command to the Search host: document Ctrl-R
  remains Vim redo, and other picker/input hosts do not acquire this binding.

Use the already representable Ctrl-R, not an Alt sequence with legacy Esc
ambiguity, an indistinguishable Ctrl/Shift combination, or a new global `R`
override. The Search host owns this action; do not bake it into the reusable
LineEdit primitive.

### Focus and acceptance

There is no need for another mini-mode or a third results-focus state merely to
merge the two existing surfaces. Retain the established field editing behavior:

| Input / owner | Meaning |
| --- | --- |
| Up/Down, or j/k in field Normal mode | Move the selected logical result without altering either field. |
| Tab/Shift-Tab, replacement hidden | Next/previous result, as in the existing search picker. |
| Tab/Shift-Tab, replacement visible | Cycle Find and With; arrows still move the result. |
| Enter, Find owns input | Open the selected source, whether With is visible or hidden. |
| Enter, With owns input | Prepare a replacement review. Never mutate files or buffers directly. |
| Ctrl-X | Toggle inclusion of the selected match in the workset. |
| Ctrl-D | Toggle exclusion of the selected match's entire file from the workset. |
| Ctrl-O | Open the included workset as a source-backed collection, in either mode. |
| Ctrl-Space, Find | Existing parser-driven query suggestions from 0051. |
| Ctrl-Space, With | No query/code completion; With is literal replacement text. |
| Esc, suggestion list open | Dismiss that list using the established suggestion owner. |
| Esc, field Insert mode | Enter field Normal mode. |
| Esc, field Normal mode | Hide/cancel the workspace and restore the origin view. |

The footer is focus-dependent: **Enter open** in Find; **Enter review** in With.
Showing replacement does not turn the primary search-field Enter into a global
edit. Suggestion acceptance takes precedence over either Enter action. A visible
Review action uses the same command as Enter in With; do not create a third apply
path. Changing input, hiding the surface or cancelling invalidates any queued
accept/review intent for the old state.

A field focus change may dismiss its suggestion popup. Retaining session data
must not leave query suggestions floating over a now-focused literal With field.
All dispatched actions appear in the existing contextual help/action registry.

## 4. The large workspace and visual hierarchy

Use one near-full-frame card with a small surrounding margin and the global
modeline left visible. **Its outer rectangle is identical with replacement on or
off** and independent of match count. No shrinking to a small Grep dialog, jumping
around as text is typed, or stealing/resizing the user's underlying editor panes.

Illustrative layout, not a capture of implemented behavior:

```text
Search — local /project                       18 hits · 3 files · 16 included
Find  language:rust path:src/ text:"retry"                    Ctrl-R replace
With  retry_later                                           replacement on
Hidden on · ignore rules on                       2 excluded from workset
──────────────────────────────────────────────────────────────────────────
query.rs  src/search/        9 hits │ query.rs:42       src/search/
   42  retry(request);             │   40  let request = prepare();
   58  retry(next);                │   41  if ready {
client.rs  src/net/          6 hits │   42      retry(request);
   91  retry(connection);          │   43  }
…                                  │ …
──────────────────────────────────────────────────────────────────────────
Enter review · Tab field · Ctrl-R replace · Ctrl-X hit · Ctrl-D file
```

### Result identity before decoration

1. **File identity is primary.** A file heading or the shared two-line file card
   shows a strong basename, quieter parent path, hit count and relevant modified/
   source-state badge. Adjacent matches may share a heading; identity must remain
   visible when a group begins above the viewport. Never return to replacement
   rows containing source text with no filename.
2. **Source lines are source coordinates.** Aligned, subdued line-number gutters;
   columns only when useful for distinguishing same-line matches. Do not expose
   synthetic collection rows or catalog indices as source line numbers.
3. **Code is code.** Use the existing source/excerpt syntax layers where valid,
   then the exact match or replacement delta overlay. Preserve indentation,
   tabs, Unicode cell widths and clipped-match visibility. A syntax fallback is
   neutral readable source, not an invented semantic classification.
4. **Selection has a clear full-width band.** Selected, excluded, matched and
   changed are distinct states. An exclusion marker/dim treatment must not look
   like a deletion diff. The selected match remains identifiable when excluded.
5. **Backgrounds separate functions.** Use existing palette roles, or a small
   coherent addition for base surface, raised/file header and selected row. Give
   the preview a distinguishable surface/separator; do not surround every line
   with a box or introduce a general theme engine. Inactive text stays readable.
6. **Preview has a stable source header.** Filename, quiet parent/namespace,
   actual source range and loading/stale/error state belong to the selected
   source. A late response cannot relabel old code as the next selected file.
7. **Replacement is an extra layer on the same rows.** With visible, preview
   exact before/after spans for the included workset using the same pure edit
   resolver consumed by review. With hidden, show source matches again. Do not
   keep a separate old replacement row formatter as a permanent branch.

The Find row uses 0051's parsed tokens/highlighting/suggestions. With is plainly
labelled literal replacement text; `$1`, backslashes and colons do not acquire
query or snippet semantics. Scope/visibility state is explicit but subordinate
to the query and results.

### Responsive behavior

At wide sizes, results and preview sit side by side. At narrow sizes, keep file
identity, line information and the active field; stack or temporarily hide the
preview before crushing both columns. Changing With visibility can change one
inner row, but preserve the selected hit and scroll anchor rather than resetting
to the top. Clip metadata before the basename or match. Derive row geometry from
display cells and logical rows, including shared file headings.

Capture at least 140×40, 100×30 and 80×24, plus a tiny-terminal/resize recovery
case. Long paths, duplicate basenames, empty results, one result, many matches in
one file and many files must all use the same layout rules.

## 5. One workset, not hidden replacement-only exclusions

The query determines the result dataset. Inclusion/exclusion determines the
working subset for **both Collect and Review**. Browsing a selected source remains
possible even when that match is excluded from the workset.

- All current matches start included. Ctrl-X toggles one match; Ctrl-D excludes/
  restores a file without erasing that file's individual match decisions.
- Excluded rows remain visible and reversible. Show included/total counts and
  an exclusion indication even when With is hidden. Hiding replacement must not
  make an invisible set of exclusions surprising on the next Collect or Review.
- Ctrl-O collects the included matches, not all catalog storage and not whichever
  rows happen to be painted. Preserve 0049/0051 source identity, live-buffer
  authority, navigation and collection editing semantics.
- A pure toggle or replacement-text edit preserves exclusions. A semantic query
  or root change creates a new dataset and resets transient match decisions
  visibly; use explicit query filters for exclusions meant to survive new queries.
- Same-query refresh may restore decisions only against unambiguous source/match
  witnesses. Never transfer `HashSet<usize>` decisions to a differently ordered
  catalog. Report decisions that cannot be restored rather than guessing.
- All-excluded is a valid, visible empty workset. Collect and Review explain why
  there is nothing to act on; they do not fall back to all matches.

A current, verified individual hit can be opened while a search is streaming.
Collect and project-wide Review require a complete current dataset, or a separately
explicit bounded-partial-workset choice. Do not silently treat a truncated/error
prefix as the whole query. Queued intents are tied to the exact query/workset and
cancel on state changes; completing a stream must never trigger an obsolete edit.

## 6. Review, Apply and Save remain different operations

Reuse 0051's shared change proposal/review/receipt path. Preparing Review captures
scope, query generation, literal replacement draft revision, included match IDs
and authoritative source observations. It does not execute the old direct
`apply_replace` loop from a newly named widget.

The review lists affected files, exact before/after changes, exclusions, dirty/
stale/unavailable sources, and whether persistence is a later explicit action.
Cancelling returns to the same search investigation, not an empty Find field.
Applying uses the same checked edit plan that produced the preview. Saving uses
normal local/remote document authority; opening an unopened file to prepare a
proposal must not accidentally persist it while open files only become dirty.

An invalid/incomplete query, stale source, changed draft or superseded review
cannot apply an old plan. No blind replacement by remembered line/column; no
silent skip-and-success summary. Surface per-source conflicts and partial
outcomes under the established change-plan policy. No new automatic retry,
background save or overwrite exception is authorized here.

Search capabilities are separate from the visibility toggle. A scope whose
provider supports reads but not project replacement must explain that limitation
and refuse Review; it must never reinterpret remote paths as local destinations.

## 7. Session, request and return ownership

Keep one bounded retained search session per relevant workspace scope, or the
existing equivalent with an explicit bounded retention policy. The persistent
state is independent of whether its card is currently visible. A session owns:

- captured resource namespace/root and provider capabilities;
- Find LineEdit, parsed/compiled query and located diagnostics;
- a **stored replacement draft plus separate visibility/focus**, not an Option
  whose removal drops the draft;
- catalog, ranking and completeness/error state;
- selected match identity, viewport anchor and workset decisions;
- preview/review ownership and the originating editor view.

Avoid keeping a process alive just to preserve a query. Hiding/cancelling revokes
unneeded reads and queued accepts; retain bounded completed data and user intent.
On return, reuse data only if current or visibly refresh it through owned work.
Do not leave stale rows actionable because the retained object still exists.

Opening a source hides the card but retains the investigation. `Space /` returns
to its query, draft, match and view; `Space R` ensures replacement is visible.
Ordinary document jump history continues to work. Persistent collection/source
round trips retain their existing stronger view-history contract. This change
does not need to invent a second document-jump system for transient popups.

Changing source focus or process cwd cannot retarget a pending search or review.
`SearchScope` uses 0042's resolved resource identity, not a display label or an
unqualified remote PathBuf. Filesystem “Search here” supplies that captured scope.
Do not add an abstract provider framework before a real backend consumes it.

### Generation boundaries

| Event | Invalidation / work |
| --- | --- |
| With visibility toggle | Repaint/focus only; preserve query/catalog/rank ownership. No new grep or rank request. |
| With text edit | Invalidate derived replacement preview/review; no search restart. |
| Inclusion change | Update workset and invalidate its review; no search restart. |
| Query/filter/root change | New query dataset/ticket; cancel old producer/rank ownership and stale intents. |
| Selected hit change | New selected-source preview ownership, using valid cached source data when possible. |
| Source revision/binding change | Invalidate affected preview/plan and refresh or refuse stale hits. |
| Card hide/cancel | Retain user state, retire unnecessary work and queued acceptance; no later focus theft. |

Bound actual outstanding work and retained memory, not just the count of logical
owners. Do not reparse files, rebuild all row strings, scan whole catalogs or copy
ropes on each render/toggle. Large replacement preparation belongs on an owned
worker; installation and rendering consume prepared data for visible rows.

## 8. Integration map and clean cutover

1. `strop-picker/src/lib.rs`: replace separate content-search kinds with one
   Search kind/model and a replacement facet that retains its draft. Keep backend
   names such as GrepWorker where they still describe actual work; no gratuitous
   renaming or compatibility aliases.
2. `strop-engine/src/editor/picker/`: one open/resume entry, scoped toggle/focus
   dispatch, shared query execution and collection acceptance. Split lifecycle,
   input and search responsibilities if the current module is already large.
3. `editor/picker/replace.rs` and `editor/changes/`: migrate preparation onto the
   common proposal owner; remove obsolete direct-apply acceptance paths.
4. `editor/collections/`: accept the shared included workset regardless of With
   visibility; preserve authoritative source mapping rather than copying rows.
5. `render/picker/{mod,rows,preview,...}`: shared card and semantic match rows,
   optional delta layer, focus-aware hints/counts. Reuse 0050/0051 components.
6. `keymap`, help, trace/replay identities and existing tests: migrate every
   separate-kind callsite and documented action. Use LSP references for exported
   symbol changes. Do not leave a deprecated Replace branch hidden behind aliases.

Avoid another giant `SearchEverything` module. Query parsing stays in the query
module, transport stays in its provider, pure edit resolution stays with changes,
and terminal geometry stays in render code.

## 9. Acceptance and evidence

Keep regressions for actual uncertain behavior, not field copies or hardcoded
help wording. The implementation evidence must demonstrate:

- Search → type/filter/select/exclude → Ctrl-R → edit With → Ctrl-R twice:
  unchanged query/result identities/workset/view, retained draft, and no new grep
  or rank work caused by toggling. Exercise a still-streaming query too.
- Enter in Find opens the same source with replacement on/off. Enter in With
  opens a review and leaves all source bytes unchanged until Apply.
- Query suggestions own Enter/Esc only while present. Ctrl-R elsewhere retains
  ordinary editor behavior. Clipboard paste edits the focused field only.
- Ctrl-X/Ctrl-D visibly curate the same workset in both modes; Ctrl-O promotes
  exactly that set, including all-excluded and same-line multiple-match cases.
- Cancelled review and source re-entry restore the investigation; a changed
  source/query cannot revive an obsolete review or misplace exclusions.
- Invalid/incomplete queries, producer failure/truncation, delayed preview and
  cancellation cannot open/apply a misleading old result.
- Source filename/parent/line identity, code/diff styling, selection, exclusions,
  scope, backgrounds and tiny/narrow resizing are verified in actual TUI captures
  and focused existing TestBackend coverage. Mockups are not this evidence.
- Open and unopened sources follow identical Apply-vs-Save policy; dirty buffers
  and failed saves retain their changes and truthful per-source receipts.

Run the specific behavior walkthroughs, then the repository's integrated
`docker compose run --build --rm test` gate (fmt, locked clippy/all-targets and
locked tests). Update existing user docs/changelog and the roadmap acceptance
ledger. That gate belongs to implementation, not to this documentation-only
investigation while another session is editing source.

## 10. Integration decisions for 0.30.0

- `Kind::Search` is the one content-search model. Replacement visibility is a
  facet; its parked `LineEdit` and modal/caret state are never destroyed by toggling.
- One retained investigation belongs to a captured `SearchScope` resource root.
  Re-entry refreshes through owned work rather than assuming disk results stayed
  current. Pure replacement toggles/draft edits never restart search or ranking.
- Workset decisions use native file identity, source coordinates and exact shared
  line witnesses, not catalog indices transferred to a new dataset. Refresh drops
  and reports decisions whose witnesses no longer identify the same match.
- The shared result renderer owns one stable outer card, logical-row viewport,
  file identity/code rows and optional delta layer. Find Enter opens a source;
  With Enter prepares review. Contextual hints follow the actual input owner.
- Replacement preparation freezes source observations and runs as a finite owned
  job. Query, draft/workset and prepared-review generations are independent;
  late preparation never retargets or steals newer focus.
- Existing change plans, exact edit witnesses, source permissions and explicit
  persistence receipts remain authoritative. Replay semantics advance for the
  changed model and keyboard contract; no old-kind aliases remain.

## 11. Implementation and acceptance — 0.30.0

| ID | Implementation and exercised contract |
| --- | --- |
| S01 | `strop-picker::Kind::Search`, one engine open/resume path and common row renderer. Removed the separate Replace kind and old per-file open/replacement assembly. |
| S02 | Parked Find/With `LineEdit`s and independent replacement visibility; the live-query regression keeps the existing producer while toggling, pasting and changing field modes. No new grep/rank work from a pure toggle. |
| S03 | `render/picker/{mod,rows,preview}.rs`: one near-full-frame rectangle, basename/parent/source coordinates, per-file hit counts, source-buffer badges, full selection/exclusion bands, raised headings and source-preview surface. Tiny views prioritize the active field without changing the retained viewport. |
| S04 | Contextual registry/help and shared field dispatch: Find Enter opens, With Enter reviews, Ctrl-R toggles, Ctrl-X/Ctrl-D curate, Ctrl-O collects. Suggestions own acceptance only while visible; terminal bracketed paste edits the focused field. Document Ctrl-R remains redo. |
| S05 | Source-witness workset, not catalog-index exclusions; visible counts in both presentations. Same-line individual decisions survive a file mask/restore. Both collection and review refuse all-excluded/incomplete/error datasets. Collection admission verifies source witnesses before projecting coordinates. |
| S06 | `changes/review/prepare.rs` freezes observations and prepares diffs on an owned worker. Canonical aliases coalesce; dirty buffers win; moved revisions/bindings, closed/read-only/unavailable sources are named refusals. Apply uses the reviewed edits; Save is separate. Existing open/unopened Apply/Save, stale-target, failed-read and remote-receipt regressions pass. |
| S07 | `picker/search.rs` retains one bounded investigation. Source opening, Cancel and re-entry preserve query/draft/selection/workset; refresh restores semantic hit identity even when stream order changes. Lost witnesses are reported, not transferred to new source text. |
| S08 | Typed captured `SearchScope`, independent dataset/intent/preparation tickets, finite cancellation and focus guards. Tests cover re-entry after cwd changes, dirty relative display spellings, late completions and save-as binding changes. Unsupported explicit namespaces refuse; only the local project backend is implemented. |
| S09 | Shared model, source row/window projection and pure witness validator. Preparation, retained lifecycle and workset have separate modules. Modified production modules remain below the 800-line ceiling; the command table remains the existing single-pattern artifact. Replay semantic version advances to 2. |
| S10 | Focused state and TestBackend regressions, release-binary headless/full-replay walkthroughs, actual terminal captures, user help/README/generated compatibility docs and measured project replacement. Required final gates and hosted release checks are recorded below. |

### Surface and behavioral evidence

- Final GNU/Linux release executable SHA-256:
  `067c42a03bc91f18d32ad094ec5d5da36df0e3543919b9078b8d4506f6d46466`.
- Private controlled fixtures and captures live under
  `/tmp/strop-search030-yqaed_fl/`. `candidate-tui.jsonl` records the actual terminal
  executable; `candidate-*.json` retains emitted cell grids, with SVG projections
  for visual inspection. Projections are not native Windows Terminal screenshots;
  the projection font lacks some CJK glyphs, while the grid retains original UTF-8.
- Exercised 140×40, 100×30, 80×24 and tiny/resize recovery, duplicate basenames,
  long parent paths, Unicode/tabs, same-line multiple matches, selection/exclusion,
  literal `$1` paste, common source/delta rows and syntax-spanned source preview.
  Separate headless captures exercise empty/one-result and suggestion/incomplete
  query states. A repeated `path:` qualifier is an OR filter, not an empty-result
  probe; the actual empty probe uses a unique unmatched content expression.
- `accepted.jsonl` is a complete release-executable Search → toggle/exclude →
  Review → Cancel → source open → retained re-entry recording; full replay checks
  the recorded requests, state and rendered observations.
- Final actual-TUI Apply/Save walkthrough: disk stayed unchanged after Apply;
  Save wrote seven literal `dispatch_$1` replacements and retained the one excluded
  `needle`. Full replay of `candidate-tui.jsonl` completed with `should_quit=true`.
  Its trace contains one grep launch; subsequent replacement toggling/paste/resize
  issued viewport analysis and Review work, not another grep or ranking request.
- `picker/search_tests.rs` defends real transition and authority boundaries:
  producer-preserving toggle, semantic re-entry, shared same-line worksets, changed
  witnesses, late preparation, pristine scratch preservation, revision-keyed
  preview validation, alias coalescing, save-as refusal and cwd-independent dirty
  source authority. Existing review/picker/collection tests use the migrated path.

### Measurements and gate limits

- Final release build, this WSL2 Ryzen 9950X3D host: 500 input+frame samples over
  10k lines at 120×40 — p50 0.41 ms, p95 0.53 ms, p99 0.64 ms, max 1.04 ms.
- 300 files × 50 matches, eight fresh replacement runs: search+settle p50/p95
  124.71/130.97 ms; Review+settle 38.35/39.11 ms; Apply+frame 19.54/19.84 ms;
  Save+settle 44.25/47.81 ms; retirement 4.65/5.07 ms. The benchmark reads every
  saved file back; it does not count a refusal or a no-op as a fast replacement.
- Compose `model` and `verify` passed; the edit kernel has 10 verified obligations,
  zero errors. These existing proofs do not prove terminal layout or filesystem
  scheduling.
- Validation exposed an existing vacuous container gate: its image lacked the
  Docker CLI. A dedicated test stage now supplies it; explicit
  `STROP_CONTAINER_TESTS=1` fails on an unreachable engine instead of skipping.
  The normal compose container gate ran 25 unit tests and five real-engine cases;
  an unreachable-engine negative control failed as required.
- Final `docker compose run --build --rm test` passed fmt, locked all-target
  clippy with warnings denied, and the complete locked suite with real SSH
  fixtures required. The corrected `container-test` service passed without any
  manual package installation. Publication/CI/demo must succeed for `v0.30.0`
  before the release task is complete.
- No S requirement is deferred. The filesystem workspace (0054), completion
  program (0052), and terminal/GUI program (0055) retain their independent scope.
  Measurements and test counts are evidence for the exercised paths, not a claim
  that every editor defect or schedule has been exhausted.
