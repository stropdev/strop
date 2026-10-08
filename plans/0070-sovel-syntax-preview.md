# 0070 — Sovel syntax preview in Strop

Status: **implemented** (2026-10-08). Dedicated Sovel presentation
grammar (ABI 15, `tree-sitter-cli` 0.25.10, runtime 0.25.10) statically
linked through `crates/strop-syntax/build.rs`; queries at
`queries/sovel/highlights.scm`; `sovel`/`sov` registered in strop-syntax
and strop-core. Implemented against Sovel handover rev `eab2ce3`, syntax
snapshot `0.1-draft.4`; grammar provenance, the pinned regeneration
recipe and the rule-to-EBNF coverage table live in
`crates/strop-syntax/grammars/sovel/{README,COVERAGE}.md`. Verified:
23/23 corpus tests, zero ERROR/MISSING parse of the unchanged showcase
and all accept/reject fixtures, real-Highlighter behavior tests
(incremental == fresh parse, contextual effect spellings, no keyword
leakage, UTF-8/CRLF/multichunk spans, Markdown fence injection), and a
real-binary headless styled-cell smoke at 100×30 showing the §5 capture
classes distinguished by the current theme. Documented deviations
(assignment LHS as expression, left-assoc comparison/range, dynamic
precedence on control-head struct literals) are in COVERAGE.md. The
28-obligation language gate remains blocked and unchanged; no Sovel
server, formatter, symbols or validation ships.

The rest of this file is the original execution handoff, kept for the
record.

## 1. Decision, authority and boundaries

The owner wants to open the existing `.sov` examples in Strop and judge how the
language feels while its syntax is still evolving. Implement **experimental
syntax highlighting**, not a new global UI theme and not a Sovel compiler.

The owner explicitly selected **“Authorize editor-only preview now”** after
being told that the previous Sovel handoff blocked Tree-sitter and Strop
integration until `G-LANGUAGE`. The narrow exception permits the presentation
Tree-sitter grammar, its generated parser assets, highlight queries, `.sov`
registration and editor preview validation before that gate. The Sovel-side
policy and `editor/STROP-HANDOFF.md` record this exception.

Still forbidden by this exception: an authoritative Sovel frontend/parser,
checker, elaborator, interpreter, runtime, C importer/shim, formatter, LSP,
semantic-token service, or compile/run/debug integration. A colored or
error-free Tree-sitter tree is **not** proof of source validity, safety,
macro hygiene, correct foreign contracts, or successful compilation. The
28-obligation language gate remains blocked and unchanged.

**Recommendation:** a dedicated Sovel presentation grammar, statically linked
through the existing syntax crate. Do not map `.sov` to Rust or globally recolor
identifier strings. Rust's parser does not understand Sovel's `using`, `from`,
effect rows, regions or staged syntax; a Rust alias would give a misleading
preview at precisely the language's important boundaries.

Use Strop's current semantic colors. A separate “Sovel palette” would confound
the language experiment with a theme experiment. Plan 0068 reserves theme work
for 0069; this plan must not take ownership of that work or undo concurrent UI
changes. No release tag, publication, deployment or contact with other people
is authorized by this plan.

## 2. Inspected baseline — recheck before editing

Strop checkout: `/home/tarek/workspace/strop`. Inspected HEAD was `72dc90a`
(`Re-qualify completion on the 0.40.0 musl artifact`). The working tree is under
active parallel UI/version work: a later read already showed a `0.41.0`
inter-crate dependency while the installed binary reported `strop 0.40.0`.
Do not equate those versions or overwrite the other agent's changes.

Sovel checkout: `/home/tarek/workspace/sovel/sovel-handover`, equivalently
`../sovel/sovel-handover` from the Strop root. Its syntax snapshot is
`0.1-draft.4`; the latest completed proof milestone at planning time is P3k
(`b13bfa5`). The subsequent editor-only authorization changes policy, not syntax.
Record the exact source/spec revision used by the implementation.

Observed integration owners:

| Existing path | Responsibility / implication |
|---|---|
| `crates/strop-syntax/src/languages.rs` | `LanguageId`, `LanguageSpec`, static `LANGUAGES`, extension/name/shebang detection. Add a real Sovel entry; no runtime grammar downloads. |
| `crates/strop-core/src/languages.rs` | Canonical extension/name catalog consumed by query and LSP routing. Register `sovel` / `sov` here too; do not introduce a third detection table. |
| `crates/strop-syntax/src/lib.rs` | Rope-backed incremental parser/highlighter, cancellation and `Class` capture projection. Reuse it, not a renderer-side lexer. |
| `crates/strop-syntax/src/injections.rs` | Existing embedded-language routing; registering the name should serve `sovel` Markdown fences through the normal route. |
| `crates/strop-syntax/src/symbols.rs` | Declaration extraction is separately supported for selected languages. Leave Sovel unsupported here for this highlighting-only task; do not reuse Rust symbols. |
| `crates/strop-engine/src/editor/analysis/mod.rs`, `worker.rs` | Background analysis and edit-mapped presentation. Preserve plan 0066's freshness, geometry, cancellation and no-flicker ownership. |
| `crates/strop/src/render/mod.rs` | `class_color` / `syntax_style` are the shared style projection for panes and previews. No per-language hardcoded colors. |
| `crates/strop-syntax/src/languages/tests.rs` | Registry precedence and core/syntax catalog agreement. |
| `crates/strop-syntax/src/tests.rs` | Real `Highlighter`, rope edits and `apply_and_highlight` conventions. Add focused Sovel tests without growing this file beyond the repository's size discipline. |
| `crates/strop-lsp/src/languages.rs`, `registry.rs` | `languages.toml` configures servers; `file-types` is ignored. Adding that TOML key is not a syntax-registration solution. Unknown language families have no embedded server. |

The inspected lockfile contains `tree-sitter 0.25.10`,
`tree-sitter-language 0.1.8`, and `cc 1.2.67`. Runtime 0.25.10 supports grammar
ABIs 13–15. Pin the grammar generator to `tree-sitter-cli 0.25.10`, generate
ABI 15 explicitly, and test against the actual locked runtime. Revisit together
if the implementing checkout has changed; never use an unpinned latest CLI.

Primary references:

- [Tree-sitter parser development](https://tree-sitter.github.io/tree-sitter/creating-parsers/1-getting-started.html)
- [Generation outputs and ABI option](https://tree-sitter.github.io/tree-sitter/cli/generate.html)
- [Pinned 0.25.10 runtime ABI constants](https://github.com/tree-sitter/tree-sitter/blob/v0.25.10/lib/include/tree_sitter/api.h)

## 3. Source of truth and grammar scope

Read these Sovel files, in addition to its agent instructions:

- `spec/grammar.ebnf`
- `spec/LEXING-AND-PARSING.md`
- `spec/LANGUAGE.md`
- `spec/C-INTEROP.md`
- `editor/STROP-HANDOFF.md`
- `examples/README.md`, `examples/showcase.sov`, `examples/accept/`, `examples/reject/`

Implement the current snapshot's syntactic families, not just a keyword demo:
modules/imports; functions, contextual parameters and calls; types/generics;
structs/enums/newtypes/traits/impls; bindings and patterns; ordinary control flow;
origin/effect clauses; regions and tasks; attributes; macro token trees and
quote/splice forms; `extern "C"` declarations and named `unsafe(contract)` blocks.
Keep a grammar-rule-to-EBNF-family coverage table with the grammar. Source name,
type, ownership, effect, stage and foreign-contract validation are not parser
responsibilities.

Load-bearing lexical/ambiguity cases:

- ASCII identifiers; Unicode content in strings/comments; UTF-8 byte ranges and
  CRLF. Do not import Rust lifetime syntax or Unicode-identifier rules.
- `//`, `///`, **non-nesting** `/* ... */` and doc block comments.
- Plain quoted strings/chars and the specified escapes; no string interpolation
  or raw strings invented for convenience.
- `#[...]` attributes versus `name @ pattern`; `#` is not a comment marker.
- `Vec<T>` in types versus expression `f::<T>(x)`; nested `>` closings and `<`
  comparisons. Do not introduce `>>` as a shift operator.
- `2.seconds()` versus `2.0`, and `1..3` versus floating-point literals.
- `||` lambda at expression start versus Boolean OR; grouped closure or-patterns.
- Function clause order: parameters, `using`, result/`from`, `!`, `where`, body.
- Result dependencies versus `(fn(...) -> ... ! {}) from capture` dependencies.
- Explicit call-site `using (...)`, effect row tails/unions/exclusions, and
  `drop(...)` as the reserved callable spelling.
- Control-head braces versus parenthesized struct literals.
- Qualified nullary constructor patterns, guards and binding modes.
- Balanced macro input is not automatically a Sovel expression. Quotes contain
  structured Sovel syntax; `$x` and `${...}` are quote splices, not strings.
- Incomplete foreign types and function declarations; named unsafe contract
  paths; Rust-style link/export attributes. No bare unsafe block is invented.

Use explicit precedence/conflict rules backed by corpus examples. Do not
blindly translate the EBNF or add a catch-all rule that swallows the rest of a
file merely to make the corpus report success. An external scanner is not
expected for this snapshot; require a concrete demonstrated need before adding
one. Parser acceptance must not be advertised as language acceptance.

## 4. Minimal packaging and integration

Use one independently understandable grammar directory, proposed at
`crates/strop-syntax/grammars/sovel/`. This path does **not** exist yet.

Expected contents: `grammar.js`, grammar metadata, a pinned regeneration recipe,
`src/parser.c`, `src/grammar.json`, `src/node-types.json`, required generated C
headers, and focused `test/corpus/` files. Keep hand-authored rules small and
split by responsibility if needed; generated tables are the documented file-size
exception. Do not hand-edit generated parser code. Record the Sovel snapshot,
generator version and grammar provenance.

Recommended static integration:

1. Compile the committed C parser from a small new
   `crates/strop-syntax/build.rs` using `cc`; add a direct compatible
   `tree-sitter-language` dependency for the language-function bridge if needed.
   Preserve `Cargo.lock` and current workspace versioning conventions.
2. Expose the statically linked `tree_sitter_sovel` language through a focused
   internal module, proposed `src/sovel.rs`. Document the precise ABI/static
   pointer invariant at the small unsafe FFI boundary; use the established
   `LanguageFn` interface rather than inventing ownership of native pointers.
3. Keep the canonical Strop highlight query at
   `crates/strop-syntax/queries/sovel/highlights.scm`; avoid divergent copies.
   The grammar and query remain extractable/reusable without the editor UI.
4. Add `LanguageId::Sovel` and a registry entry with canonical name `sovel`,
   extension `sov`, aliases `sovel` and `sov`. Register the same language in
   `strop-core` for language filters. Keep existing detection precedence.
5. Do not add an embedded server or formatter. With default configuration,
   opening `.sov` must not spawn rust-analyzer, an imagined `sovel lsp`, or a
   compiler. Do not change explicit user server-configuration policy.
6. Verify Cargo packaging includes generated C, headers, queries and build
   inputs. Avoid a new unpublished path/git grammar-crate dependency that makes
   the existing Strop crates unpublishable. Do not add a sibling-checkout path
   dependency on the Sovel repository.

Node/CLI tools are regeneration-time tools only. A normal locked Strop build
and the delivered static binary must not run npm, download grammars, need a
JavaScript runtime, or require Sovel to be installed. Use the existing Docker
C toolchain and static-verification path from plan 0002.

Preserve upstream header/query licenses and attribution. The Sovel handover
currently has no project license grant: this authorization does not relicense
its documents or fixtures. Do not copy them wholesale into distributed Strop
artifacts under an assumed license. Use original small test snippets and record
provenance; resolve any required redistribution license before publication.
Local implementation/validation is not authorization to publish a new package
or cut a Strop release.

## 5. Highlight contract — use the current theme

`Class::from_capture` uses standard capture heads, not arbitrary new semantic
classes. Map to those existing classes:

| Syntactic role | Capture / existing class |
|---|---|
| Declarations, control, `using`, `from`, `region`, tasks, stages, `extern`, `unsafe` | `@keyword` → `Keyword` |
| Declared/called function names, macro names | `@function`, `@function.macro` → `Function` |
| Type positions, trait/type declarations, generic type parameters | `@type`, `@type.parameter` → `Type` |
| Effect labels/tails **inside effect syntax** | `@type` / `@type.parameter` → `Type` |
| Capability/origin binders and references | `@variable.parameter` / `@variable` → `Variable` |
| `#[...]` attribute names | `@attribute` → `Attribute` |
| Strings/chars, comments, numeric literals, booleans | Standard string/comment/number/constant captures |
| `!`, `?`, `&`, splice markers and actual operators | `@operator` → `Operator` |
| Braces/brackets/parentheses/separators | `@punctuation.*` → `Punctuation` |

Keep `using` authority, `from` dependencies and `!` behavior readable and
textually distinct; a color is not a permission or proof. Effect spellings such
as `read`, `write`, `alloc`, `io`, `foreign` are **not global lexer keywords**.
The identifier `read` in ordinary code must not be painted as an effect just
because of its spelling. Do not classify all uppercase identifiers as types.
Keywords in comments/strings stay comments/strings. Do not blanket-color an
arbitrary macro DSL as executable Sovel code.

Use only query predicates/directives the current Strop highlighter actually
handles. Keep one style projection for panes, picker previews and injected code;
no renderer-local keyword overlays or per-keystroke whole-buffer strings.
Preserve the analysis worker and plan 0066's edit-mapped presentation.

Document the capability as **“Sovel — experimental syntax highlighting”** in
language support/help/release notes. Keep canonical machine identity `sovel`;
do not overload it with a display label. Use an existing language-info surface
if available; do not invent a global status/banner framework. The showcase
already carries a visible “Syntax preview only” header. No success/safety badge.

## 6. Execution order and acceptance

1. Record current working-tree ownership and revisions. Read Strop plans 0001,
   0002, 0039 and 0066; coordinate with active 0068/0069 work. Do not reset or
   commit another agent's files.
2. Implement grammar and corpus against the pinned syntax snapshot; generate
   and validate its static parser. Pin regeneration inputs.
3. Add registration/query/static build integration through the existing owners.
4. Add behavior tests and exercise the real editor; then update documentation,
   changelog and the relevant assurance inventory using their existing rules.

**Done means all of the following, not just a parser build:**

- Opening the unchanged Sovel showcase and every current accept/reject fixture
  selects Sovel highlighting automatically. Semantic rejection fixtures do not
  become syntax errors merely because they are under `examples/reject/`.
- The showcase's functions, contextual/dependency/effect clauses, attributes,
  traits, regions, tasks, quotes and splices are usefully distinguished by the
  current theme. Foreign call/callback/export fixtures are included too.
- `lang:sovel` / the existing language-filter vocabulary agrees with `.sov`
  detection. A `sovel` Markdown fence uses the ordinary injection route.
- Default `.sov` opening starts no LSP/compiler/formatter and provides no false
  symbol-index or semantic-validation claim.
- Incomplete `#[derive(`, `using`, `! {`, generic closings, quotes/splices,
  comments/strings and braces remain editable without crashes or hangs.
  Legitimate nonlocal syntax reinterpretation is allowed; stale spans may not
  paint the wrong bytes or bypass current-revision admission.
- Actual incremental edits use the core journal and retained parser. Compare
  incremental output to a fresh parse for representative edit/undo sequences;
  assert token boundaries/classes and no keyword leakage into strings/comments,
  not just a nonempty highlight list or source-text assertions.
- Cover UTF-8 comments/strings, CRLF and multichunk ropes; every emitted span is
  valid, in-bounds and suitable for existing normalization/rendering.
- Existing language tests and query compilation remain green. Use focused
  tests or a `tests/` submodule rather than inflating the existing 575-line test
  file toward the repository ceiling.
- Real rendered frames at normal and narrow widths demonstrate useful colors
  in the main pane and a preview; smoke Markdown injection as well. Attach a
  readable screenshot or styled cell-grid evidence from the actual new build.
- The normal locked build needs only shipped artifacts and preserves static
  linking/packaging. Run the repository's full compose gate before completion.

Suggested verification (run commands only after inspecting current compose
services; record exact successful commands):

```sh
# From the grammar directory, using the pinned generator environment:
tree-sitter generate --abi 15
tree-sitter test

# From the Strop checkout, focused checks during iteration:
cargo test --locked -p strop-syntax
cargo test --locked -p strop-core
cargo test --locked -p strop-lsp

# Required repository gate:
docker compose run --build --rm test
```

Also validate the existing static artifact/package route without publishing.
Do not claim a platform was exercised if its build was unavailable.

For a live headless showcase smoke, create this temporary script, use isolated
HOME/XDG roots, and open the real Sovel fixture with the **newly built** binary:

```text
resize 100 30
settle 5000
frame
keys 50G
settle 5000
frame
```

Run the actual binary with `--headless SCRIPT PATH_TO_SHOWCASE --log-file NEW_TRACE
--log-content`. Inspect styled cell evidence, not only the plain `frame` text;
if the selected build does not emit those cells, use the existing styled
TestBackend/GUI capture route and report that limit rather than inventing
colors. Do not save edits to the user's specification fixture.

## 7. Planning evidence and handoff limits

During preparation, `strop --version` reported **0.40.0**. A real isolated
headless run opened `examples/showcase.sov` at 100×30, navigated to line 37,
printed both frames and exited successfully. This establishes current file
opening/navigation, **not** implemented Sovel highlighting or a post-change
visual result. The inspected source has no Sovel language entry. The captured
run supplied render hashes rather than an inspected styled-cell grid, so no
before-color measurement is claimed.

No Strop source, dependency, grammar, palette or installed binary was changed
by this planning task. This file is the execution handoff. Re-run the current
repository checks and provide real post-implementation evidence; do not carry
forward these baseline observations as proof that the new grammar works.
