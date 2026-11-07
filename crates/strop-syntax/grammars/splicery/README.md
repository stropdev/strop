# tree-sitter-splicery — regeneration recipe

Presentation grammar for **Splicery /1** (experimental syntax
highlighting; Strop plan 0083 — NOT an authoritative frontend; the
Splicery repository's `crates/splicery-syntax` owns lexing/parsing and
every refusal).

## Provenance

- Splicery source of truth: `/home/tarek/workspace/splicery`
  (treated as **read-only**), revision `c696b6e` ("Correct final SG00
  receipt paths"; dirty working tree under active SG-gate work — this
  grammar implements the committed spec only). Normative sources:
  `spec/splicery.ebnf` (24 productions), `spec/language.md` §Lexer,
  with `crates/splicery-syntax/src/{lexer,parser}` as the
  token-boundary reference and the frozen-replay shape from
  `tools/check_language.py`.
- Splicery is MIT licensed ("Writfold authors"); small quotes are
  permitted with notice, but per the 0070 convention every corpus
  snippet here is an original Strop-side test input.
- Generator: `tree-sitter-cli` **0.25.10**
  (`cargo install tree-sitter-cli --locked --version 0.25.10`).
- Generated assets (`src/parser.c`, `src/grammar.json`,
  `src/node-types.json`, `src/tree_sitter/`) target **ABI 15**; the
  locked Strop runtime is tree-sitter **0.25.10** (supports ABI 13–15).
- **One external scanner**: `src/scanner.c` implements the heredoc
  (`<<'TAG'` … `TAG`) — a dynamic terminator cannot be a regex token
  (the concrete need plan 0070 required before adding a scanner).
  `heredoc_start` / `heredoc_body` / `heredoc_end` mirror the reference
  lexer exactly (leading-whitespace lookalikes and `TAGX` suffixes are
  content; `TAG\r` at EOF is not a terminator; the closer may end at
  EOF; an empty body is valid).

## Regenerate (regeneration-time tools only; never in the locked build)

```sh
cd crates/strop-syntax/grammars/splicery
# node is needed by the CLI to load grammar.js; any node >= 18 works.
tree-sitter generate --abi 15
tree-sitter test
```

Do not hand-edit `src/parser.c`, `src/grammar.json` or
`src/node-types.json`. After regenerating, rerun:

```sh
cargo test -p strop-syntax
```

The normal Strop build compiles the committed `src/parser.c` and
`src/scanner.c` via `crates/strop-syntax/build.rs` (`cc`); it never
runs the CLI, node or npm.
