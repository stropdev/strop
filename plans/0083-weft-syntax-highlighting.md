# 0083 — Weft syntax highlighting in Strop

Status: **landed** (2026-11-07) — grammar at
`crates/strop-syntax/grammars/weft/` (tree-sitter-cli 0.25.10,
`--abi 15`; corpus 12/12 green including the vendored showcase; all
eight sibling-checkout examples parse with zero ERROR/MISSING), query
at `queries/weft/highlights.scm`, registered as `weft`/`.weft` in
strop-syntax and strop-core, `//` comment toggle wired, tests in
`crates/strop-syntax/src/tests/weft.rs` plus styled-cell evidence in
`crates/strop/src/render/buffer/tests/syntax_0083.rs` (100×30 pane,
40×15 narrow pane, picker preview through the preparation route).
`docker compose run --build --rm test` green (fmt, strict locked
clippy, locked tests). No LSP entry, no symbols, no release cut.

## 1. Decision and rejected alternatives

Weft is a two-day-old formal-review language owned by the same author
(`/home/tarek/workspace/weft`, MIT licensed). Strop is its natural editor.
The correct approach is the established one:

- **Adopt:** author `tree-sitter-weft` as a presentation grammar under
  `crates/strop-syntax/grammars/weft/`, generated with the pinned
  `tree-sitter-cli 0.25.10` (`--abi 15`, matching the locked 0.25.10 runtime),
  with generated C committed and compiled by `build.rs`. Register `weft` /
  `.weft` through the normal language registry. This is the Sovel pattern
  (0070) with one simplification: Weft is MIT, so fixture text from
  `examples/*.weft` may be vendored into corpus tests with attribution.

- **Reject aliasing `.weft` to an existing grammar** (Rust, SQL, …): Weft's
  load-bearing surface (`:=`, `in` as both binder and operator, set algebra
  `| & -`, single non-chaining comparisons, `review` structure) is exactly
  where an alien grammar would mislead. 0070 §1 rejected the same for `.sov`.

- **Reject a regex/keyword highlighter**: 0039 forbids regex substitutes, and
  the whole pipeline — analysis worker, incremental reparsing from rope
  chunks, 0066's edit-mapped presentation, picker previews, injections — is
  tree-sitter-native. A second lexer path is a parallel convention.

- **Nothing to wait for**: no published `tree-sitter-weft` crate exists, and
  the language is the owner's own; the grammar is small (a 58-line EBNF).

A colored tree is **not** source validity. `weft-frontend`'s lexer/parser
remains the authoritative frontend; Strop ships no Weft LSP, formatter,
symbol index, checker or run integration. Parser acceptance must never be
advertised as language acceptance.

## 2. Sources of truth

Read, in the Weft checkout:

- `spec/grammar.ebnf` — full revision-3 syntax.
- `spec/language.md` — lexical draft (§Lexical), precedence decisions, and
  the formation/assurance map.
- `crates/weft-frontend/src/lexer/token.rs` — the real token set (`Kind`,
  22 keywords in `Kind::word`); use it as the lexical oracle.
- `crates/weft-frontend/src/lexer/` tests — accept/reject behavior for
  maximal-munch and escape rules.
- `examples/*.weft`, especially `set-algebra.weft` — the showcase; line 37
  (`shared | {9} & intersection - {2}`) is a ready-made precedence corpus
  case.

In Strop, the Sovel integration is the template: `grammars/sovel/`
(README regeneration recipe, COVERAGE table), `build.rs`, `src/sovel.rs`,
`queries/sovel/highlights.scm`, `languages.rs` entry, `strop-core/src/languages.rs`
catalog, `src/languages/tests.rs`, `src/tests/sovel.rs`.

## 3. Grammar scope

Implement every revision-3 syntactic family, not a keyword demo:

- Review header in order: `context STRING at STRING ;`, `profile STRING ;`,
  `target IDENT = proposal STRING ;`; one review per file.
- Evidence bindings (`claim|asset|rule IDENT = citation STRING ;`), set
  bindings (`IDENT := set_expression ;`), `need IDENT ;`,
  `assume IDENT : expression ;`, `check IDENT : expression [by IDENT {, IDENT}] ;`.
- Expressions with the specified precedence, tightest to loosest: paths and
  predicate calls; set atoms/literals/parenthesized set expressions; `&`;
  `|` and `-` left-associative at equal precedence; one optional
  non-chaining comparison (`== != in <=`); `not` (right-nesting); `and`; `or`.
- Set literals `{ … }` with optional trailing comma and `...spread` items;
  quantifiers `forall|exists IDENT in set_expression { expression }`.

Load-bearing lexical/ambiguity cases (from `spec/language.md`, all must have
corpus entries):

- ASCII identifiers; keyword boundary — `android` is one identifier, `in`
  ends at an identifier boundary. No Unicode identifiers.
- `STRING`: only `\" \\ \n \r \t` escapes; no raw newlines, interpolation or
  raw strings. Comments are not recognized inside strings. Non-control UTF-8
  allowed (multibyte spans must stay byte-correct).
- `INTEGER`: `0` or nonzero-leading digits; **no sign** — `-` is set
  difference only, never a negative literal.
- Maximal munch: `...` before `.`, `:=` before `:`, `==`/`!=`/`<=` before
  `=`/`<`/`!`. No standalone `!`, `+`, `^`, `~`, `&&`, `||`, `<`, `>` — do
  not invent tokens the language rejects.
- `in` dual role: quantifier binder keyword and membership operator — same
  token, different nodes; both highlight as keyword-operator.
- Three brace roles: review body, set literal, quantifier body.
- Comparisons do not chain (`a == b == c` is rejected upstream); keep the
  grammar's single-optional-operator shape rather than a loop.
- Whitespace: space, tab, LF, CRLF. The isolated-CR rejection is the
  frontend lexer's job; the presentation grammar may treat CR as an extra
  and must document that deviation in `COVERAGE.md`.
- Incomplete input (`check x: a ==`, unterminated string, missing `}`) must
  recover locally without swallowing the rest of the file.

Use explicit `prec`/conflicts backed by corpus examples; do not blindly
transcribe the EBNF or add a catch-all rule. No external scanner unless a
demonstrated need appears (none is expected).

## 4. Packaging and integration

1. `crates/strop-syntax/grammars/weft/`: `grammar.js` (split into `rules/` if
   it grows), `tree-sitter.json`, committed `src/{parser.c, grammar.json,
   node-types.json, tree_sitter/}`, `test/corpus/`, a `README.md` recording
   provenance (Weft revision, spec revision 3, pinned CLI + `--abi 15`,
   node ≥ 18 regeneration-only rule) and a `COVERAGE.md` grammar-rule ↔
   EBNF-family table including documented deviations.
2. Extend `crates/strop-syntax/build.rs` to compile both committed parsers
   (sovel and weft) via `cc`. The locked build still never runs the CLI,
   node or npm, and downloads nothing.
3. Add `src/weft.rs` mirroring `src/sovel.rs`: `extern "C"
   tree_sitter_weft`, `LanguageFn::from_raw`, the same SAFETY argument
   (parser compiled into this crate; no dlopen, no version skew).
4. `src/languages.rs`: `LanguageId::Weft` and an `Entry { name: "weft",
   extensions: &["weft"], filenames: &[], aliases: &["weft"], highlights:
   include_str!("../queries/weft/highlights.scm"), injections: "" }` with the
   grammar closure. Keep existing detection precedence (extension route).
5. `src/comments.rs`: `Weft => Some("//")` — unlike Sovel, Weft has line
   comments, so Ctrl-C comment toggle must work on `.weft`. Extend its test.
6. `strop-core/src/languages.rs`: register `weft` / `.weft` so `lang:weft`
   filters agree; add the catalog-agreement test next to the Sovel one.
7. `queries/weft/highlights.scm` — one canonical query, no divergent copies.
8. Do not add an embedded server, formatter or symbols support. With default
   configuration, opening `.weft` must spawn nothing. A ` ```weft ` Markdown
   fence must work through the normal injection route (comes free once the
   name is registered; test it). Do not modify the Weft repository — its
   workspace lints forbid `unsafe_code`, so the C-parser bridge could not
   live there anyway; Strop is the sole host.

## 5. Highlight contract — existing theme, no new classes

`Class::from_capture` (strop-syntax `lib.rs`) maps capture heads; use only
standard captures:

| Weft role | Capture | Class |
|---|---|---|
| `review context at profile target proposal` | `@keyword` | Keyword |
| `claim asset rule citation need assume check by` | `@keyword` | Keyword |
| `and or not`, `forall exists`, `in` (both roles) | `@keyword.operator` | Keyword |
| `true` / `false` | `@constant.builtin` | Constant |
| `INTEGER` | `@number` | Number |
| `STRING` (+ `@string.escape` inside) | `@string` | String |
| `// comment` | `@comment` | Comment |
| `== != <= := = & \| - ...` | `@operator` | Operator |
| Braces/parens/`;`/`,`/`.` | `@punctuation.bracket` / `@punctuation.delimiter` | Punctuation |
| Predicate call names | `@function` | Function |
| Declaration/label/binder identifiers | `@variable` | Variable |

Keywords inside comments/strings stay comments/strings (query structure gives
this; assert it). No per-language hardcoded colors, no renderer-side overlays,
no "Weft palette". The analysis worker and 0066's edit-mapped presentation
must be untouched — this plan adds a grammar and query, nothing upstream.

## 6. Verification and acceptance

**Done means all of the following, not a parser build:**

- `tree-sitter generate --abi 15` then `tree-sitter test` green in
  `grammars/weft/`, covering every §3 case (precedence line, maximal munch,
  dual `in`, brace roles, CRLF, multibyte strings, trailing comma, spread,
  incomplete-input recovery).
- Focused tests in `src/tests/weft.rs` (mirroring `tests/sovel.rs`; do not
  grow `tests.rs` past the size ceiling): showcase families distinguished by
  class; no keyword leakage into strings/comments; incremental edits through
  the real journal equal a fresh parse for representative edit/undo
  sequences; spans valid, in-bounds, UTF-8/CRLF/multichunk-correct;
  ` ```weft ` fence injection; comment toggle on `.weft`.
- Detection agreement: `.weft` extension, `weft` name/alias, and
  `strop-core` catalog (`lang:weft`) all resolve to the registered language;
  the highlight query compiles against the statically linked grammar.
- Real-binary headless smoke with the newly built binary (0070 §6 procedure,
  isolated HOME/XDG): open a copy of `examples/set-algebra.weft` at 100×30
  and a narrow width; styled cell-grid evidence showing keyword, operator,
  string, number and comment classes distinguished; type and undo mid-file —
  no crash, no stale-span mispaint (0066 guarantees hold).
- Repository gate: `docker compose run --build --rm test` (fmt, locked
  strict clippy, locked tests). Static linking and packaging unchanged; the
  normal build needs only committed artifacts.
- Document as **"Weft — syntax highlighting (presentation grammar)"** in the
  language-support surface and `CHANGELOG.md`. Ship with the next routine
  minor release per 0079; no separate release for this change. Record the
  landed evidence in this file's status line, not a new plan.

When Weft's syntax evolves, regenerate from the updated spec revision and
update `COVERAGE.md`; the grammar README pins the recipe.
