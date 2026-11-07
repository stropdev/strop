# tree-sitter-weft — regeneration recipe

Presentation grammar for **Weft, spec revision 3** (syntax highlighting;
Strop plan 0083 — NOT an authoritative frontend; `weft-frontend` in the
Weft repository owns lexing/parsing and refusal).

## Provenance

- Weft source of truth: `/home/tarek/workspace/weft`, revision
  `a115f51` ("Ship pinned writfold.agent/1 control client and local
  reuse gate"), spec revision 3 (`spec/grammar.ebnf`,
  `spec/language.md` §Lexical, `crates/weft-frontend/src/lexer/token.rs`
  as the lexical oracle).
- Weft is MIT licensed, so fixture text from `examples/*.weft` may be
  vendored into corpus tests with attribution
  (`test/corpus/showcase.txt`); every other corpus snippet is original
  Strop-side test input.
- Generator: `tree-sitter-cli` **0.25.10**
  (`cargo install tree-sitter-cli --locked --version 0.25.10`).
- Generated assets (`src/parser.c`, `src/grammar.json`,
  `src/node-types.json`, `src/tree_sitter/`) target **ABI 15**; the
  locked Strop runtime is tree-sitter **0.25.10** (supports ABI 13–15).
- No external scanner: strings are single-line and comments are
  line-based, so regex tokens suffice.

## Regenerate (regeneration-time tools only; never in the locked build)

`grammar.js` holds provenance, precedence/conflict policy and all
rules.

```sh
cd crates/strop-syntax/grammars/weft
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
`crates/strop-syntax/build.rs` (`cc`); it never runs the CLI, node or
npm.
