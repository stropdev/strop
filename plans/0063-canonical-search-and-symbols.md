# 0063 — Canonical search: one query language, automatic workspace symbols

Status: **authorized** after [0058 unified native worker](0058-unified-native-worker.md).
This plan supersedes the earlier search-deferral grouping: everything
query-bearing converges here, and filesystem notifications/reconciliation move
to 0058 rather than a separate plan. UI polish lives in
[0064](0064-ui-polish-scrollbars-and-cursor-fade.md). Completion (0059),
debugger (0060) and GUI (0061) stay deferred until the functionality landing
before them is complete and bug-hardened.

This document records requirements; it does not claim that new editor code,
tests, models or proofs have been executed.

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
