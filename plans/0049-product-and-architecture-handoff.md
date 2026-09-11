# 0049 — Product and architecture handoff: finish the multibuffer experience

Status: **§4 landed in 0.24.0; §§5–8 core landed in 0.25.0** (collection
correctness: identity, scoped undo/redo, projection invalidation,
save/close, g<Space>; occurrence selection in buffers and collections;
review for multi-target rename/code-action). Boxed per-file cards with badges, gap rows,
path ordering and picker locator polish landed in 0.27.0. Remaining on
the roadmap (0028 "Current program"): §6 syntax projection, §8 bounded
delivery + project-replace migration + applyEdit, §9 accompanying work,
§10 later. Known limit vs §5: write-back happens at action boundaries;
journal-driven per-keystroke write-back is the named follow-up.

Baseline: `fc3fbfa673171e4260530f45f0b170dac2d25f68`, Strop 0.22.1.
Plan 0047's symbol/jumplist/mark work is in flight in the shared checkout;
0048 records the confirmed Escape-input defect. Reconcile both before editing
their files. This document updates the delivery assessment in 0041; it does
not erase its historical decisions or restart completed work.

Inputs: the original 9 September `strop-implementation-handoff.md` at
`/mnt/c/Users/tarek/Downloads/`, plans 0041–0048 and their actual consumers,
Rootle's grep/file-find presentation at `../rust/rootle/`, current source,
and production-binary experiments described below.

## 1. Designer's decision

**The next flagship is a trustworthy, attractive, source-backed multibuffer
with occurrence selection. Not another backend, GUI, or generic framework.**

The mechanism exists. The experience does not yet meet the original handoff:
users can collect and change code, but ordinary undo fails in the collection,
grouped undo leaves stale displayed text, saving addresses an unnamed scratch
buffer, source navigation/context controls are missing, and the surface does
not show source-aware syntax. Calling this a finished Zed-like multibuffer
would be misleading.

The user's priorities in this review are explicit:

- Fix undo and source/view synchronization, not just the appearance.
- Add matching-word/selection multicursors; column stacking is not enough.
- Use Rootle's well-presented file/excerpt boxes as a concrete visual precedent,
  with syntax highlighting and useful source context.
- Preserve C++ language services after `gd` leaves the working tree for a
  library/system header.
- Keep Vim grammar, real buffers, responsive input, and the existing TUI.
  GUI remains last and deferred.

Recommended order:

| Priority | Complete outcome | Why now |
| --- | --- | --- |
| P1, first | Land 0048; repair external-header LSP continuity (§4) | Basic mode/navigation trust. The LSP fix can proceed independently of collection work, after coordinating with 0047. |
| P1, next product milestone | Source-backed collection transactions, ordinary undo/redo/save, Rootle-inspired presentation, occurrence multiselection (§§5–7) | Finishes the feature the user actually wants. A prettier broken scratch buffer is not acceptable. |
| P1/P2, next | Reviewable project changes and visible receipts (§8) | Rename/code actions currently apply immediately; multi-file intent must become inspectable. |
| P2, accompanying work | Narrow the engine boundary, correct work placement, improve explanation and honest gates (§9) | Do the enabling work needed by the actual workflows, not a whole-editor rewrite first. |
| Later | Structural selections, then saved investigations and source-bound task evidence (§10) | Build on a usable selection/change system; do not start with a recipe language. |

Landing order may use smaller complete changes. Product sign-off for the
multibuffer milestone requires **behavior + selection + presentation together**.

## 2. What landed from the original handoff

“Implemented” below means current source and entry points exist. Historical
release/gate statements are not new gate runs. “Partial” is a scope statement,
not a request to replace the working part.

| Original investment | Current delivery | Remaining gap / disposition |
| --- | --- | --- |
| S0 Vim fidelity | WORD objects (`ciW`/`diW`/`daW`) and differential cases delivered; typed input-owner dispatch delivered | Keep these, fix 0048. Do not reopen the old 0041/0028 statements as if these were still absent. |
| S0/S4c forensic capture | Schema-3 chunking and strict assembly exist in `strop-trace/src/{lib,chunk,replay}.rs` | Preserve completeness/privacy limits. This is not permission to call every oversized diagnostic record replayable. |
| S1 resource identity | `strop-workspace`, namespace-aware resource types, local/SSH/container identities, editor registry, `:explain` | Shared filesystem/execution-operation routing is not complete. Registry incarnation is not a universal admission fence. The relative-path collection bug demonstrates a consumer bypassing existing identity rules. |
| S2 engine | `strop-engine` is real; production Cargo graph has no Ratatui/Crossterm; frame capture is injected | Public mutable state remains broad. Rendering still calls service/analysis admission and mutates viewport state. 0046's read-only presentation/API-tightening stage is incomplete. |
| S3 containers | Read-only browse/read, identity revalidation, container server launch, Git discovery/context exist | Container location navigation and diagnostics-picker arms still reject/drop results; commit diffs are refused; writes remain deliberately unsupported. Not full development-workspace parity. |
| S4 change plans/LSP | `:format`, `:rename NEW`, `Space a`, receipts, `:undo-change`, revision-checked mutations | No user-visible prepared-plan review/confirmation, no bounded multi-document delivery, unopened/unbound edit targets refused, no implemented `workspace/applyEdit` acceptance path. |
| S4 project replacement | Real project replacement retains per-hit verification and its owned open/save path | Shares the mutation gateway, not the change-plan/receipt workflow. Migrate without weakening its stricter stale-hit checks. |
| S4 formal work | `specs/ChangePlan.tla`, `change-gate.sh`, and inclusion in `specs/gate.sh` exist | 0043's sentence saying the model remains open is stale. Do not build another model because of that sentence. Extend correspondence when behavior changes. |
| S5 editable collections | `Ctrl-O` in grep/references/diagnostics, local background source loading, overlap merging, multi-region write-back, permitted remote sources | A projection/write-back foundation, not the complete reading/editing/undo/save/selection experience. Unopened remote targets are not loaded by the local background-load loop. |
| Occurrence/visual multiselection | `Q`, `Space c`, mirrored insert and normal-mode cascades exist | No next/all occurrence command. Visual operators generally consume the primary range; `SelectionSet::set_extras` rebuilds extras as collapsed cursors. Not full multi-range editing. |
| Explain/discoverability | Searchable `:help`, generated keybinding help, which-key, Ex-name completion, picker `ctrl-o collect` hint, `:explain` | Effective binding decisions, config-source provenance, complete Ex help and inspectable change receipts remain incomplete. Do not add a second Ex registry: `normal.rs::EX_COMMANDS` already exists. |
| S6 structural selections/recipes | Vim text objects/macros are real; Tree-sitter highlights and guides are real | Node/field selection and reusable semantic recipes are not implemented. Symbol navigation in 0047 does not supply structural selection. |
| S7 investigations/checkpoints/tasks | Automatic per-project sessions and per-buffer undo persistence; explicit shell output buffers | No named investigations/saved collection definitions, selective checkpoints, structured task/test results tied to source/environment identity. |
| S8 provisioning | Existing-container attachment only | No full Dev Container discovery/trust/provision/rebuild lifecycle. Keep deferred. |
| S9/S10 | Text-hunk Git workflows, LSP, shell jobs | No structural move/function review, generalized resource-operation undo, or external-tool result protocol. Keep behind completed editing workflows. |
| Verus / broader verification | Production-linked edit geometry pilot and compose/CI verification exist (0045) | Composition/stretch properties are not all proved. Loom/Miri/fuzz breadth remains a separate evidence-led decision, not a prerequisite to basic UX. |
| R1 / G1/G2 | No installed Strop daemon or GUI implementation | Correctly deferred. Engine extraction is not authorization to begin a GUI. WSL evidence is not native-Windows evidence. |

Important status cleanup at implementation time: 0041 still defers Verus,
0044's body still describes obsolete v1-only restrictions, 0046 still says
stage A is landing, and portions of 0028 describe already-replaced dispatch.
Keep one current acceptance/limitations record at the top of each affected
plan. Preserve history, but never make readers infer completion from releases
and contradictory paragraphs.

## 3. Evidence from the real editor

### Scope and reproducibility

Used a frozen copy of the installed `strop 0.22.1` (SHA-256
`3a808f32ddbcaf0712aa703c02c6a8bd76552413da27a64328ee6d273576924f`), plus
cached Docker image `8e925f90268c`'s debug binary for the path comparison.
The latter's input-boundary source and lockfile had been compared against the
baseline during 0048. Fixtures, HOME/XDG state, scripts and traces are private
under `/tmp/strop-handoff-audit/`; no project sources were used as edit fixtures.

These are real `strop --headless` workflows and rendered TestBackend grids,
not a separate editor model. They establish editor/state/render behavior, not
physical terminal latency. 0048 separately exercised real PTYs. No new full
compose/model/proof gate was run for this documentation-only audit.

Two source files:

```text
a.txt: alpha needle one\nkeep a\n
b.txt: beta needle two\nkeep b\n
```

From their directory:

```text
keys <space>/needle
settle
keys <c-o>
settle
frame
keys :%s/needle/thread/g<cr>
state
frame
keys :w<cr>
settle
state
keys :undo-change<cr>
state
frame
```

Run with an absolute `a.txt` operand for the two-source control. A relative
`a.txt` operand is the identity-defect reproduction.

| Observation | Result / source cause |
| --- | --- |
| Relative startup path, grep shows two hits | Collection includes only `b.txt`; one hit is skipped. Both installed and cached binaries reproduce it. Absolute startup yields both excerpts. |
| One ranged substitution over both excerpts | Sources really change through the plan gateway; trace records changes in both source buffers. This feature is not imaginary. |
| Plain `u` after a collection edit | Reports `already at oldest change`. Every regeneration calls `SystemEdit::replace_all`, which resets the projection's history (`buffer/mutation.rs:291–304`). |
| `:undo-change` | Trace records both sources restored to `needle`; next collection frame still shows `thread`. Projection invalidation is missing. |
| `:w` in the collection | Reports `write failed: no file name — :w {path} to name it`, despite source buffers being the real save targets. |
| `q` in the collection | Starts the normal macro-prefix input; does not close. The generated title falsely promises `q closes`. Preserve Vim `q`, correct the collection contract. |
| Appearance | Anonymous `[scratch]`, global projection line numbers, textual separator lines. User reports missing syntax highlighting; source analysis has no per-excerpt syntax projection. |
| `:explain` | Real workspace/resource/server/config sections, but no active-key decision or per-setting provenance. |

**Do not misdiagnose undo as bypassing the mutation lease.**
`doc_mut(...).buf.undo()` publishes through `DocumentEdit::Drop` →
`sync_document`; excerpt anchors are remapped. The missing operation is
refresh/invalidation of the dependent presentation and its history ownership.

Identity defect locations: `collections/mod.rs::open_collection_from_picker`
and `build_collection` compare `buf.path` with a cwd-joined absolute path.
`Buffer::open` preserves the spelling; ordinary open dedup already uses
`Document::matches_target` and `file_identity`. Reuse that owner-aware rule;
do not canonicalize remote paths locally or add another identity convention.

### Measured work-placement problem

Ordinary `--bench input_frame`: 500 samples, 10k lines, 120×40, key+frame
p50 **0.83 ms**, p95 **1.19 ms**, p99 **1.34 ms**, max **2.87 ms**.
This is a local observation, not a claim about every editor operation.

Separate collection probe: alternating matching/context lines in one file,
one excerpt per match, then `3G0iX<esc>`. Metadata trace elapsed time from the
semantic Escape input record to the next render record:

| Excerpts | Source size | Commit-to-next-render interval |
| --- | --- | --- |
| 100 | 2.7 KB | 1.342 ms |
| 1,000 | 27 KB | 23.563 ms |
| 2,000 | 54 KB | 89.397 ms |

One sample per size, recording enabled; **not p95/p99**, not enqueue latency,
and not physical display latency. Linux/WSL2 x86_64, Ryzen 9 9950X3D. Exact
compiler/optimization flags of the installed binary were not recovered.

The source explains why this needs work: `maybe_sync_collection` flattens and
clones the projection; `diff_lines` allocates a whole-view O(n×m) LCS table;
write-back and regeneration run at a normal-mode action boundary. The code's
“collection views are small” assumption contradicts S5's thousands-of-excerpts
acceptance target. Moving the existing quadratic algorithm to a worker alone
would hide the stall, not fix its retained-memory cost.

Evidence files: `discover.keys`, `edit-collection.keys`,
`collection-controls.keys`, `edit-collection.jsonl`,
`collection-controls.jsonl`, `collection-identity-results.json`, and
`collection-scale-results.json` plus their `.out`/trace fixtures. The recipes
and observations above remain useful after temporary files disappear.

## 4. First independent fix: external-header LSP continuity

User report, accepted as ground truth: `gd` from project C++ reaches
`vector.hpp` outside the working tree; the next `gd` reports no language server.
Static inspection identifies a concrete matching mechanism; this audit did
not rerun the user's clangd session.

- `.hpp` maps to `cpp` in `strop-lsp/src/registry.rs`.
- `lsp.rs::finish_lsp_jump` has the original `ReplyContext` and source binding,
  marks outside-root content read-only, then calls `lsp_maybe_attach` without
  preserving the replying server as the target's navigation context.
- `lsp/lifecycle.rs::lsp_server_for` requires matching language/namespace and
  `path.starts_with(root)`. An external header fails that test.
- Both `lsp_did_open_current` and `lsp_request_with` repeat root-based lookup.
  The latter emits the misleading installation advice.
- Extensionless standard-library headers such as `.../include/c++/vector`
  additionally fail the extension-only language guard.

### Required behavior

1. A fresh, server-originated navigation carries its **language-service
   context**, not just the target path. For a compatible resource in the same
   namespace, retain the originating live server/project/compile context.
   Outside-root read-only policy remains separate from language-service access.
2. Resolve requests from an explicit active view/navigation context and its
   valid binding first. Longest-covering-root discovery remains the fallback
   for unbound ordinary opens. Never scan unrelated live servers and pick the
   first one merely because it speaks C++.
3. Use the same binding-aware resolver for didOpen, sync, request preparation,
   chained jumps and retirement. **Adding a binding only in `finish_lsp_jump`
   is insufficient while request-time code still ignores it.**
4. Inherit the originating language for extensionless/ambiguous C/C++ headers
   when supported by that service. Do not globally label extensionless files
   C++, override unrelated explicit languages, or rediscover project commands
   from `/usr/include`.
5. Same shared header reached from two projects must retain the intended
   project context per view/navigation. Reuse source text identity; do not
   silently switch another pane's compile context through the current global
   `DocumentId → Binding` slot. Make the service-binding scope explicit.
6. A manually opened external file with no unambiguous context gets a truthful
   explanation and an explicit context-selection route, not arbitrary reuse
   or false “install clangd” advice. Server death, unsupported capability and
   unknown language are distinct outcomes in messages and `:explain`.
7. Do not change cwd, widen the server root to `/`, spawn a new clangd per
   header, grant write authority, or cross local/SSH/container namespaces.

Primary files: `editor/lsp.rs`, `editor/lsp/{state,lifecycle,attach}.rs`,
`editor/io.rs`'s `OpenIntent::LspLocation`, and the existing LSP regression
fixtures. Extend existing bindings/tickets; no generic provider rewrite.

Acceptance: project → external `.hpp` → second definition; extensionless
`vector`; ambiguous `.h`; two project contexts sharing a header; back/forward
navigation; server retirement/restart and late replies; manual open without
context; namespace rejection; didOpen language and actual subsequent server
request. A real clangd smoke must accompany deterministic ownership tests.
Coordinate the shared `lsp.rs`/picker boundary with 0047's implementation owner.

## 5. Multibuffer correctness contract

### Source ownership, projection and input

The source documents own text, revisions, history, language services and save
baselines. A collection owns query provenance, ordered excerpts, selection and
viewport state, plus a derived presentation. It is not a second set of files.

- Give collections explicit document/surface identity; stop presenting an
  anonymous Output/scratch buffer as an ordinary savable document. Reuse
  existing document IDs; add distinct identities only for distinct lifetimes
  such as an async build or a stable excerpt that survives reordering.
- Use a versioned source/projection map with typed protected rows. Translate
  the actual resolved operation to source `ChangeSet`s **before mutation**,
  then use the existing gateway. Preview and execution use the same prepared
  operation and mapping, including admission/refusal.
- Eliminate full-view diffing as the primary edit transport. Existing mutation
  journals already describe changes; source/projection geometry should locate
  the touched spans without comparing every line with every other line.
- Show typing in every view of a source as it happens, while keeping one Insert
  undo group open until its normal boundary. Do not wait for Escape to make a
  second pane reflect the typed source, and do not add a modal confirmation
  dialog to each ordinary keystroke.
- All edits to a source—collection, ordinary pane, undo/redo, LSP or reload—
  invalidate dependent projection regions. Refresh text and mapping together,
  preserving the logical excerpt/caret and selection endpoints. No stale text
  may remain presented as current after an acknowledged undo.
- Preflight protected spans, source revisions and capabilities for the whole
  intended interactive edit. A header-crossing or read-only-source edit must
  not first modify some other targets and only then reveal the refusal.
  Preserve the user's pending input when a conflict needs resolution; never
  discard an Insert session by blindly regenerating the view.
- Build/refresh jobs carry their own owner and intended view. Replace the
  unowned `waiting` countdown with request membership/terminal outcomes tied
  to the current build. Cancelling/reopening cannot let an old source load
  complete a newer collection or steal focus.

Heavy map/index/snapshot preparation and retirement belong on owned workers;
interactive publication must be bounded. Keep ordered typeahead and small
local edits independent of slow remote sources. Source lookup, overlap
merging and selection dedup use resource/document identity, not display paths.

### Undo and redo

**`u`/`Ctrl-R` in a collection undo/redo its last editing group across the
actual affected sources.** Normal source buffers retain their buffer-local
Vim history. Do not make `u` an alias for “pop whichever global receipt is last.”

- Record collection-originated groups against source history/changes, not the
  regenerated projection rope. `SystemEdit` resetting a generated snapshot's
  history is reasonable; using that history as collection undo is not.
- Keep exact group identity, source before/after history state, selection
  restoration and outcomes. Support two successive group undos/redos: a
  monotonic `BufferRevision` alone cannot identify an earlier history position
  after an intervening undo has created newer revisions.
- Preflight every member. An intervening independent edit must survive and
  cause a visible conflict/refusal or a separately reviewed inverse plan.
  Do not consume the receipt on refusal or silently skip a file.
- Refresh all dependent projections after history moves. Source-buffer undo
  can invalidate a collection group; it must never leave a lying view.
- Huge project groups can have bounded delivery and explicit partial outcomes;
  do not claim unlimited cross-file atomicity. Partial history/save outcomes
  remain inspectable and recoverable rather than only a transient count.

### Save, close and source navigation

- `:w` saves the collection's dirty source buffers through their existing
  backend save paths, with a deduplicated target list and per-file outcomes.
  Show which files are modified, saving, refused, conflicted or unconfirmed.
- Apply is not Save. Remote permits and verification remain authoritative;
  `:w!` grants no new capability. Container sources remain read-only unless
  their separate conditional-write contract is implemented.
- `:w PATH` must not silently write headers/snippets as though they were the
  source file. Refuse that ambiguous operation; exporting a presentation is
  outside this milestone.
- `:q` closes the collection view, not its sources and not their dirty edits.
  `:wq` closes only after the scoped saves are confirmed; failures stay visible.
  Unpublished conflict drafts cannot disappear on close.
- Preserve normal `q{register}` macros. Remove the false `q closes` title.
- **`g<Space>` opens the full source under the primary caret** from a
  collection in Normal or Visual mode. This is `g` then Space, not a leader
  sequence. In an excerpt body, map to the exact source byte/character
  position; on a file header, open that file at its first displayed excerpt.
  Reuse the live source document, including unsaved edits, in the current
  pane—do not reload disk text or open an independent copy.
- Enter on a protected file header and `:collection source` invoke the same
  action. Enter inside code retains its normal Vim motion; `gd` retains LSP
  definition navigation. Register the action and its hints through the existing
  command/input tables, not a terminal-level shortcut.
- Record the collection's view identity, logical caret, viewport and complete
  selection set before jumping. **Ctrl-O returns to that working context**,
  remapped through any intervening source changes rather than restored by stale
  row number. Preserve the selected source's workspace/LSP context (§4).
- With multiple selections, open only the primary selection's source; retain
  the other selections in the collection for the return trip. No surprise
  multi-file tab/pane explosion. Closed sources or stale maps use the owned
  open/refresh path with an explicit outcome, never a guessed offset or a
  local-path fallback.
- Advertise `g<Space> open source` in the collection's contextual hint surface
  and explain Enter's header-only action. This must be discoverable after
  grep → collection, not a shortcut the user has to know in advance.
- Provide previous/next excerpt, expand context and refresh actions through
  the same command/help registry. Do not hijack `gd` into a different operation.
- Vim text motions retain their collection-view meaning. Show source
  coordinates as labeled provenance; do not silently change `:N` or counted
  motions into a different address space. Open the source for whole-file work.

## 6. Presentation: Rootle's clarity, Strop's real editing

### The concrete precedent

Rootle's actual implementation is reusable as a **design**, not as another
readonly widget stack:

- `../rust/rootle/crates/rootle/src/components/global_search/render/results.rs`
  `hit_box`: path in the top rule, right-side match/stale badge, focused rails,
  numbered source snippets, a gutter-aligned gap row, bottom rule.
- `global_search/render.rs::preview_line`: source number + dim `│` divider +
  styled text spans.
- `global_search/model.rs`: same-file result merging and match emphasis that
  composes with existing syntax styles.
- `global_search/results.rs` and `preview/{mod,render}.rs`: common full-file
  preview vocabulary, context expansion and return to the original result.
- `doc/house-style.md`: stable entity-based selection, width-aware clipping,
  scrollbar in the right border and one hint surface per input owner.

Do not copy Rootle's line-only readonly controls, GitHub/blob-fetch adapters,
whole-result-per-render materialization, or truncation limits as if they were
an editable source model. Do not add a dependency on the Rootle application.

### Proposed default at 100×30 (schematic, not a screenshot)

```text
 references: send_request       8 matches · 3 files · 2 selections · 2 dirty
╭─ src/net/client.cpp ─────────────────────── C++ · 3 matches · modified ─╮
│  84 │ auto request = make_request(options);                             │
│  85 │ send_request(request);                                           │
│     ⋮  19 source lines omitted                                         │
│ 105 │ retry(send_request, deadline);                                   │
╰────────────────────────────────────────────────────────────────────────╯
╭─ tests/net_test.cpp ─────────────────────── C++ · 5 matches · modified ─╮
│  31 │ TEST_CASE("request retry") {                                     │
│  32 │     send_request(fixture);                                       │
│  33 │ }                                                                │
╰────────────────────────────────────────────────────────────────────────╯
 NORMAL   tests/net_test.cpp:32:5 · view 9:5 · 2 selections in 2 files
```

- **One card per source file**, multiple disjoint excerpts inside it; do not
  box each individual occurrence. Merge overlapping context and retain exact
  editable-span ownership.
- Default to restrained rounded boxes/rails in Strop's existing palette.
  Focus uses the active card's border/path; character selections use bands;
  search matches use a separate quieter treatment. Neither should erase all
  syntax colors or confuse the active cursor with a selected file.
- Use original source line numbers, not synthetic concatenation numbers in
  the gutter. Show useful surrounding code, initially a small context radius
  merged per file; expand without losing the active excerpt/selection.
- Title/provenance must distinguish local, SSH and container sources when
  necessary. Include read-only, dirty, conflict, loading and stale states.
  The modeline names the collection/source context, never `[scratch]`.
- At narrow widths, shed badges/padding/side rails before source identity or
  code. Do not add a full metadata row per match; tiny dimensions must be safe.
- Border glyphs, gutters and hint chrome are decorations, not editable source
  bytes, clipboard text, search targets or LSP input. Protected metadata rows
  remain typed/addressable within the real collection surface.
- Keep query edits reversible and selection anchored to excerpt identity.
  Show partial/filtered/loading counts honestly; “select all” must not mean
  a silently capped subset.

### Syntax and language services are source-aware

`editor/analysis::FrameAnalysis` and ordinary buffer rendering already provide
the needed language spans and geometry. Request analysis for the **source
snapshot and visible source ranges**, then project its spans into each excerpt.
Do not parse one synthetic `.cpp` file made of unrelated snippets and headers.
Mixed Rust/C++/Python excerpts must retain their own language and injection
context; an excerpt can begin inside a multiline string/comment.

Use the same text/span/overlay machinery as ordinary buffers and picker
previews. Whole-file formatting and LSP requests address the underlying source,
not the collection's title text. Syntax preparation stays on workers; painting
borrows visible rows and cached spans. A late syntax result cannot restore old
text or clear newer user selections.

A reusable **source-excerpt presentation** seam should serve grep/file preview,
references/diagnostics collections and later change review. Share gutter,
clipping, syntax, overlays and provenance—not mutable application state.
This is a component split by responsibility, not a generic widget framework.

## 7. Occurrence selection: required in ordinary buffers and collections

Current Strop: `Q` toggles a secondary cursor; `Space c` adds one on the next
line; Normal-mode Escape clears extras. Neither selects matching occurrences.
The user recalled VSCodeVim's **`gb`** (`gc` is commentary). Zed's Vim multibuffer
commands are `gl` for next and `g a` for all. Borrow the behavior, not an
inconsistent pile of aliases.

### Proposed Strop interaction

- **`gb`: select/add next occurrence.** With no active occurrence selection,
  select the word under the caret. Repeating it adds the next unselected match.
  A nonempty characterwise visual selection seeds literal selected text.
- **`gB`: select all occurrences in the current scope.** A documented Strop
  extension, not a claim about standard Vim. Check the live binding table after
  0047; the audited table reserves neither `gb` nor `gB`.
- Provide named `:select-next`, `:select-all`, `:select-skip`, `:select-pop`
  actions for help/completion. Skip advances the candidate without adding it;
  pop removes the last-added occurrence without destroying the others.
- Escape retains modal meaning: finish Insert/Visual first, then Normal Escape
  clears extras and occurrence-selection state. Do not steal Ctrl-D's existing
  Vim/picker behavior or `gc` for this feature.
- Show selection and affected-file counts. Render every selected range and the
  primary caret clearly; a successful edit should not reset focus to the title.

### Semantics that must be explicit

1. Word seed uses the existing Vim word-classification/resolution contract;
   selected-text seed is literal, not regex interpretation of punctuation.
   Refuse an empty seed clearly. Do not create a second word parser.
2. Scope is the current buffer, or **all editable excerpt spans in this
   collection**, not just the viewport and not unseen whole-file content.
   Headers, gaps, path labels and hint rows cannot match. Project-wide expansion
   is a separate explicit query/collection operation.
3. Traverse in displayed source/excerpt order, wrap at most once, and dedupe by
   source identity/range. Overlapping excerpts never apply the same edit twice.
   Exhausting candidates reports it without cycling duplicates into the set.
4. These are real selections, not only caret heads. Preserve anchor/head and
   direction through motions, text objects, `c`, `d`, `y`, insert, remapping and
   undo. Fix the primary-only visual consumers and collapsed-extra conversion;
   adding two keybindings alone is not this feature.
5. Existing scalar Vim resolver stays authoritative. Resolve per selection,
   combine/validate the resulting changes, and use the same result for preview
   and execution. Single-selection semantics remain unchanged.
6. One multiselection action is one logical edit group; `u` and redo restore
   all affected source edits and selections through §5. Read-only/conflicting
   selected sources are named before mutation; they are not silently omitted.
7. Large match enumeration uses existing owned analysis/resolution machinery,
   revision-stamped snapshots and cancellation. No regex compile/full-buffer
   materialization/whole-project scan on every keypress.

First product proof: grep an identifier → collect several files → `gb`/`gB`
choose occurrences → `cNEW<esc>` → source panes agree → `u` restores all →
Ctrl-R reapplies → `:w` saves the affected sources → `g<Space>` opens the
full source at the primary caret → Ctrl-O restores the collection working set.
Run it with Unicode, repeated matches on one line, separated excerpts of one
file, overlapping context, and a read-only source. This is the milestone, not
only a unit test that counts cursors.

## 8. After the flagship: complete inspectable project changes

Reuse `editor/changes/`, but finish the original S4 promise:

- An immutable prepared proposal has identity/provenance, source bases and
  complete intended target inventory. Show a real-buffer diff review before
  rename/code-action/project-wide application, with explicit Apply/Cancel.
  Ordinary interactive collection typing remains direct editing, not a dialog.
- Source edits visibly invalidate the proposal. Do not quietly recompute from
  newer text at approval time or display a preview unrelated to the applied base.
- Resolve unopened targets through owned full-source loading. Dirty open
  documents win over disk. Preserve versionless-server uncertainty and refuse
  unsupported resource operations/external commands honestly.
- Move sorting, position conversion and expensive prepared state off input;
  publish bounded units and exact receipts. `apply_change_plan` currently
  loops through all targets synchronously despite its bounded-unit comment.
- Show refused targets and recovery actions in the review/receipt buffer, not
  only `N target(s) refused`. Keep receipts after conflicts and partial undo.
- Migrate project replace to shared proposal/receipt/application semantics while
  retaining its per-hit content checks. Do not merely rename its independent
  pipeline or weaken it to whole-buffer revision checks.
- Implement `workspace/applyEdit` only with truthful protocol response timing:
  success means actual application, not proposal enqueue. Negotiate only the
  edit/resource-operation/failure capabilities actually supported.

Primary files: `editor/changes/`, LSP editing adapters, `picker/replace.rs`,
`editor/io.rs`, mutation/history interfaces, and a source-excerpt review
renderer. Extend existing `ChangePlan.tla` and production correspondence rather
than creating another parallel change model.

## 9. Architecture and quality work worth doing now

### Finish boundaries where the product needs them

- `strop-engine` extraction is delivered. Keep it. Tighten the public surface
  around admitted actions and borrowed presentation queries as collections and
  LSP context are changed. `Editor.docs`, many ownership fields and mutable
  frontend hooks remain public; a crate move is not encapsulation.
- `render::render(&mut Editor)` still calls `refresh_hunks`; buffer rendering
  updates viewport state and analysis queries can admit jobs. Move admission to
  explicit view/edit/viewport actions, then render borrowed prepared state.
  This is 0046 stage C and its stated seam, not speculative GUI architecture.
- Split `collections/mod.rs` by actual concerns: identity/build, projection,
  edit admission, history, presentation data and tests. Keep rendering in the
  binary. Do not introduce one crate per feature or duplicate source storage.
- Keep namespace-specific paths, document revisions, service contexts and build
  identities distinct. Reuse `DocumentEdit` publication; no raw mutable-buffer
  escape for a renderer or collection adapter.
- Measure and fix the demonstrated collection preparation cost first. Keep
  input/terminal bookkeeping live; bound large result retention/retirement.
  Do not replace all channels with blocking bounded sends as a first response.

### Small quality corrections with clear value

- Finish `:explain`: actual active binding/owner, selected LSP command/root and
  why it was chosen, per-setting provenance, source/save capability, last
  proposal/receipt. Generate it from decisions, not log-string inference.
- Reuse `EX_COMMANDS` and the binding table for complete searchable command help
  and completion. Ex completion already exists; do not implement it twice.
- Arena allocation still has `generation += 1` and `slots.len() as u32`
  (`strop-core/src/id.rs:78–99`). Complete checked exhaustion in a focused change
  with a small seeded boundary test. This is an outstanding original S0 item,
  not a reproduced everyday failure or a reason to delay undo UX.
- Pin the normal build/compiler inputs deliberately: production still starts
  from floating `rust:alpine`. Keep the separate pinned Verus toolchain.
- Real container tests exist, but `tests/docker.rs::gate` returns “skip” even
  when `STROP_CONTAINER_TESTS=1` and Docker is unavailable. Explicit required
  mode must fail; wire appropriate container changes to that real gate.
- Source-size spot check found `strop-lsp/src/client/tests.rs` at 1,235 lines
  and `strop-git/src/remote.rs` at 817; split by responsibility when touching
  those areas. The roughly 980-line keymap is an intentional data-table
  exception. Do not mistake a line-count campaign for architecture improvement.

Container service continuity is a separate useful vertical slice: route its
existing LSP locations/diagnostics into real container resources and views.
Do not expand provisioning first, and do not replace the deliberate no-write
policy with `docker cp` masquerading as conditional atomic save.

## 10. Keep the good deferred ideas, in a useful order

1. **Structural selection before recipe syntax.** Once real multi-range editing
   works, add syntax expand/shrink, parent/sibling and argument/field selection
   using source-owned Tree-sitter results. Deeply exercise Rust and C++, the
   user's current language context. Incomplete code and unsupported constructs
   must be explicit; syntax spelling is not semantic identity.
2. **Save working sets, then named investigations.** Persist query/provenance,
   workspace locators, excerpt definitions and notes—not stale raw offsets,
   live server/process IDs or remote write permits. Re-resolve on restore;
   preserve the current remote-content persistence policy.
3. **Tasks and evidence.** Build a small explicit argv/cwd/execution-context
   runner whose output locations can become the same collection. Results carry
   source/environment identity and become historical after edits. No automatic
   repository hooks on open; no ad hoc log parsing presented as compiler truth.
   Reuse owned process/output machinery and the existing trust model.
4. **Selective checkpoints, structural Git review, external tools.** These
   become much more useful when they can all propose, review and reverse one
   source-backed change. They do not justify another mutation path.
5. **Dev Container lifecycle / installed service / GUI.** Retain their existing
   prerequisites and evidence gates. Do not let them displace the current
   editing experience. No GUI toolkit evaluation or prototype in this sequence.

## 11. Execution handoff and acceptance

### Ownership and sequence

- **Independent fix A:** 0048 input preservation, owned/coordinated separately.
- **Independent fix B:** §4 external-header service continuity. Serialize shared
  LSP/picker edits with 0047; do not classify its unfinished changes as regressions.
- **Multibuffer integration owner:** owns source/projection/history contracts in
  §5 and the versioned view/selection interface consumed by §§6–7.
- After that interface is explicit, visual presentation and occurrence-command
  work can proceed independently, with one owner for their shared registry and
  source-map mutations. All consumers must use the same source coordinates,
  revision checks, edit-group receipts and readonly/refusal policy.
- Finish the §5–7 product proof before calling S5 complete. Then §8 uses the
  same machinery for reviewable project changes. §9 accompanies the affected
  boundaries rather than becoming a prerequisite mega-refactor.

### Concrete first assignment

Read current 0047/0048 status; preserve the other agent's work. In an isolated
implementation worktree, replace the collection's scratch-history contract
with source-backed edit groups and projection invalidation. Start from the
exact relative-path, plain-undo and stale-grouped-undo witnesses in §3. Specify
and exercise `:w`/`:q` semantics, then land the Rootle-inspired source-excerpt
renderer and occurrence-selection workflow against that same model. Do not
stop at changing the title, adding cursor heads, or refreshing once after undo.

### Verification that earns acceptance

- Keep deterministic regressions for the demonstrated path/undo/refresh bugs;
  test through actual adapters and mutation/history publication, not helper
  copies or source-text assertions.
- Exercise ordinary/scalar Vim behavior unchanged, multi-range edit/undo/redo,
  two sequential edit groups, intervening source edits, source close/reopen,
  stale build/refresh delivery, duplicate excerpts, read-only sources, Unicode,
  CRLF/missing final newline and save conflicts/uncertain outcomes.
- Golden TestBackend grids **including styles** for mixed-language cards,
  protected rows, primary/secondary selections, dirty/refused/loading states,
  narrow splits and horizontal scrolling. Plain-text snapshots cannot prove
  syntax or selection contrast.
- Run the real TUI: discover collection from the picker footer, select matching
  words, edit, undo/redo, save, `g<Space>` into the full source and Ctrl-O back.
  Verify body/header mapping, Unicode/tab positions, dirty-source reuse,
  retained multiselections, viewport restoration after source edits, and
  namespace/LSP continuity. Enter on a header opens; Enter inside code still
  moves normally. Inspect emitted terminal output for geometry/escaping
  defects. No headless-only interaction path.
- Re-run the 100/1,000/2,000-excerpt fixture and ordinary `input_frame`, now with
  repeatable samples and retained-memory/high-water observations. Attribute
  input handling, preparation, publication, render and retirement separately;
  no flaky wall-clock-only CI threshold or silent result cap.
- External-header LSP acceptance includes actual clangd navigation after the
  first jump, plus deterministic namespace/context/retirement cases. Container
  capability work requires real required-mode container fixtures.
- Implementation gate: `docker compose run --build --rm test`; run `model`
  when protocol/history/ownership changes and `verify` for the accepted geometry
  boundary, maintaining current CI requirements. The actual completed commands
  and limitations go in the implementation record; do not reuse this audit's
  observations as proof of a new implementation.
- Update 0013, 0041, 0043, 0044, 0046 and relevant help/changelog where their
  contracts change. Remove obsolete paths/comments in the same cutover; keep
  temporary Python experiments outside the repository's Rust test stack.

## References

- [0041 adoption](0041-handoff-adoption-and-roadmap.md),
  [0043 changes](0043-change-plans.md), [0044 collections](0044-editable-collections.md),
  [0045 proof](0045-verus-pilot.md), [0046 engine](0046-engine-extraction.md),
  [0047 navigation](0047-symbol-and-jump-pickers.md), [0048 Escape](0048-escape-input-disambiguation.md).
- [0013 multicursor](0013-multicursor.md) and [0006 verification tiers](0006-e2e-harness.md).
- [Zed multibuffers](https://zed.dev/docs/multibuffers): source editing/saving,
  occurrence selection and source navigation—the interaction benchmark, not
  permission to replace Vim grammar.
- [VSCodeVim README](https://github.com/VSCodeVim/Vim): `gb` occurrence selection
  and `gc` comment toggling. Proposed Strop `gB` is explicitly our extension.
- Rootle source paths in §6 are local implementation references, inspected
  read-only; no Rootle source or preview widget was copied into Strop.
