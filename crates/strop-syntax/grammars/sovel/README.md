# tree-sitter-sovel — regeneration recipe

Presentation grammar for Sovel **0.1-draft.4** (experimental syntax
highlighting; Strop plan 0070 — NOT an authoritative frontend).

## Provenance

- Sovel source of truth: `sovel-handover` repository, revision
  `eab2ce3290bb32fb32872d32890c313ee1828df2`
  ("Authorize editor-only Sovel syntax preview and record Strop handoff"),
  syntax snapshot `0.1-draft.4`
  (`spec/grammar.ebnf`, `spec/LEXING-AND-PARSING.md`).
- Generator: `tree-sitter-cli` **0.25.10**
  (`cargo install tree-sitter-cli --locked --version 0.25.10`).
- Generated assets (`src/parser.c`, `src/grammar.json`,
  `src/node-types.json`, `src/tree_sitter/`) target **ABI 15**; the locked
  Strop runtime is tree-sitter **0.25.10** (supports ABI 13–15).
- Corpus snippets under `test/corpus/` are original Strop-side test inputs
  written against the spec families (no Sovel fixture text is vendored;
  the handover has no license grant — Strop plan 0070 §4).

## Regenerate (regeneration-time tools only; never in the locked build)

`grammar.js` holds provenance, precedence tables, conflicts and assembly;
the rules themselves live in `rules/declarations.js` and
`rules/expressions.js` (repo file-size discipline).

```sh
cd crates/strop-syntax/grammars/sovel
# node is needed by the CLI to load grammar.js; any node >= 18 works.
tree-sitter generate --abi 15
tree-sitter test
```

Do not hand-edit `src/parser.c`, `src/grammar.json` or
`src/node-types.json`. After regenerating, rerun:

```sh
cargo test -p strop-syntax
```

The normal Strop build only compiles the committed `src/parser.c` via
`crates/strop-syntax/build.rs` (`cc`); it never runs the CLI, node or npm.
