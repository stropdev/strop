# 0051 — Whole-editor polish and one query language

Status: **implemented for 0.29.0; release verification is recorded in §12**.
The original review baseline below remains **Strop 0.28.0**,
`c7a5ac6c9089cff654365a0376f6cf87d746db95`.

This follows 0049/0050 and incorporates the user's subsequent requirements.
The scope is the **entire existing editor experience**, not only pickers:
buffers, modeline, splits, selections, collections, navigation, replacement,
Git, documentation, prompts, settings, and loading/error/recovery flows.

The source investigation and live review made no Strop implementation changes.
The other implementation session owns those changes. Reconcile its current
branch before starting; reuse work already landed instead of implementing it
twice. Code completion has its own, explicitly later release in **0059**.

## 1. Release contract — scope is not optional

**R01–R11 are all required for the next release.** They are not a ranked menu
from which an implementing agent may select the easiest entries. Small commits
and parallel ownership are welcome; quietly shrinking the release is not.

| ID | Required outcome | Evidence required before release |
| --- | --- | --- |
| R01 | One uniform file-filter/query language in `Space f`, `Space /`, and the Find field of `Space R` | Cross-surface candidate/filter tests and a real three-surface walkthrough, including filter-only file finding |
| R02 | Parser-driven query highlighting, diagnostics, shared modal field rendering, and lightweight discoverability | Same parsed spans drive execution/highlight/suggestions; real caret/edit/paste/resize cases |
| R03 | Dotfiles/dotfolders shown by default, with separate visible hidden/ignored controls | `.github`/dotfile fixtures; ignore boundaries; toggle and query override agree in all three surfaces |
| R04 | Global replacement receives the same presentation quality and an honest review/apply/save workflow | File identities, exact before/after edits, exclusions, stale/invalid query protection, per-target outcomes; open/unopened files behave consistently |
| R05 | Multibuffer is a usable source-backed editing experience, not only painted cards | Context, correct source syntax, accurate matches, live consistent views, grouped undo/redo, save/close/source navigation, and stress evidence |
| R06 | Every existing UI family in §8 passes the common visual/interaction contract | Completed surface matrix, styled grids and representative real TUI flows; no unreviewed family marked “later” |
| R07 | Deliberate jump placement and view-restoring history | Definition/reference/picker/collection jumps center sensibly; Ctrl-O/Ctrl-I restore view state and survive resize/edits |
| R08 | Conservative indentation inference plus explicit per-buffer controls | `:tab-size`, direct numeric/Auto forms, effective setting/provenance in the modeline/selector; all consumers use the same resolved setting |
| R09 | Nonblocking, source-aware matching delimiter highlighting | `()[]{}` correctness, lexical context, stale-result rejection, source projection, offscreen behavior and no typing stall |
| R10 | Readable architecture and completed component migrations | Named domain types, the concrete overgrown modules split, no duplicate parsers/render paths, no new blocking input work |
| R11 | Integrated verification, documentation and scope accounting | Required gates, real visual evidence, updated help/config/roadmap/changelog, and an acceptance record for every R item |

### What an implementing agent must not do

- Do not call the release complete because it builds or because one screenshot
  looks good. Do not relabel missing behavior “v1,” “foundation,” “core landed,”
  or “follow-up” to evade this table.
- Do not land `language:` in grep but defer file finding or replacement.
  Do not land execution but defer highlighting, error behavior or discovery.
- Do not style file/grep results while leaving replacement on its old renderer.
  Do not finish popups while excluding the modeline or ordinary editor.
- Do not add the indentation selector while leaving conflicting effective
  widths in input, rendering and formatting.
- Do not add a brace color while retaining a wrong/non-cancellable pairing
  source, or declare collections complete from manually injected highlight spans.
- Do not defer the required module splits or hide errors with defaults, ignored
  results, warning suppression, placeholders or silent partial success.
- A real technical blocker is reported with evidence. It is not unilateral
  permission to drop an R requirement. Obtain explicit user approval for any
  reduction and record that decision before claiming release completion.

### Acceptance and deferral ledger

Maintain an entry for each R ID: owner, implementation paths, observable
behavior, failure/cancellation behavior, exercised checks/captures, remaining
limitations and status. `Not started` and `In progress` are honest; neither
means `Done`. A “done” claim needs the named behavior and evidence.

Only the explicit deferrals in §11 are authorized. Each deferred item belongs
in `plans/0028-roadmap-and-review.md` with a linked plan, reason, user impact,
re-entry condition, and the exact scope left out. “Later” without an owner or
re-entry condition is not a roadmap. Deferring an R item additionally requires
an explicit user-approved scope exception; creating a roadmap bullet alone
cannot authorize it.

## 2. Review findings — preserve progress, close the actual gaps

### Live evidence and limits

A frozen installed 0.28.0 binary was driven on real Linux PTYs, using a private
Git/C++/Rust/Python fixture and private HOME/XDG directories. Binary SHA-256:

```text
db347d0e49b6754970dc6b5d6661101d53b07e53e0d44ec5695df741c5c71398
```

The review covered ordinary source rendering, split panes, grep-to-collection,
global replace fields/exclusion/cancel, occurrence selection/change/undo,
undo history, help, explain, read-only opening, real clangd hover, Git
log→commit→diff→return, shell stdout/stderr, and remote chooser/address UI
without making a connection. The whole render tree and remaining existing
surface families were also inspected in source. Remote save/failure variants
and container states were not live-exercised in this review; implementation
must use the existing hermetic adapter fixtures for those gates.

Captures and traces: `/tmp/strop-ui-release-review/{live,live-final}/` and
`evidence-summary.json`. SVGs reconstruct actual styled cells; they are not
native Windows Terminal screenshots. The proxy font lacks some CJK glyphs,
and terminal effects such as strike-through should be checked against the
cell JSON/raw output rather than inferred from this proxy. The first control
script ended on an invalid probe command, not an editor exception; the final
session exited 0 with a complete trace. No project-wide gate was rerun for
this documentation-only investigation.

### What is materially better

- The landed multibuffer now has named file cards, original source line
  numbers, omitted-line gaps and visible query hits. It is much closer to the
  intended experience than the original anonymous scratch buffer.
- Ordinary source syntax, generated help, Git log/file/diff decoration,
  preview navigation and the newer file/grep rows have useful foundations.
  Keep good existing behavior; reviewing every surface does not mean rewriting
  every surface merely to produce a diff.
- `LineEdit` is already shared by picker fields and pending prompts. The
  problem is not absence of a modal text model; parsing, field presentation,
  caret placement and host behavior are still split across callers.

### Findings that shape this release

| Finding | Evidence / consequence |
| --- | --- |
| Collections still expose mostly matching lines, not enough surrounding code | Live cards often show a declaration/call with its body omitted. Default context and explicit expansion are required, not decorative borders alone. |
| Source-syntax projection needs end-to-end proof | Ordinary C++/Rust panes were colored; the captured mixed collection remained largely neutral. Source-analysis delivery, cache/window selection and projection must be verified through the real flow, not only seeded tests. This review does not claim a fully isolated root cause. |
| Replacement is still the old row family | `render/picker/rows.rs::render_replace_results` omits source filenames beside edit rows and has separate clipping/selection logic. Find/With are flat, unhighlighted strings. |
| Replacement persistence depends on whether the file was open | `picker/replace.rs` modifies open buffers; `io.rs::finish_open(OpenIntent::Replace)` additionally schedules a save for newly opened targets. The same visible action must not have different persistence semantics based on cache/open state. |
| Query parsing is not robust | `source/query.rs::split_query` splits whitespace, has no real quote/escape handling, consumes `-t...`/`-g...` prefixes, and can drop an incomplete flag instead of preserving its scope restriction. File finding does not use it at all. |
| Jump history cannot restore a view it never recorded | `jumps.rs` stores `(DocumentId, offset)` only; `jump_to` restores the caret but no viewport/selection context. The user's “random landing” report is accepted; this is the matching ownership gap. |
| Indentation inference has concrete limitations | `detect_indent` counts only space widths ≤8 but includes deeper lines in its denominator; ties favor a larger divisor; tab-leading lines choose width 4 unconditionally; alignment/comment/string context is not considered. The user's exact file was not captured. |
| Effective indentation is not used consistently | Editing/LSP formatting can use per-document indent; several buffer/caret/preview rendering paths still read `editor.config.tab_size`. A selector alone would not fix that split. |
| Some newly added settings are not explained truthfully | The live `:explain` buffer showed `indent_style = ?`, `indent_detect = ?`, and `auto_format = ?`. Settings/help must expose actual values and provenance. |
| View changes can carry inappropriate selection state | The captured undo browser still displayed `4×` and retained extra selection positions from its source pane. Temporary/read-only surfaces must not inherit unrelated edit selections; returning must restore the intended source state. |
| The agreed source-size discipline has already been exceeded | Measured: `render/buffer.rs` 930 lines, `editor/collections/mod.rs` 1,258, `editor/lsp.rs` 824. These are implementation responsibilities, not generated tables. R10 is mandatory. |

Do not turn these findings into a second ad hoc editor. The source document,
mutation gateway, owned jobs, typed input owner and existing presentation
seams remain the foundation.

## 3. One query language for file find, grep and replacement — R01

### Decision

Adopt a small **GitHub-inspired qualifier language**, not raw `rg` options and
not a claim of full GitHub search compatibility. The same parser/compiler and
filter meanings serve:

- `Space f`: find filenames/paths;
- `Space /`: search file contents;
- `Space R`: search contents, then propose replacements.

In-buffer `/` and `?` retain Vim search grammar. The Ex line, shell command
line, remote address and replacement value retain their own semantics. They
share the input component, not an incorrectly universal query grammar.

### Canonical examples

```text
language:rust
language:rust parser
language:rust path:src/ retry_request
language:rust glob:"src/my files/**/*.rs" text:"request-id"
language:rust -glob:"**/*-test.rs" text:"retry_request"
language:rust regex:"\brequest_(id|name)\b"
hidden:exclude ignored:exclude language:cpp text:"retry_request"
text:"-test.rs"
text:"language:rust"
text:"file[1]-draft.rs"
```

`language:rust` alone in file find must list all eligible Rust files. A
filter-only content/replace query must explain that a content expression is
needed; it must not silently search every byte or enable an empty replacement
operation.

### Vocabulary and exact meanings

| Form | Meaning |
| --- | --- |
| `language:rust` | Include the language/file-type family from one canonical in-repo metadata catalog; `rs` may be a documented value alias for `rust` |
| `path:src/` | Literal substring in the workspace-relative display/search path, not an inferred glob |
| `glob:src/**/*.rs` | Explicit portable path glob; quote a value containing spaces |
| `-language:...`, `-path:...`, `-glob:...` | Exclude those file types/paths/globs; exclusions win |
| `hidden:include` / `hidden:exclude` | Include/exclude hidden entries, subject to the separate ignore and protected-metadata policy |
| `ignored:include` / `ignored:exclude` | Explicitly include or respect ignored entries; not a write grant |
| `case:smart` / `case:sensitive` / `case:ignore` | Search-expression case behavior, resolved once and used consistently |
| `text:"..."` | One explicit literal filename/content expression |
| `regex:"..."` | One explicit supported Rust-regex expression, never shell syntax |

Same positive filter repeated: OR within that filter family. Different
families: AND. Exclusions: remove any matching candidate. Identical repeated
singleton options are harmless; conflicting `case`/visibility options are a
located error, not hidden last-one-wins behavior.

`path:` is deliberately literal and `glob:` explicitly patterned. A filename
containing `[` or `?` must not change meaning merely because a parser noticed
metacharacters. This differs from GitHub's combined `path:` behavior and must
be documented rather than described as GitHub parity.

### Search-expression rules

- Files keep **fuzzy path search** for ordinary bare input. Contents use
  **literal search by default**; regex is explicit via `regex:`. Show the
  effective mode in the field/scope summary. This is a deliberate query-language
  cutover, not an accidental change to Vim `/` search.
- A bare expression may contain multiple words. File matching uses the defined
  fuzzy-term behavior; content matching treats it as one literal phrase with
  ordinary inter-token whitespace. Quoted text preserves internal whitespace.
- A wholly quoted primary expression is a literal expression. Use `text:` for
  unambiguous exact text and `regex:` for regex, especially around operators,
  quotes, or reserved-looking names. Do not mix an explicit `text:`/`regex:`
  expression with additional free text; report the conflicting spans.
- Do not introduce implicit Boolean `AND`/`OR`/`NOT` operators in this release.
  The grammar must not delegate raw query text to another parser that assigns
  those or fuzzy metacharacters a second, undocumented meaning.
- Regex compilation/validation uses the existing Rust regex ecosystem, with
  a documented bounded supported syntax. No PCRE/shell execution escape hatch.
  File and content adapters must agree on flags, Unicode/case and match ranges.
- Replacement text remains **literal** under the current contract. `$1`,
  `language:rust`, backslashes and hyphens in With are replacement text, not
  filters or secretly supported capture substitutions. If replacement expansion
  is added later it must have its own explicit grammar shared by preview/apply.

### Quote, escape and filename contract

Qualifiers are recognized only at token boundaries outside quotes, with exact
qualifier names. A hyphen is ordinary text unless it prefixes a supported
negative qualifier. `foo-bar.rs`, `-test.rs`, `--notes.rs` and a literal `-t`
are not CLI options.

Support quoted qualifier values and literal text with spaces. Quotes are part
of the raw editable source, never discarded from the display. Within a quoted
value, escape the active quote and backslash; preserve other backslash
sequences for the literal/regex layer. Do not apply C/JSON escapes that turn
`C:\temp\file.rs` into tabs/form-feeds. Document and test single/double-quote
rules, embedded quotes, and a literal trailing backslash.

Quoted or explicit-literal text protects reserved-looking filenames such as
`language:rust`. URI schemes, drive prefixes and C++ `::` are not blindly split
into qualifiers. Unknown qualifier-shaped input such as `langauge:rust` needs
an error and correction, not silently ignored scope. Literal forms must always
provide an unambiguous way to search such text.

The old UI `-t`/`-g` parsing is removed in one cutover. Update help, demos,
examples and tests together. Do not retain a second successful grammar or an
attached-flag heuristic that consumes `-test.rs`. Old flag-looking text is text,
not a backend option; release notes and query help explain the migration.
Version saved queries/replay contracts where interpretation changes. Migrate
only unambiguous older data or refuse clearly; never reinterpret it while
claiming an equivalent replay.

### One file-selection authority

Use a compiled `FileSelectionPlan` containing the resource namespace/root,
file-type rules, path/glob predicates, visibility/ignore policy and provenance.
All three surfaces consume it. Reuse the existing language metadata via a pure
data-only view; do not build a fourth language table or query installed LSPs to
find out what `language:rust` means. Language filtering is declared file-type
classification, not an expensive semantic inspection of every file.

The first implementation can retain the supervised `rg` content backend:
select eligible native paths on an owned worker, then search bounded explicit
path batches with controlled argv. Do not obtain AND semantics by blindly
appending a collection of positive `--glob` options. Argument count/byte bounds,
empty candidate sets, cancellation and terminal bookkeeping must be real.
Use `-e <pattern> -- <paths...>` or an equivalent correctly separated argv;
never interpolate the query into a shell.

Dirty open source snapshots are authoritative for their search/replacement
presentation. Search them without saving them as a side effect. A shared
semantic query compiler and adapter conformance cases must keep literal/regex
behavior consistent between frozen ropes and disk scanning. If an in-process
backend is chosen instead, justify it against these same ownership, matching,
memory and cancellation gates; it is not permission to narrow the behavior.

Cache/reuse file catalogs and compiled filters where valid; changing only the
needle should not force avoidable catalog rebuilding. Respect the operation's
real capabilities: binary/symlink/unreadable/partial resources may have different
content-search eligibility, but the UI must name those boundaries rather than
pretend filters mean something different in each surface.

## 4. Shared input, highlighting and discovery — R02

### Reuse the model; complete the component

`strop-picker::LineEdit` already owns modal text/caret editing. Reuse it.
Introduce one shared field projection/controller contract used by picker query
fields, both replacement fields, Ex/search/pipe prompts, remote address fields,
and new setting/filter selectors.

The shared part owns:

- source text, byte-valid caret/selection and existing Vim text motions;
- one printable grapheme/display-cell layout, horizontal reveal and clipping;
- label/prompt geometry, focus/mode indication, diagnostics and suggestion list;
- explicit host actions for submit, field movement, result movement and dismiss.

A host supplies a **semantic role**: search query, replacement text, Vim search,
Ex command, shell command, remote location or plain text. This prevents a filter
parser from interpreting replacement text or an SSH URI. Do not force every
host's Enter/Tab meaning to be identical; make its ownership/action policy data.

Replace hardcoded `2 +` / `10 +` caret offsets and `.chars().count()` geometry.
The same layout must place the caret and paint styled text, including tabs,
combining marks, CJK, emoji, quotes, long input and width changes.

### One lossless parse, not two lexers

A linear span-producing lexer/parser returns tokens, a typed query, source
ranges, diagnostics and a state such as Ready/Incomplete/Invalid. Highlighting,
semantic compilation and suggestions consume that same result/revision.

Rootle's qualifier coloring is useful precedent, but
`global_search/grammar.rs::style_query` and `parse` are separate scanners.
Do not copy that split. Do not highlight with a regex while executing a
`split_whitespace` parser. The spans must cover the raw input without dropping
or rewriting characters; display sanitation is an explicit later projection.

A small explicit lexer composed with existing glob/regex engines is preferable
to a shell parser or a new Tree-sitter language for this bounded grammar.
`globset`, `regex`, `regex-syntax`, `shlex` and `winnow` already appear in the
lockfile, but transitive availability is not an API contract: declare any direct
dependency deliberately. A tokenizer/combinator is not an off-the-shelf Strop
query language or a guarantee of correct incremental highlighting.

Highlight roles: qualifier key, punctuation, value, literal, regex, negation,
incomplete construct and error. Use the common palette, not filled chips that
move the caret. Quoted `"language:rust"` must not be colored as an active filter.

Incomplete typing is normal. `language:` and an unclosed quote are neither a
crash nor permission to omit the restriction. Keep previous results only as
visibly stale/non-actionable results; disable accepting a new operation and
all mutation actions until the current query is valid. Never silently broaden
search/replace while the user finishes a token.

### Lightweight discovery, required now

- Empty-field examples show the actual vocabulary (`language:rust`, `path:`,
  `glob:`, literal/regex forms), not obsolete `-t` hints.
- **Ctrl-Space in a query field** opens a small manual suggestion list for
  qualifier names, language names/aliases and enumerated values. It uses static
  metadata/the current parse position, not LSP or a new filesystem scan.
- Suggestions replace the exact token/value span and escape/quote inserted
  values correctly. They must not overwrite text after the caret or the rest
  of the query.
- Preserve Tab's field/result behavior when suggestions are closed. While the
  suggestion list owns focus, its accept/cancel keys are explicit; Escape first
  dismisses that list, then returns to the host's established modal behavior.
- A compact summary shows **effective** filters and their provenance, including
  defaults/session overrides. It is derived from the same compiled plan.
- `:help query` documents syntax, examples, supported languages and literal
  escaping. Errors point to a token and name valid alternatives.

This is query assistance, not the later code-completion subsystem. Disabling
code completion in 0059 must not disable manual query help/suggestions.

## 5. Hidden and ignored files — R03

Default policy:

```toml
[search]
show_hidden = true
respect_ignore_files = true
```

- Show unignored dotfiles/dotfolders by default, including useful project
  configuration such as `.github/`. The current hardcoded hidden exclusion in
  `WalkBuilder` and `rg` defaults must not survive as a different hidden policy.
- Hidden and ignored are separate concepts. Showing hidden entries does not
  imply including ignored build output or ignored secrets.
- Unignored `.env` files are included under this policy; do not claim that
  filenames constitute a secrets policy. No search content is uploaded, and
  existing content-capture/privacy rules remain intact.
- Repository internals such as `.git/` stay outside ordinary project search,
  even with `ignored:include`. Explicit file opening is a separate intent.
- Provide visible Hidden/Ignore controls through the shared search-options
  presentation, also reachable by `:search-options`. Query qualifiers override
  session/workspace defaults; show where the effective value came from.
- The three search surfaces share the same preference owner. Do not store
  three independent hidden booleans that drift. A text query override remains
  explicit and scoped to that query; a workspace toggle affects its defaults.
- Resolve ignore rules once with the canonical file selection, including the
  supported `.gitignore`/`.ignore`/`.rgignore` policy. Do not let file finding
  and recursive grep silently consult different rule sets.
- Visibility changes invalidate query results, exclusions and pending replace
  proposals coherently. A stale proposal cannot apply under a changed scope.
- Existing remote/container directory UIs should expose the same hidden-entry
  concept where supported and state backend-specific ignore limitations.
  Never apply client-local ignore files to a remote namespace by accident.

## 6. Replacement and multibuffer completion — R04/R05

### Global replacement gets the shared result/presentation path

Remove the old private replacement row behavior. Reuse the source-hit/excerpt
presentation with an optional replacement delta:

```text
Replace in workspace                                  12 matches · 8 files
Find   language:rust text:"retry_request"
With   dispatch_request                               literal replacement
Scope  Rust · hidden on · ignored off

[x] query-parser.rs                                   src/search
  5  - pub fn retry_request(attempt: usize) -> bool {
     + pub fn dispatch_request(attempt: usize) -> bool {

[ ] request-tests.rs                                  tests · excluded
  5  - assert!(retry_request(1));
     + assert!(dispatch_request(1));

Enter review · Tab field · Ctrl-X match · Ctrl-D file · Esc cancel/back
```

This is a schematic, not a screenshot or an instruction to add decorative
boxes around every hit. Use the common diff roles, source numbers, readable
filename/context, exact match/replacement spans and a full selected logical
block. Exclusion is not an error: use clear neutral inclusion state, not the
same red treatment as a failed operation.

- Find uses the shared parser/highlighter; With stays literal. Editing either
  value, the filter scope or exclusions invalidates the prepared operation.
- Preserve per-hit content witnesses and the preview/apply shared replacement
  function. Do not weaken them while adopting change plans.
- **Enter proceeds to a prepared review**, not an accidental bulk write from an
  incomplete text field. Reuse the existing change-review flow and commands.
  The review identifies the complete intended file/match set and refusals.
- Apply, Save and Cancel are distinct. Apply creates dirty source edits through
  the shared gateway/receipt model; it must not save only those files that
  happened to be unopened. Add a scoped **Save changed files** action (for
  example `:save-change`) using the existing per-document save contracts.
- Dirty open buffers win over disk. Unopened targets load through owned jobs.
  Stale/readonly/failed targets are visible; no silent omission or false global
  atomicity claim. Preflight, application and retirement remain bounded.
- Save outcomes are per file. Format-on-save is a named stage with its own
  result; a formatter refusal followed by a successful write must not lose the
  warning or claim that formatting succeeded. Remote uncertainty stays explicit.
- Cancelling review returns to the saved query/replacement/exclusion working
  context without mutation. Accepted groups retain exact receipts for undo and
  recovery. Do not discard a receipt just because an inverse conflicts.

### Finish the source-backed collection experience

The card paint already landed; do not rebuild it as an unrelated widget. R05
requires the remaining observable behavior from 0049 to be demonstrated:

- Include useful context around hits by default; merge overlapping context.
  Provide explicit expansion/contraction and a clear omitted-line action.
  A single matched declaration without its body is not enough reading context.
- Source syntax must survive actual async delivery and projection, including
  excerpts starting inside multiline strings/comments and language injections.
  Aggregate visible source requests; do not request/parse independently per row.
- Keep match counts separate from excerpt/line counts. Multiple matches on one
  line must not be lost through line deduplication.
- Update every view of a source while typing, with the ordinary Insert undo
  group kept open. The old action-boundary-only limitation is not a quiet
  deferral for this release. Choose a bounded implementation that preserves
  source ownership; do not disguise stale source text as live content.
- Cards/gutters/header widths are display-cell based; chrome is not editable
  source data and does not horizontally scroll like code. Preserve precise
  source positions, language, namespace, readonly/save state and selection.
- Scoped `u`/Ctrl-R, `:w`, `:q`, `g<Space>` and return navigation remain correct
  after context expansion, edits in another pane, resize and source closure.
- Complete source/projection lookup and invalidation work needed for this
  behavior. A generic transport rewrite is not mandatory, but the observable
  behavior and measured work bounds are; “the rewrite is later” is not an excuse.

## 7. Navigation, indentation and matching delimiters — R07/R08/R09

### Deliberate jump placement

Use one explicit policy, not scattered `scroll_to_cursor`/centering calls:

| Intent | Placement |
| --- | --- |
| New definition/reference/symbol/grep/mark/collection-source destination | Put the target near vertical center, respecting file bounds; preserve a comfortable already-visible view rather than introduce needless jitter |
| Ctrl-O/Ctrl-I and returning from a temporary surface | Restore prior caret, viewport, horizontal origin, selection and relevant source/service context |
| Saved view invalid after edit/resize/context change | Remap/clamp it; keep the target visible, centering if the saved view is no longer meaningful |
| Ordinary small motions/typing | Stable scrolling with a modest context margin; no recenter on every key |
| Explicit `zt`/`zz`/`zb` | Honor the user's placement command |
| Passive pair/diagnostic/highlight update | Never scroll or steal focus |

Store a real named navigation/view record, not only `(DocumentId, offset)`.
Reuse existing return-point/projection machinery where applicable. Compute
placement against the destination pane's geometry before its first committed
frame, not first reveal then jump again after syntax arrives. History entries
must not restore dead document IDs or unrelated old row numbers.

The user reported poor landing placement; this review inspected the matching
missing viewport state, not the user's exact navigation session. Neovim's
`jumpoptions=view` and `scrolloff` demonstrate the distinction between restoring
a prior view and keeping context; this plan does not claim that every editor
centers every jump by default.

### Indentation that can be understood and overridden

Required commands:

- **`:tab-size`** opens a compact selector showing current width/style and
  provenance; offer common widths plus a validated custom number and Auto.
- **`:tab-size 4`** sets a per-buffer width override.
- **`:tab-size auto`** clears the width override and resolves it from the
  configured/detection policy.
- Provide the corresponding style choice (`Spaces` / `Tabs` / `Auto`) through
  the same selector and a discoverable `:indent-style` command.

Changing these settings changes rendering/new indentation, **not existing
file bytes**. Whitespace conversion, if later added, is an explicit reviewed
text edit, not a side effect of choosing a width.

Keep existing flat fallback config keys valid. Runtime resolution is per
source document: manual property override → confident permitted detection →
configured fallback. Disabling detection uses configured fallback. A detected
Tab style does not determine a display width; do not hardcode 4 as an inferred
fact. Record provenance separately when style and width have different sources.

The modeline shows the effective `Spaces:4` / `Tabs:4`, with the selector or
`:explain` exposing Manual/Detected/Configured, confidence and ambiguity.
Keyboard access is required; clickable modeline support is an explicitly
separate optional pointer enhancement (§11), not a fake mouse affordance.

Replace the current fragile evidence model:

- Separate actual indentation evidence from continuation alignment, blank
  lines, comments/strings, mixed whitespace and deeply nested indentation.
- Do not count deep lines in a denominator while omitting their evidence, or
  resolve ambiguous divisor ties by confidently choosing the largest width.
- Bound sampling by bytes as well as lines. Low evidence means Unknown and a
  configured fallback, not a fabricated guess. Use source syntax facts where
  useful without making file opening wait on an unbounded parse.
- Prepare detection off the interactive path. A late detection result cannot
  overwrite a manual choice or change the indentation under an active Insert
  session; offer an explicit adoption when the effective choice is already in use.
- Validate widths as positive bounded values (initial supported manual range
  1–16, including 3); reject invalid config/command input visibly rather than
  allowing zero or an enormous allocation on Tab.
- One resolved per-document setting feeds insertion, Tab, autoindent, shifts,
  alignment, hard-tab display, caret/hscroll, guides, previews and formatter
  requests. Collections use each source's settings, not the projection's fallback.
- Opening another buffer, a preview or an output surface must not change this
  document's override. Reload/config refresh preserves explicit overrides.
  Session persistence may store override metadata under the existing private,
  versioned policy, never source content/permissions merely to remember a width.

### Matching brace/paren/bracket highlighting

This ships with the polish release, not the later completion release.

- Highlight the matching `()`, `[]`, `{}` pair when the active caret is on a
  delimiter, and the relevant just-typed delimiter in Insert mode.
- Use the real source matching authority. Current `match_pair` is a raw byte
  scan, includes angle brackets unconditionally and has no cancellation hook.
  Correct its fidelity/work-bound problems rather than bolt on a second matcher.
- Keep grammar pure. Source lexical facts can be immutable revision-bound data
  from the analysis worker; `%`, operators and the highlight must not disagree
  because the renderer privately interprets strings/comments differently.
- Respect lexical context, escapes, nesting and incomplete code. Do not assume
  every comment delimiter should be skipped or every `<` is a template bracket.
  The Neovim differential corpus arbitrates actual Vim semantics before changing
  `%`; `<>` are not universal matching delimiters merely because C++ has templates.
- Pairing in a collection happens in its underlying source. Map visible endpoints
  back to any valid excerpts of **that same source**; never pair across unrelated
  files, gaps or generated headers. An offscreen partner does not scroll the view.
- Use a quiet dedicated overlay that coexists with syntax/search/selection;
  unmatched/unknown and offscreen states are not stale previous-pair highlights.
- Invalidate immediately on caret/edit/mode/view change. Expensive matching runs
  in an owned cancellable job; late results recheck source revision, caret,
  view and projection identity. No whole-file scan per frame/keystroke or await
  on input. Extend scan cancellation itself, not only the delivery guard.

## 8. Whole-editor visual and interaction contract — R06

### Common rules

A single palette/role vocabulary, display-cell geometry and field/card/list
primitives should serve the editor. Distinguish primary text, readable context,
quiet chrome, focused selection, query evidence, changes, warnings and failures.
Kind color is not focus; exclusion is not failure; a dirty buffer is not the
same thing as an unstaged Git change.

Use the existing typed input owner to keep rendering and keyboard precedence
consistent. An unrelated late hover/blame reply must not paint over a modal
query or consume its next character. One documented layer/focus matrix governs
popup visibility, nested query suggestions, dismissal and restored focus.

The modeline describes the **effective input owner/mode** and active target.
While typing in a query, do not leave a contradictory undifferentiated NORMAL
label as the only global state. Preserve the underlying buffer's semantic mode;
this is presentation of ownership, not a new editor mode.

Reserve essential information before decoration: current mode/owner, target,
unsaved/readonly/partial/unconfirmed state, selection count when editing multiple
ranges, effective indentation and position. Under width pressure, shorten
branch/directory/percentage/secondary prose before silently removing safety
state. Degenerate widths get an honest compact fallback and a path to details.

Split panes need visible active/inactive identity. Add a restrained per-pane
identity strip when split, using the common layout so content/caret geometry
moves together. Do not repeat a loud full modeline around every pane. Temporary
surfaces do not inherit source selections; returning restores the correct view.

### Required coverage matrix

Every row is inspected and evidenced. Existing good behavior can remain unchanged
when it passes; inspection/evidence cannot be deferred. Apply only meaningful
states, not a useless Cartesian test explosion.

| Family | Required release treatment |
| --- | --- |
| Ordinary/untitled source buffers | Correct source syntax and injections, tabs/Unicode/long-line geometry, restrained gutters/guides, readonly/dirty feedback, no stale cells after edits or resize |
| Panes/splits | Clear identity and active focus, independent source selections and horizontal/vertical views, no selection leakage between unrelated documents |
| Modeline | Effective owner/mode, source/workspace identity, essential safety states, selection count, indentation/provenance access, consistent position domain and width priorities |
| Selections/operator/search previews | Every selected range visible; exact shared resolved operation; explicit overlay precedence; scalar Vim behavior unchanged |
| Collections | R05 complete, including context, source syntax, accurate counts, source coordinates, live views, grouping and return behavior |
| Files/buffers/jumps/symbols pickers | Keep 0050 hierarchy/alignment; preserve disambiguating context, valid counts, whole selected logical rows, stable selection and responsive width allocation |
| Grep/locations/diagnostics pickers | Exact source evidence, source identity/location, severity distinct from match/focus, named empty/loading/error states |
| Global replace | R04; no private old renderer, missing file names, implicit persistence differences or mutation under incomplete queries |
| Query/Ex/Vim-search/pipe/address fields | Shared field geometry/mode/caret; semantic role-specific parsing; no duplicate highlight parser or hardcoded label offsets |
| Which-key, marks and registers | One command registry; truthful live/unsupported status, whole hints, overflow count/paging rather than silently omitted actions |
| Help and explain | Searchable real buffers, current commands/settings, actual values/provenance and findable refusal/config errors; no `?` placeholders |
| Hover/documentation | Source-aware fenced-code/markup presentation, readable wrapping, explicit long-content access, no late focus theft or raw format artifacts |
| Blame card/gutter | Clear historical/uncommitted identity and context, fitting summaries, appropriate loading/failure state, consistent focus and return |
| Git log/changed-files/diff/sidebar | Preserve good typed decoration; historical SHA rather than misleading current branch, clear sidebar focus, exact source numbers/change roles, loading/error/return behavior |
| Undo tree | Current history state and selected restore target visually distinct, readable branches, no inherited edit selections, correct source/view restoration and stale-origin refusal |
| Proposal/review/receipt buffers | Shared diff styles and source identities, clear Ready/Stale/Applying/Applied/Cancelled/Partial outcomes, visible per-target refusals and recovery actions |
| Shell/output buffers | Distinguish stdout/stderr and completion/failure/cancelled outcomes, useful short title plus full command/cwd details, bounded output, no unexpected focus switch |
| Remote destinations/address/directory | Consistent fields and hints, readable provenance/permissions/unknown metadata, visible hidden policy where supported, honest load/error/cancel outcomes |
| Remote full/range/tail/follow/save/verify | Persistent namespace, partial/follow/readonly/write-authority and unconfirmed-save state; no local fallback or stale success; retained user edits on failure |
| Container listing/source/feedback | Same directory/picker vocabulary, visible container identity and real supported capabilities; honest readonly/refusal/loading states, no new backend promises |
| Config/trust/startup/recovery | Errors are findable after the transient message disappears; name the failed stage and remedy; no fake “healthy” setting values or unowned cleanup |
| Welcome/empty/no-result states | Current useful examples/actions, no advertising unimplemented commands, no blank unexplained modal card |
| Search options and indentation selector | Shared compact selector/field presentation, real setting values and provenance, accessible keyboard controls, no fake clickable controls |
| Matching delimiters | R09; precise passive source overlay, never a new focus owner or viewport mover |

For every applicable family inspect normal/focused/inactive, narrow/wide/resize,
empty/loading/failure/cancel/stale, readonly/dirty/partial and return-to-origin
behavior. Successful happy-path paint alone is not acceptance.

## 9. Architecture and code-quality requirements — R10

The user explicitly values readable maintainable code over terse patches.
Optimize the compiled path and the next reader's understanding, not token count.

### Shared boundaries

- One query lexer/parser/compiler; one field presentation/controller; one
  source-hit/excerpt/diff presentation seam where semantics are shared.
- Retain domain distinctions: query source byte offsets, matcher indices,
  terminal cells, source document positions, resource identities, revisions,
  service owners and view history are not interchangeable integers/strings.
- Descriptive names such as `SearchQuery`, `QueryDiagnostic`,
  `FileSelectionPlan`, `SourceMatch`, `CollectionExcerptSpan`,
  `NavigationLocation` and `IndentationResolution`. Do not add `Q`, `Ctx`,
  `Mgr`, `DocHits` or deeply nested tuple aliases in place of known domains.
  Ordinary local loop indices do not need ceremonial wrappers.
- Keep input→render synchronous and bounded. Parsing a short query once per
  revision is different from compiling globs/regex, walking a project, parsing
  documents or flattening a rope during paint. Those expensive steps are jobs.
- Prepare source analysis/projection per visible source window, not one job or
  an all-excerpt scan per painted row. Published immutable data is borrowed by
  rendering. Include final-drop/retirement cost, not only preparation time.
- Move render-side job admission toward explicit input/view/viewport preparation
  actions. Do not solve another feature by exposing more mutable `Editor` state
  to renderers or adding a competing global UI manager.
- Reuse the existing gateway, journals, job/ticket cancellation, registry and
  replay seam. Complete migrations, remove obsolete production paths, preserve
  owned terminal bookkeeping and truthful typed failures.

### Required responsibility splits

At the reviewed baseline these modules exceed the repository's approximate
800-line ceiling and must not grow further under this release:

- `editor/collections/mod.rs` (1,258): split model/build, projection, editing,
  history/save, navigation and tests as the actual responsibilities warrant.
- `render/buffer.rs` (930): split pane/viewport geometry, line/overlay rendering
  and typed source/collection decoration. Keep one underlying renderer.
- `editor/lsp.rs` (824): split reply routing/navigation/presentation domains;
  future completion belongs in its own subsystem, not appended here.

`picker/rows.rs` (641), `editor/io.rs` (near the ceiling), and change review
also need discipline as they gain work. Split by concern, not every N lines;
do not create empty scaffold modules. Test files follow the same readability
principle. Data-only keymap tables remain the legitimate single-pattern exception.

The old `source/query.rs` mixes UI query parsing with `rg` JSON decoding. Move
those responsibilities apart. A reasonable query directory has lexical tokens,
parsing/diagnostics, semantic compilation/filter planning and tests; highlighting
roles come from that parse, not another scanner. Use actual code size to decide
file boundaries rather than preallocating empty files.

Settings metadata/help/selectors must obtain real values from one typed
setting description/access path. No new wildcard match returning `"?"` for
settings added elsewhere. Validate values at boundaries and surface real I/O,
parse, capability, conflict and cancellation errors. Do not suppress warnings or
paper over a failed assumption with a default-to-local/default-to-success path.

## 10. Implementation sequence and verification — R11

### Owners and integration

One integration owner settles query/field/source-presentation/view-setting
contracts. Independent owners can then work on query/backend convergence,
replacement/multibuffer, ordinary editor/modeline/navigation, and auxiliary
surfaces. Serialize shared mutations in the item/command/settings registries.
No sibling validates a half-integrated tree; run the combined gates after the
interfaces and callers are migrated.

Suggested complete increments, all inside the release:

1. Freeze baseline evidence; split the overgrown responsibilities being touched;
   define typed query/field/view/setting contracts and the acceptance ledger.
2. Land R01–R03 together across all three surfaces, with highlighted/discoverable
   input and guarded incomplete queries. No single-surface launch.
3. Land replacement review/receipt/persistence convergence and complete source
   collection/context/syntax/live-view behavior.
4. Land navigation placement, indentation controls/consumers and delimiter
   overlay against those source/view contracts.
5. Apply/verify the common roles, layout, focus, states and return behavior across
   the entire §8 matrix; keep good existing designs where they pass.
6. Run integrated real-surface and required code/model/proof gates; finish the
   ledger and roadmap before release claims.

### Essential query and scope cases

- Rust filter-only file finding; identical language/path/glob/visibility
  predicates across file find, grep and replace.
- Leading/interior hyphens, literal `-t`, spaces, embedded quotes, colon-looking
  qualifiers, backslashes/drive-like strings, Unicode/combining/CJK names,
  literal glob metacharacters and multiple submatches per source line.
- Quoted values and regex escapes retain exact meaning; replacement values do
  not become filters or unsupported capture expansions.
- Incomplete key/value/quote, unknown language/qualifier, invalid glob/regex,
  conflicting options and a query edit while results/review are pending.
- Hidden shown by default; hidden off; ignored explicitly included; ignored
  secrets still excluded by default; protected `.git` metadata not searched.
- No shell interpolation, option injection or native-path reconstruction from
  display strings. Versioned saved-query/replay migration is explicit.

### Essential editor/flow cases

- Multilanguage collections with surrounding context; excerpts inside multiline
  comments/strings; matching source rows after edits/undo; live split views;
  accurate match counts; context expansion; source jump and return.
- Global replacement with open and unopened, dirty, readonly, stale and failed
  targets; all exclusions and outcomes visible; Cancel changes nothing; Apply
  versus Save consistent; save while typing retains newer dirty edits.
- Definition→external header→back/forward, symbol/grep/mark jumps, resized panes,
  horizontal scroll, edited source anchors, closed documents and temporary
  surfaces. No selection leakage into unrelated output/help/history buffers.
- Indentation at 2/3/4/8, tabs with configured non-4 width, deep indentation,
  alignment/comments/strings, mixed/low evidence, overrides/reload/config changes,
  multiple buffers and collection sources with different settings.
- Pair highlighting inside source context, unmatched pairs, escapes, incomplete
  code, offscreen mates, stale jobs and large buffers. Differential fidelity
  protects `%`; UI overlay tests protect actual painted positions.

### Proof of completion

- Focused deterministic regressions through real parsing/admission/mutation/
  publication interfaces; source text checks and mock echoes are not behavior.
- Styled TestBackend grids (foreground/background/modifiers/cursor), repeated
  resize on the same terminal, and emitted VT behavior for Unicode/control
  safety. Text-only screenshots do not prove syntax or selection quality.
- Real TUI flows at 80×24, 100×30, 140×40 and constrained splits. Exercise input,
  selection, cancel, review, save, failure and return—not only open a popup.
- Measure representative large files/lines, many results/excerpts, rapid query
  edits and retirement while typing. Report preparation/publication/render,
  p50/p95/p99/max and retained memory separately on a recorded build/machine.
  No performance claim from a single warm frame, and no flaky noisy CI timer
  in place of deterministic work/fairness bounds.
- `docker compose run --build --rm test` is required for implementation. Preserve
  current CI `model`/`verify` gates and run affected protocol/proof boundaries;
  adapter changes use the required real SSH/container fixtures. Missing tools
  are unverified/failed gates, not silent successful skips.
- Update 0003, 0013, 0032, 0044, 0047, 0049, 0050 and the roadmap only where
  contracts/status actually change; update command/config help and the existing
  changelog. Keep investigation scripts/artifacts outside the source tree.

## 11. Explicitly allowed deferrals — record these on the roadmap

| ID | May be deferred from this release | Re-entry condition / boundary |
| --- | --- | --- |
| D01 | Multi-source **code** completion | Separate release 0059, after architecture, core verification and native worker, explicitly authorized by the user; LSP/current-buffer sources, nonblocking lifecycle and config gates belong there. This does **not** defer R02 query suggestions. |
| D02 | Full Boolean/GitHub search language, semantic `symbol:` predicates and arbitrary provider qualifiers | A concrete workflow beyond the bounded language above, after R01/R02 parity and diagnostics are stable. Do not advertise unsupported forms now. |
| D03 | Clickable modeline/general mouse interaction | A coherent pointer/hit-region and terminal-capture contract. Keyboard `:tab-size`, the selector and visible effective setting are required now. Do not enable mouse capture only to discard most mouse input. |
| D04 | General theme engine, experimental terminal typography protocols, GUI and new backend provisioning | Existing roadmap prerequisites/evidence gates; none is needed to deliver this TUI polish. Use the existing palette/layout first. |
| D05 | Completion extras outside the bounded 0059 release, such as rich snippets or additional providers | The explicit 0059 extension ledger, after its required safe two-source completion is complete. |

Previously deferred broader work (named investigations, structural recipes,
Dev Container lifecycle, installed remote service) keeps its existing roadmap
contract. It is not a reason to postpone the existing UI families' quality.

**Nothing in this table authorizes dropping R01–R11.** If an implementation
report says “not done,” it must either fail release acceptance or cite an
explicit user-approved exception with its roadmap record.

## 12. Implementation and acceptance — 0.29.0

Integration owner: Main. Scope is R01–R11; no R item was moved to the deferral
ledger. 0053 is the next separate implementation/release, not part of this cut.

| ID | Implementation and exercised contract |
| --- | --- |
| R01 | `strop-core/languages.rs`, `strop-picker/query/`, shared selection/grep adapters and engine picker query control. Parser/compiler, cross-surface filtering, dirty snapshots and literal replacement regressions; real filter-only C++ Files and the same C++ grep/replace query. |
| R02 | Shared `LineEdit`/field projection, parser spans, located worker diagnostics and manual suggestions. Actual Ctrl-Space acceptance, bracketed Unicode paste, field modes, 30×8 recovery and 80/100/140-column captures; non-query fields do not acquire query grammar. |
| R03 | One local visibility policy, hidden-on default, ignore-file tests, protected repository metadata, overrides and search-options UI. Actual `.github` discovery; SSH/container listings identify their fixed hidden-on/no-ignore policy without applying local rules. |
| R04 | Prepared review/rendering, full-line match witnesses, review revision guard and owned per-file save receipts. Open/unopened/dirty/stale/readonly cases, literal values, cancellation restoration and real Apply→Save disk readback. Formatter warning and remote uncertainty paths remain distinct. |
| R05 | Collection model/projection/editing/journal/context/history/navigation modules. Real mixed source-colored C++ collection, context/source round trip and live source/collection split editing. Source authority, grouped history, late-load focus, missing final newline and remote save/close regressions. |
| R06 | Shared field/chrome/diff roles and input-owner modeline. The family matrix below includes unchanged good surfaces; failures, incomplete results and unavailable capabilities remain named rather than painted as success. |
| R07 | One full `JumpRecord` for jump history and temporary returns, including selections and byte-anchored viewport. Source journals remap retained records. Real help/documentation/Git/source returns and deterministic edited/resized/closed-origin coverage. |
| R08 | Conservative source detection, manual width/style overrides, captured selector owner and shared effective settings. Widths/ambiguity/readonly source projection regressions; actual split-source width 3, selector and Auto, with bytes unchanged. |
| R09 | Cancellable grammar delimiter resolver shared by motions and passive source overlays. Differential `%`, quoted/escaped/unmatched cases, source projection, stale jobs and actual painted-cell regressions. |
| R10 | Production responsibilities and the two oversized LSP regression roots are split. The command table is the only Rust file above 800 lines. Ranking admission coalesces while one snapshot is in flight; no stream-sized queue of catalog snapshots remains. |
| R11 | Command/config help, README, current collection/navigation/modeline contracts and changelog updated. Required compose, proof/model and real-adapter commands are the release gate; measurements and capture limits are stated below. |

### Surface coverage matrix

| Family | Evidence |
| --- | --- |
| Ordinary/untitled source | Actual 80/140-column C++ source; terminal/headless and styled-grid Unicode, tabs, long-line and welcome coverage. |
| Panes/splits | Actual live source/collection split, independent identity strips, close/return; selection and geometry regressions. |
| Modeline | Actual Files/Replace Insert/Normal, source indentation and readonly receipts; narrow safety-state grids. |
| Selections/operator/search previews | Shared-resolver/differential and styled overlay grids; actual named-register yank and source editing. |
| Collections | Actual source syntax, matches, context, split publication and return; source/save/history/ownership regressions. |
| Files/buffers/jumps/symbols pickers | Actual Files, jumps and clangd symbols; per-kind row, selection, empty/error and clipping grids. |
| Grep/locations/diagnostics | Actual grep and source opening; severity/location/ownership grids and server reply tests. |
| Global replace | Actual delta/review/Cancel/Apply/Save and disk readback; stale/readonly/failed/closed-target regressions. |
| Query/Ex/search/pipe/address fields | Actual suggestions, Ex/search, Unicode bracketed paste and address cancellation; shared field and pipe regression coverage. |
| Which-key/marks/registers | Actual mark card and named-register prefix/yank; registry coverage, overflow and modal dispatch regressions. |
| Help/explain | Actual query help and effective config values, source return; typed settings and persistent diagnostics coverage. |
| Hover/documentation | Actual clangd hover and real documentation buffer; late/stale/Insert ownership and markup grids. |
| Blame card/gutter | Actual historical/uncommitted gutter; card, source identity, stale refresh and return regressions. |
| Git log/files/diff/sidebar | Actual complete dive and return chain, sidebar focus; native-path and styled-grid regressions. |
| Undo tree | Actual edit→browser→return→undo; selection isolation and stale-origin/history authority tests. |
| Proposal/review/receipts | Actual prepared diff, cancellation, applied and persistence receipts; partial/refused/stale metadata coverage. |
| Shell/output | Actual stdout/stderr, exit-7 failure metadata and return; cancellation, late focus and partial-output tests. |
| Remote destinations/address/directory | Actual chooser/Add-host/address paste; real SSH directory fixtures and unknown/zero/native metadata grids. |
| Remote full/range/tail/follow/save/verify | Required real SSH suite plus engine receipt, cancellation, readonly/partial/uncertain-state regressions. No enterprise NFS run is claimed here. |
| Container listing/source/feedback | Real container inspect/list/read/stale-identity fixtures; frontend readonly/namespace/refusal coverage. No new container mutation capability. |
| Config/trust/startup/recovery | Actual explain/clangd readiness; persisted diagnostics, trust, startup, session and replay tests. |
| Welcome/empty/no-result | Actual empty/invalid query states; welcome and no-result grids, role-correct hints and visible failures. |
| Search options/indentation | Actual options, source-specific selector and direct/Auto width; precedence, bounds and captured-owner tests. |
| Matching delimiters | Shared grammar/differential and styled pair-cell tests, including source projection and stale rejection. |

### Measurements and evidence limits

Controlled PTY fixtures and actual styled cell captures live under
`/tmp/strop-release-029-1ardhx4j/integrated-{b,c,e,g}` on the verification host.
SVGs are projections of the emitted cell grid, not native Windows Terminal images.
The proxy font lacks some CJK glyphs; cell occupancy and original UTF-8 are retained.
Some preliminary driver runs aborted on probe assumptions (batched Escape, covered
wide cells, no-op resize or an unsupported probe command); their unfinished traces
are not claimed as successful full replays.

Release-build measurements on the recorded WSL2 Ryzen 9950X3D host. Final
0.29.0 GNU/Linux executable SHA-256:
`1b5788e32bfd809f4a9fe4ca68a2140247aeaf46cceed7b8dddc083e0d2699da`.

- 10k-line input+frame, 500 samples at 120×40: p50 0.39 ms, p95 0.49,
  p99 0.56, max 1.04; process peak RSS 11.3 MiB.
- 300 files × 50 matches, eight fresh runs: search p50/p95 114/117 ms,
  prepared review 49/51 ms, Apply+frame 20/21 ms, Save+settle 47/57 ms;
  retirement p50 2.2 ms, max 2.8 ms. Every target was read back from disk.
- A real 300-source/15,000-hit collection, 200 typed characters with traced
  frames: input→next frame p50 1.30 ms, p95 1.47, p99 1.75, max 1.80;
  traced render p50 0.40 ms, max 0.50. Undo restored the source position
  (view line 3, column 1), not the stale post-insert byte offset. The workload
  and complete metadata trace are in `/tmp/strop-collection-perf-b0ejxowb`.
- 100k results: snapshot coalescing reduced sampled peak live heap from about
  2.0 GB to 50.8 MB, with peak process RSS about 118 MiB. Stream+settle median
  was 1.08 s; settled frame median 0.44 ms. The glibc sample is diagnostic,
  not a production allocator change or a universal memory ceiling.
- 1 MiB line: frame-at-end median 0.58 ms; explicit worst-case `fZ` took up to
  59 ms. This is not a claim that every explicit motion stays below a frame budget.
- Full-content record→replay of actual selection/edit/undo, query help, shell
  output and quit completed successfully using the final executable:
  `/tmp/strop-release029-replay-88vbbfxs/full.jsonl`.

Completed local release gates: `docker compose run --build --rm test` (fmt,
locked clippy/all-targets and the complete test suite with real SSH required),
`model` (TLC), `verify` (10 edit-kernel obligations, zero errors), and
`container-test` (25 unit cases and five real-engine cases). Hosted publication
and CI are tracked by the `v0.29.0` release; they must be green before completion.
No test count or proof result is a claim that all editor defects or schedules have
been exhausted. The permitted D01–D05 deferrals remain exactly those in §11.

## References

- [0049 product/architecture handoff](0049-product-and-architecture-handoff.md),
  [0050 picker polish](0050-picker-visual-polish-handoff.md),
  [0059 completion release](0059-nonblocking-code-completion.md),
  [0028 roadmap](0028-roadmap-and-review.md).
- [GitHub code-search syntax](https://docs.github.com/en/search-github/github-code-search/understanding-github-code-search-syntax): qualifiers, quoting and explicit regex precedent; this plan deliberately defines a smaller/different language.
- [ripgrep guide](https://github.com/BurntSushi/ripgrep/blob/master/GUIDE.md): ignore/glob/type/matching behavior and argv boundaries.
- [VS Code basic editing](https://code.visualstudio.com/docs/editing/codebasics): indentation controls, scope/exclusion visibility, search and replacement presentation.
- [Helix configuration](https://docs.helix-editor.com/configuration.html) and [pickers](https://docs.helix-editor.com/pickers.html): configurable file visibility and explicit picker fields.
- [Neovim options](https://neovim.io/doc/user/options.html): `jumpoptions=view`, `scrolloff`, `sidescrolloff`; distinguish view restoration from centering.
- [Zed multibuffers](https://zed.dev/docs/multibuffers) and [outline](https://zed.dev/docs/outline-panel): source-backed reading/editing context.
- [Kitty text sizing/width discussion](https://sw.kovidgoyal.net/kitty/text-sizing-protocol/): why terminal/client width agreement matters, not a dependency to adopt in this release.
- Rootle local source: `../rust/rootle/crates/rootle/src/components/global_search/grammar.rs`, its result/preview renderers, and `doc/house-style.md`; visual precedent, not permission to duplicate parsing or copy a second editor.
