# 0063 — Canonical search: one query language, automatic workspace symbols

Status: **landed** (2026-09-16). Every section's code and verification
slices are in the tree with their evidence; the only residual obligations
are owned by [0058](0058-unified-native-worker.md) (incremental index
reuse/invalidation across searches, remote catalog/kind evidence) and by
[0057](0057-core-verification-and-assurance.md) (the TLAPS proof lane the
lifecycle model's proof obligations attach to — §6.6 landed its Verus and
trace-replay correspondence). Filesystem notifications/reconciliation stay
folded into 0058; UI polish lives in
[0064](0064-ui-polish-scrollbars-and-cursor-fade.md). Per the user's
2026-09-16 direction, completion (0059), debugger (0060), GUI (0061) and
distribution (0062) are out of scope — not sequenced, not started.

This document now records executed work; each landed slice below names its
evidence.

## 1. Ownership and boundaries

| Concern | Owner |
|---|---|
| Filesystem notifications, reconciliation, guarded reload, Git/catalog invalidation and their verification | [0058 unified native worker](0058-unified-native-worker.md) — folded in there, no separate plan |
| Shared query language, Boolean operators, all query-bearing panes, typed document/workspace symbols, automatic project discovery, mixed-language providers and verification | **This plan** |
| Per-pane scrollbar/Git overview, then optional slow cursor fading | [0064 UI polish](0064-ui-polish-scrollbars-and-cursor-fade.md) |

Workspace symbols were reserved as future work in
[0047](0047-symbol-and-jump-pickers.md); that reservation is fulfilled here —
0047 did not, and does not need to, cover the automatic mixed-repository
experience described below.

## 2. Automatic symbol search

The acceptance flow: open an ordinary parent directory containing several
projects, invoke workspace symbols, and search. No repository list, no
prior file-opening ritual, no repository/workspace mode selection.

- The opened directory is the common search scope for Files, Search and
  workspace symbols. Repository and language-project boundaries are
  discovered internally through one shared catalog and the canonical
  filesystem worker (0058). Nested repositories, Git worktrees, monorepos,
  non-Git projects and loose source directories are all recognized. No
  special behavior for any directory name — `workarea` included.
- Repository boundaries and language-server roots are different concepts. A
  repository may contain several languages or independently configured
  subprojects. Native namespace, project root, server profile,
  configuration/environment and provider incarnation are preserved in their
  identities.
- Reuse Strop's existing LSP attachment owner (it already distinguishes
  filesystem target, language and root). Extend it to initialize eligible
  unopened projects without creating fake buffers. Providers start lazily
  with bounded concurrency; warm sessions are reused; servers serving active
  documents are protected. Never start every installed server for every query.
- Respect each project's actual configuration: compilation information for
  C++, Python environment/configuration roots, Lua library settings. Missing
  configuration produces an actionable project status while other projects
  continue returning results.
- A bounded syntax-definition fallback covers Python, C++, Lua and Rust,
  reusing existing parser foundations with tested declaration extraction.
  It provides useful results when a semantic server is unavailable or still
  loading, and its coverage is labeled accurately: syntax extraction does
  not guarantee macro-expanded or inferred declarations.
- Indexing is incremental and outside the input/render path. Unchanged
  results are reused, unsaved sources overlay, and invalidation is driven by
  source observation, catalog generation and filesystem notifications.
  Watching improves freshness; it is never the only cache-validity mechanism.
- The picker combines results, preserves project/source provenance and
  avoids duplicate locations. Same-name symbols in different projects remain
  distinct. Loading, unavailable and truncated coverage are shown clearly; a
  partial empty result must never appear to mean "no symbols exist."

## 3. Canonical query language

Uppercase `AND`, `OR`, `NOT` and parentheses, with precedence
**NOT > AND > OR** — a familiar searchable-code convention
([GitHub code-search syntax](https://docs.github.com/en/search-github/github-code-search/understanding-github-code-search-syntax))
that preserves Strop's existing simple-query behavior.

| Example | Meaning |
|---|---|
| `(language:python OR language:cpp OR language:lua) parser` | Search the selected source-language families. |
| `(kind:class OR kind:struct) parser` | Either symbol classification. |
| `(repo:engine OR repo:tools) NOT glob:**/vendor/** parser` | Optional repository narrowing and an exclusion. |
| `text:"retry" AND text:"request" NOT text:"test"` | Both positive literals occur on the same logical line; the excluded literal does not. |

Preserved simple-clause conventions:

- Repeated positive qualifiers in one family remain OR alternatives; different
  families and exclusions combine with AND.
- Bare multiword content remains one phrase. Explicit `AND` requests separate
  predicates.
- Existing negative metadata qualifiers remain supported.
- `NOT` binds one atom or a parenthesized expression. Quote multiword operands.
- Code punctuation such as `!` and `|` stays literal outside explicit regex
  syntax.
- `kind:` is canonical; `type:` normalizes as an input alias through the same
  qualifier catalog.

Specify lexical boundaries, parentheses, quoting and escapes before
implementation. Literal URLs, namespace separators, backslashes and
negative-looking filenames survive parsing. Regex contents stay opaque to
Boolean parsing.

**One typed AST and one shared evaluator.** All accepted syntax lowers once;
no pane selects a different parser, and no separate Boolean implementation
survives beside the existing query parser.

Inventory and migrate every query-bearing surface: Files, Search, Directory,
document/workspace symbols, other search pickers and embedded search
operands. Qualifier metadata, diagnostics, completion and highlighting are
shared. Surface-specific candidate types and default matching modes are
explicit; unsupported qualifiers produce diagnostics rather than silently
disappearing.

The `With` replacement payload remains replacement text. Command syntax stays
owned by the command parser; embedded search operands delegate to the
canonical query machinery.

Stored queries carry a syntax version and migration. Previously literal
operator words and newly recognized qualifiers must not silently change
saved-query meaning. Query-wide options (`case:`, `hidden:`, `ignored:`) stay
outside Boolean branches; canonical formatting places them in a leading
preamble.

The language catalog consolidates during this work: at the 0.32.4 tag Lua
exists in syntax support but is missing from the shared core language catalog
and the default LSP registry, and C++ extension membership is inconsistent.
Resolve both centrally, with one policy for ambiguous headers.

## 4. Execution and replacement

- Boolean content expressions evaluate against complete logical source lines,
  consistently across disk results and unsaved-buffer overlays. `AND` never
  means "matches somewhere on different lines in the same file"; file-level
  conjunction would require an explicit additional feature
  ([git-grep's distinction](https://git-scm.com/docs/git-grep)).
- Branch relationships are preserved: `(repo:a AND foo) OR (repo:b AND bar)`
  cannot flatten into independent repository and text alternatives. Provider
  prefilters may overfetch; final admission uses the original AST. Parsing,
  compilation, candidate retrieval, provider fan-out and retained results are
  bounded.
- Negative terms contribute no highlights or replacement spans. Failed or
  unavailable evidence must not become false and then a successful match
  through `NOT`.
- LSP accepts a query string, not Strop's grammar
  ([workspace/symbol contract](https://microsoft.github.io/language-server-protocol/specifications/lsp/3.17/specification/#workspace_symbol)).
  Provider requests are planned and bounded; returned candidates are filtered
  locally against the AST. Coverage stays honest — server-defined retrieval
  is not guaranteed to enumerate every matching symbol.
- First replacement implementation: exactly one identifiable positive
  literal/regex target, optionally constrained by Boolean metadata and
  negative content guards. Multiple positive targets (`foo OR bar`) remain
  valid searches but replacement requires an explicit ambiguity diagnostic.
  Preview and apply share the same frozen AST, scope, source evidence and
  checked ChangePlan.

## 5. Code structure

Separate owners for query parsing/evaluation, shared language/kind metadata,
project discovery/catalog state, syntax extraction/index scheduling, LSP
lifecycle, result admission and presentation. Extend or extract the existing
canonical owner when dependencies justify it. No competing crawlers,
pane-specific parsers, duplicate language tables or proof-only
implementations; migrated callers and removed paths land together. Native
work, regex compilation, indexing and process startup stay outside
input/render.

## 6. Required verification

Verification is part of this epic's acceptance criteria:

1. **Grammar:** reviewed parse-tree/result corpus, precedence/grouping/
   negation cases, incomplete-input diagnostics, Unicode/escaping,
   parse-format-parse properties, syntax-version migration and bounded parser
   fuzzing.
2. **Semantics:** an independent reference interpreter; Boolean algebra checks
   over complete inputs; differential disk/dirty-buffer evaluation;
   branch-sensitive filtering; cross-surface consistency; provider-prefilter
   soundness.
3. **Replacement:** single-target eligibility, negative guards, rejection of
   ambiguous targets, literal replacement payloads, frozen preview/apply
   scope, stale-review rejection.
4. **Lifecycle:** extend the existing TLA+/TLC search lifecycle model or add a
   focused model in the same verification registry. Cover scope/query/provider
   generations, cancellation, popup dismissal, partial results, indexing,
   restart, invalidation, lazy resolution and resource bounds.
5. **Named safety properties:** no foreign results, no retired-query
   publication, no known-stale symbol acceptance, no cross-project
   configuration leakage, no false completeness. Progress assumptions stated
   explicitly.
6. **Production correspondence:** replay model traces through actual
   admission/decoder handlers; extend applicable Verus/TLAPS obligations and
   test real synchronization through the repository's established seam.
7. **Negative controls:** deliberately remove `NOT`, swap `AND`/`OR`, flatten
   branches, ignore qualifiers, accept stale generations and mark unopened
   projects complete. Each defect must fail its intended assertion.
8. **End to end:** open a mixed parent directory containing separate Python
   environments, C++, Lua, nested repositories, a worktree and non-Git
   sources. Obtain expected symbols without first opening files or
   configuring a repository list. Exercise missing servers, syntax fallback,
   dirty sources, cancellation, restart, Unicode locations and remote
   namespace isolation.

These checks register in the actual CI/release gates alongside the 0058
assurance work, with model bounds, assumptions, meaningful mutant failures and
native/provider coverage recorded. A skipped fixture or an unrelated green
gate is not passing evidence.

### §3 grammar and §4 execution: landed slice (2026-09-14)

The typed AST is live end to end in the one shared parser: standalone
uppercase `AND`/`OR`/`NOT` tokenize as operators (lowercase and quoted
forms stay literal content); operator-bearing queries group with
balanced-paren scoping, where `(` opens a literal span so
`glob:**/(1)/*.rs` and `foo(1).txt` stay whole and `)` only delimits at
depth zero. Precedence NOT > AND > OR, juxtaposition is an implicit
AND, and a bare-word run remains one phrase. Query-wide options
(`case:`/`hidden:`/`ignored:`) keep their flat meaning outside
branches. Every operator-free query parses byte-identically to the
previous grammar — pinned by tests.

Execution follows the §4 split: the provider pattern (rg) is the union
of positive content atoms — an overfetching prefilter — while exact
admission evaluates the original AST per (path, line) at every
consumer (rg hits, dirty-buffer snapshots, directory names): content
atoms against the complete logical line, metadata atoms against
path/extension, branches never flattened, NOT never turning unknown
evidence into a match. Negative terms contribute no highlights (the
prefilter carries only positive spans). Evidence so far: precedence/
grouping/negation corpus, literal-compatibility corpus, located
dangling-operator diagnostics, parse-format-parse over the spec
examples, same-line AND semantics, branch-preservation, prefilter
soundness over an admitted corpus, and case-mode atom behavior. Still
open from §3: stored-query syntax versioning, surface-by-surface
inventory beyond the picker pipeline, and the §6 model/fuzzing tiers.
Follow-up (2026-09-14): `kind:`/`repo:` are grammar-valid with `type:`
normalized as the documented alias. Until project discovery (§2) can
decide them they compile to an explicit Unknown that admits — overfetch
inside AND, widens OR, and neutralizes any NOT above it, so pending
qualifiers never silently drop lines — and a flat `kind:` query
explains itself instead of disappearing.

Project discovery slice (2026-09-14): one bounded catalog scan
(`source/catalog.rs`) discovers the scope's boundaries — nested Git
repositories, linked worktrees (`.git` file), marker subprojects
(Cargo.toml/pyproject.toml/setup.py/CMakeLists.txt/package.json/go.mod)
and the loose scope — deepest-enclosing lookup for `repo:` names,
bounded and cancellation-aware with an honest `truncated` flag. Admission
is now three-valued end to end: Yes/No/Unknown with Unknown admitting
(overfetch), which replaced the compile-time Unknown collapse with
runtime evidence — `NOT` over an undecidable atom admits instead of
dropping lines. `repo:` decides exactly with a catalog on the local
search path (one discovery scan per search, worker-side); remote and
no-catalog paths stay Unknown until 0058's worker owns remote
marker, loose dir), ignored-repo miss classified soundly, repo
branch-sensitivity, NOT-repo inversion, no-catalog overfetch.

Syntax-fallback slice (2026-09-14): `kind:` now decides exactly from
real evidence. `strop-syntax` gains declaration extraction over the
already-linked grammars (`queries/{rust,python,c,lua,cpp}/symbols.scm`
+ `src/symbols.rs`): kind, name and inclusive line span per
declaration, with methods recognized by member context (impl/trait,
class body, Lua method syntax) and honest gaps kept out (macros,
typedef aliases, forward declarations, inferred Python constants).
The picker builds one bounded `SymbolIndex` per search — worker-side,
only when the compiled AST consults `kind:` (ordinary queries pay
nothing) — from the selection walk's own paths (4096 files / 2 MiB per
file / 64 MiB total; beyond stays absent = Unknown). Admission's
evidence grew a `line` number and the index: a complete entry answers
Yes/No by span containment, absent/incomplete evidence stays Unknown
and admits; dirty buffers overlay their disk entry because unsaved
text is authoritative. Flat `kind:`/`repo:`/`type:` queries upgrade
to the Boolean parse as implicit ANDs — the "combine with AND"
failure is gone because narrowing now exists. `kind:function`
subsumes methods; unrecognized kind values admit and explain through
suggestions. Evidence so far: per-language extraction corpora (spans,
methods, qualified C++ names), index bounds/cancel/overlay tests,
three-valued narrowing tests (AND, NOT inversion, branch
sensitivity, unknown value), the rg-record seam passing line numbers
through `parse_json_match_with`, and suggestion listing. Still open:
LSP lazy init and the workspace-symbols picker surface (§2),
incremental cross-search reuse and invalidation (0058's worker),
stored-query versioning (§3), and the §6 tiers.

Workspace-symbols surface slice (2026-09-14): `space S` is live —
the picker lists every declaration in the opened scope through the
syntax-fallback tier, one bounded source (`run_workspace_symbols`):
the shared selection walk, the symbol index, then deterministic
path-ordered rows in the 0047 convention (`name  path · :line`, kind
chip in the badge column). Rows jump like grep hits — cursor lands on
the declaration's name via the new byte column on `Declaration`.
Coverage stays honest: `SymbolIndex::coverage_gap()` (eligible files
minus complete extractions) surfaces as a visible warning, never as
silence; non-extractor languages are out of the tier's declared
coverage, not a gap. The list is static — ranking is local per
keystroke through the ordinary picker path; no per-keystroke respawn.
The pane shares the symbols preview shape (numbered gutter, one ▶ on
the declaration line). LSP warm sessions and unopened-project lazy
init merge into this surface next; `docs/vim-compat.md` regenerated.
Evidence: source worker test (rows, chips, non-source exclusion),
engine test (open → list → Enter jumps to name at line:col), render
preview pin, compat report freshness.

Warm-server merge slice (2026-09-14): the workspace-symbols surface
now folds in language servers. `workspace/symbol` rides a
document-free lane in strop-lsp (`Client::workspace_symbols` → a
`WireJob::WorkspaceSymbols` on the ordered wire): admission requires
a ready server advertising the provider — `NotReady` servers skip and
join on their `Ready` event; both reply shapes map to the shared
`ProtoSymbol` row form, and uri-only locations (a resolve-support
contract never advertised) drop honestly. Replies correlate on the
picker's query generation, not a document stamp: every input change
bumps the generation and re-asks warm servers; superseded replies
trace-reject and never merge. Merged rows dedup against the syntax
tier by (path, line, name) and re-rank locally. Failures surface per
server at the live generation (R9: one terminal event either way).
Still open from §2: lazy initialization of eligible *unopened*
projects (bounded concurrency, catalog-driven), per-project status
for missing configuration, and incremental index reuse (0058).
Evidence: client wire tests (flat/nested mapping, uri-only drop,
error terminal, NotReady/Unsupported refusals with zero wire
traffic), engine merge test (dedup against the syntax tier, stale
generation rejection).

Lazy warm-up slice (2026-09-14): eligible *unopened* projects start
their servers. The catalog's marker subprojects now carry their
marker file (`ProjectKind::Marker(&str)`); the workspace-symbols
source reports them once per run (`PickerMsg::ScopeProjects` — the
same walk, no second scan), and the engine enqueues bounded warm
attaches: at most four in flight, one completion drains the next,
installing any other surface or closing the picker ends the queue —
never every installed server for every query. A warm attach reuses
the ordinary discovery path (config layers → trust → executability →
spawn) keyed by the project root; live placements and sticky
refusals are respected, and no document is ever faked. Marker
families map explicitly (Cargo.toml→rust, pyproject/setup.py→python,
CMakeLists.txt→cpp, go.mod→go); `package.json` stays cold — JS/TS is
ambiguous without configuration evidence. Combined with the
Ready-event join, the acceptance flow works: open a parent directory,
`space S`, and servers warm up, attach, and merge their symbols
without opening a file. Still open from §2: per-project status rows
for missing configuration, and incremental index reuse/invalidation
(0058's worker). Evidence: catalog marker test, source ScopeProjects
emission test, engine gate tests (services-off, queue lifecycle,
live-placement and sticky-refusal respect, ambiguous-marker cold
path).

Verification tier slice (2026-09-15, §6.1/§6.2/§6.8): deterministic
property corpora and an independent reference interpreter landed in
`query/properties.rs` — a fixed-seed LCG, no RNG dependency. The
parser corpora pin: arbitrary inputs (operator words, parens, quotes,
escapes, colons, multibyte) never panic and always settle into
Ready/Incomplete/Invalid; canonical formatting is a fixed point for
Ready queries; operator-free, evidence-free inputs stay flat. The
reference interpreter evaluates generated ASTs (≤ depth 3, literals +
all metadata families, negations) over five paths × five lines with
fixture ground-truth evidence closures — 125,000 differential
evaluations agree with `BooleanPlan` admission. The differential found
two real bugs, both fixed at source: (1) canonical quoting was too
weak — hostile values (backslashes, parens, colons) round-tripped
into Invalid or different trees; quoting now escapes `\` and `"` and
covers every lexer-hostile character; (2) `glob_literal_match` used a
first-match scan that silently rejected unanchored stars — `*.rs`
missed `a/src/x.rs`; replaced with the classic single-resume wildcard
match, pinned by its own regression test. §6.8's shape landed as a
mixed-directory e2e through the REAL pipeline (rg child, reader
threads, catalog, symbol index): nested repos, a vendor subtree,
pyproject marker, Lua/C++ loose sources — `repo:` narrowing,
branch-plus-exclusion, `kind:function` and cross-language
`kind:class` all exact through real rg output. Still open from §6:
the TLA+/TLC lifecycle model, negative controls in the model tier,
Verus/TLAPS correspondence, and the full §6.8 fixture with worktrees,
dirty buffers, cancellation and remote isolation.

Lifecycle model slice (2026-09-15, §6.4/§6.5/§6.7): `specs/
SearchLifecycle.tla` joins the verification registry — generation-
tagged provider jobs, late-arriving evidence, dirty-source revisions,
bounded warm-up and scope restart, at the publication boundary the
Rust code has. The §6.5 named properties are the model's invariants:
RowsCurrent (no retired-query publication), RowsInScope (no foreign
results), StaleAcceptsNever (no known-stale symbol acceptance),
CompletionHonest (no false completeness — the flag requires the
current generation's untruncated terminal and dies with the surface),
WarmBounded (resource bound). The kept mutant drops the publication
generation guard and the acceptance revision check; TLC kills it by
exactly RowsCurrent + StaleAcceptsNever (negative control §6.7 — any
other kill would be a modeling bug). Wired into `specs/gate.sh` via
`search-gate.sh` (the compose `model` service and CI's Protocol-model
step run it); bounds: 2 generations, 2 paths, 2 revisions, warm ≤ 4.
Not modeled (header states it): text/scoring (the differential
interpreter owns it), per-project LSP configuration scoping,
liveness. Still open from §6: Verus/TLAPS correspondence (§6.6) and
the full §6.8 fixture (worktrees, dirty buffers, cancellation, remote
isolation).

Replacement slice (2026-09-15, §4/§6.3): With/Review works under the
Boolean grammar. `ContentPlan::replacement_target()` decides
eligibility — simple queries target their content expression; Boolean
queries target the unique positive content atom by NOT-parity
(`NOT NOT x` is x); several positives are an explicit ambiguity
refusal ("this query has N"), none a named refusal — never a
first-match. The review worker re-checks every span against the
single target: foreign prefilter spans (and any stale provider span)
never edit. With one positive atom the provider union equals the
target, so streamed rows and previews are target-true by
construction. The frozen-AST/stale-review machinery is unchanged —
preview and apply already shared the ChangePlan freeze. The flow
tests exposed and fixed a real §3 migration gap: the engine's Search
surface itself rejected Boolean queries (`content search needs a
text or regex expression`) — the grammar had only ever been exercised
through the worker-level sources; the gate now accepts either form
and the mode chip reads "boolean". Evidence: eligibility unit pins
(single/ambiguity/missing/parity/case), engine flow tests
(negative-guarded replacement edits only target spans; ambiguity
refuses by name and mutates nothing; conjunction and negative search
through the surface), all through real rg.

Per-project status rows slice (2026-09-16, §2): every warm-attach
outcome now lands as a per-project status row in the workspace-symbols
picker instead of only flashing by on the status line. Recording happens
at all four decision points — ambiguous marker (package.json stays
`cold`), sticky-refusal skip, dedup/live-placement skip (recorded
healthy, superseding stale rows, never rendered), and warm-originated
attach completions (`AttachState::warm_attempts` gates recording so
document attaches keep status-line-only reporting). Rows ride a pinned
tail inside `Picker` (`set_pinned`; `append` re-seats the tail so late
source batches and LSP merges can never bury them; filtering never hides
them, ranking never reranks them) with a dedicated
`Payload::ProjectStatus(root)` — accept opens the project root as a
directory buffer, and the payload marker structurally excludes status
rows from symbol candidacy in merge/dedup/preview. Only non-healthy
outcomes render (NoServer "no srv", TrustRequired/TrustError "trust",
NotExecutable "no exec", SpawnFailed, RemoteIo, Ambiguous "cold");
wording reuses `explain_decision`'s actionable-reason convention.
Installing any non-WorkspaceSymbols surface clears the statuses with the
warm queue. Evidence: five new `warm_up` tests (ambiguous-cold row,
sticky-refusal row, surface-change clearing, NoServer completion row,
document-keyed completions record nothing) plus an engine test ending in
`editor.directory() == root` on accept.

Workspace-symbols AST slice (2026-09-16, §3/§4): the surface consumes
the one shared parser end to end. Input parses through
`picker_query_eval` — diagnostics, highlights and the error card are
exactly the Search surface's path. The syntax tier compiles a
`ContentPlan` when the query is Boolean and admits each declaration by
three-valued AST admission (content against name/qualified form via the
new `SymbolEvidence`, `kind:` from the candidate's own classification,
`repo:` against the run's catalog, path-family atoms against the
relative path; Unknown admits). The LSP tier's wire carries only
`SearchQuery::wsymbols_probe()` — positive content literals or empty;
qualifiers, operators and regex never reach `workspace/symbol` — and
replies are filtered locally against the full AST at the live generation
before dedup/merge (no engine-side catalog, so `repo:` honestly
overfetches). Operator-free input keeps the byte-identical static list +
local fuzzy narrowing; invalid/incomplete queries keep rows, show the
diagnostic, retire in-flight replies and ask servers nothing. Evidence:
plan tests (probe carries only bare content, kind decides from the
candidate, qualified-form matching), a source admission test, five
engine tests across both tiers, and a strop-lsp wire test pinning that
the probe crosses verbatim. Document symbols followed the same migration
(see below).

Stored-query versioning slice (2026-09-16, §3/§6.1):
`strop_picker::query::store` is the one versioned record convention for
persisted queries (no picker query persists yet; search history and
session storage will use this record): `StoredQuery { syntax_version,
source }`, version 1 = the pre-Boolean flat literal semantics, version 2
= this grammar. Unknown versions are typed rejections at the serde
boundary and at use time, never silent re-reads (the path_serde
precedent). `migrate_v1` is pure and reuses the lexer's token
boundaries: standalone AND/OR/NOT words and unquoted
`kind:`/`type:`/`repo:` tokens are quoted so v1 literal meaning survives
verbatim (quoting transitively keeps zero-depth parens literal, since
grouping triggers only on unquoted operator words); migration is a fixed
point. The slice exposed and fixed a real source bug: query-wide options
(`case:`/`hidden:`/`ignored:`) previously surfaced as inert Metadata
atoms *inside* Boolean branches — compiled to Unknown, silently
broadening OR admission. `BooleanParser::peek` now skips option tokens,
options exist only as the query-wide fields, and
`canonical_source` emits them in a leading preamble (`case:… hidden:…
ignored:…` order) — a Ready-query fixed point. Evidence: the §6.1
migration corpus (14 cases), unknown-version rejection, parse→format→
parse over migrated output, a serde wire-shape pin, and the
leading-preamble pin.

Surface inventory slice (2026-09-16, §3): the complete query-bearing
inventory, with each surface's engine made explicit:

| Surface | Engine | Mode |
|---|---|---|
| Files picker, Search picker, Replace query field, Directory filter | shared `SearchQuery` AST | full Boolean + qualifiers |
| Workspace symbols (`space S`), Document symbols | shared `SearchQuery` AST over `SymbolEvidence` | Boolean + qualifiers; LSP wire carries the bare probe only |
| Buffers, Jumps, Diagnostics, Locations pickers | raw fuzzy needle | explicit surface-specific default matching (fzf-style filter; not qualifier-bearing) |
| Normal-mode `/` `?` `*` `#` `n`/`N`, incsearch, command-parser search operands | `strop_grammar::CompiledQuery` (vim-magic regex) | the vim pattern dialect — 0001's fidelity doctrine keeps vim semantics; this is not a second *Boolean* implementation |
| Ex `:[range]s/pat/repl/` | literal split on `/` | vim ex semantics, unchanged |
| Occurrence session | raw literal needle | literal, unchanged |

Qualifier-typed input on the fuzzy surfaces matches literally by design
(their mode is explicit); every qualifier-bearing surface goes through
the one parser, so unsupported qualifiers produce located diagnostics,
never silence.

Document-symbols AST slice (2026-09-16, §3): `Kind::Symbols` parses
through `picker_query_eval` and filters candidates locally with the
`SymbolEvidence` admission (content against name/qualified form, `kind:`
from the candidate's classification); documentSymbol has no server-side
query parameter, so filtering is purely local. Operator-free narrowing
is byte-identical to the previous fuzzy behavior. Evidence: targeted
engine tests (kind narrowing, Boolean content+kind, literal
compatibility, diagnostics path).

§6.6 production-correspondence slice (2026-09-16):
`strop_core::searchguard` is the verified publication-boundary kernel
(0045's same-source pattern — verus! blocks with spec/exec tie-outs,
ghost code erased in normal builds; the compose `verify` stage proves
it). One decision per named invariant: `generation_is_live`
(RowsCurrent), `revision_is_current` (StaleAcceptsNever),
`completion_is_honest` (CompletionHonest), `warm_slot_free`
(WarmBounded). Production guard sites call the kernel
behavior-preservingly: `handle_picker_event` ticket ownership,
`merge_workspace_symbols` generation check, `lsp_drain_warm_queue` warm
bound, and the replacement-preview witness `BufferRevision` re-check
(the numeric correspondent of the model's Accept guard;
`SymbolIndex::overlay`'s staleness is structural — dirty text replaces
the disk entry before admission — and is exercised by the trace replay
instead). `completion_is_honest` has no pre-existing production boolean
to replace and is asserted at the handler seam rather than force-wired.
Model-trace replay drives the actual handlers with the spec's transition
vocabulary on both sides: engine-side (`picker/lifecycle_traces.rs` —
submit/supersede, late retired-generation publish refused, truncated
completion dishonest, warm-bound enforcement) and picker-side
(`source/lifecycle_traces.rs` — EditBuffer → acceptance re-check through
`SymbolIndex::overlay`). Negative traces the model rejects are refused
by the handlers. TLAPS proof obligations for the lifecycle model attach
to 0057's planned tlaps lane; the applicable Verus obligations landed
here. Evidence: the trace suites pass, and each guard assertion fails
when its guard is removed (kept-mutant reasoning per test).

§6.8 full-fixture slice (2026-09-16): `source/grep/e2e.rs` extends the
mixed-directory harness through the REAL pipeline (rg child, reader
threads, catalog, symbol index): a worktree whose `.git` is a file
(`repo:wt-feature` narrows to the worktree basename, not the parent), a
dirty `SourceSnapshot` contradicting disk (buffer text authoritative for
content AND `kind:` evidence, with a differential control), cancellation
of a provably-running search (exactly one terminal
`Finished(Cancelled)`, zero late rows, then disconnect), supersede
(retired generation never publishes; the current generation publishes
the complete 9-row result set), Unicode paths/symbols with exact byte
columns, and remote namespace isolation at the seam (remote hits keep
`Filesystem::Remote`; forged `../` and absolute paths and NUL-bearing
names are rejected before any item exists — hermetic, no sshd; remote
catalog/kind evidence stays 0058-deferred per `remote.rs`). Evidence:
`cargo test -p strop-picker source` — 34 passed, the six new e2e tests
repeated 10× with zero flakes.

Remaining obligations, by owner: incremental cross-search index
reuse/invalidation and remote marker/catalog evidence →
[0058](0058-unified-native-worker.md); TLAPS proof lane for the
lifecycle model → [0057](0057-core-verification-and-assurance.md).
