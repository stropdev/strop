# 0083 — Splicery syntax highlighting in Strop

Status: **landed** (2026-11-07) — grammar at
`crates/strop-syntax/grammars/splicery/` with the one external scanner
(`src/scanner.c`, heredocs; opener records the tag, body and terminator
line are scanner tokens), corpus 21/21 green including every heredoc
edge (empty body, EOF-terminated body, terminator at EOF, CRLF lines,
indented and suffixed lookalikes, body containing `#`/quotes); all
eleven sibling-checkout examples plus the frozen replay parse with
zero ERROR/MISSING; query at `queries/splicery/highlights.scm`,
registered as `splicery`/`.spl` in strop-syntax and strop-core, `#`
comment toggle wired, tests in
`crates/strop-syntax/src/tests/splicery.rs` plus styled-cell evidence
in `crates/strop/src/render/buffer/tests/syntax_0083.rs` (100×30 pane,
40×15 narrow pane, a frame after an edit inside a heredoc body, picker
preview). `docker compose run --build --rm test` green (fmt, strict
locked clippy, locked tests). No LSP entry, no symbols, no structural
indentation, no release cut.

## 1. Decision, authority and boundaries

The owner wants Splicery recipes (`.spl`) to open in Strop with useful colors
while the language is still evolving, and asked whether Tree-sitter is the
proper mechanism. It is: implement a **dedicated presentation grammar**
statically linked into `strop-syntax`, exactly the route plan 0070 established
for Sovel. Do not map `.spl` onto an existing grammar (nothing shares
Splicery's `@handle :=`, `rx"…"`, `hex"…"` or `<<'TAG'` heredoc shapes; an
alias would miscolor at the language's distinctive boundaries) and do not add
a renderer-side regex lexer.

Splicery owns its authoritative lexer/parser (`crates/splicery-syntax` in the
Splicery repository). The Strop grammar is presentation only: a colored or
error-free tree is not proof of a valid recipe, exact-match inventory, or any
Splicery gate. This plan authorizes highlighting, nothing else:

- no Splicery frontend/checker/kernel code in Strop, and no build-time or
  runtime dependency on the Splicery checkout (it is read-only input);
- no LSP server or spawn for `.spl`, no formatter, no symbol index, no
  semantic validation or "recipe valid" claim of any kind;
- no structural-indentation registration (0072's whitelist is unchanged;
  `.spl` inherits whitespace indentation);
- current theme and shared class-to-style projection only (0068/0069 own
  palette work); no per-language colors, no new configuration knob;
- no release tag or publication by this plan; land with the next routine
  release following the standing process (0079).

## 2. Inspected baseline — recheck before editing

Strop checkout: `/home/tarek/workspace/strop`, inspected HEAD `98b50f9`
("Complete electric C-family labels and release 0.51.2"), workspace version
0.51.2. Lockfile pins are unchanged from 0070: `tree-sitter 0.25.10`
(ABI 13–15), `tree-sitter-language 0.1.8`, `cc 1.2.67`. Pin the grammar
generator to `tree-sitter-cli 0.25.10`, generate ABI 15. No existing Strop
crate claims the `spl` extension; `.spl` currently renders plain.

| Owner | Responsibility for this plan |
|---|---|
| `crates/strop-syntax/src/languages.rs` | `LanguageId::Sovel` + its `Entry` (~lines 33, 397) is the registration pattern: name, `grammar:` closure, extensions, aliases, `highlights: include_str!`. Add the Splicery entry; keep detection precedence. |
| `crates/strop-core/src/languages.rs` | Canonical extension/name catalog for filters and routing (`sovel` entry + test at ~line 65). Register `splicery`/`spl`; no third table. |
| `crates/strop-syntax/build.rs` | Compiles committed `grammars/sovel/src/parser.c` via `cc`. Extend for Splicery's `parser.c` **and** external scanner. No CLI/node in the locked build. |
| `crates/strop-syntax/src/sovel.rs` | The `LanguageFn` FFI bridge pattern with its SAFETY invariant. Copy as `src/splicery.rs`. |
| `crates/strop-syntax/queries/splicery/highlights.scm` | New hand-authored query; capture heads must be ones `Class::from_capture` and `Emphasis::from_capture` actually handle (`lib.rs` ~line 54). |
| `crates/strop-syntax/src/comments.rs` | `line_comment(LanguageId)` feeds comment toggle/insertion. Add `Splicery => Some("#")` — Splicery has real `#` comments, unlike Sovel (`None`). |
| `crates/strop-engine/src/editor/analysis/` | 0066's worker, freshness admission and edit-mapped presentation. Untouched by this plan; its guarantees must survive (§6). |
| `crates/strop/src/render/mod.rs` | Shared `syntax_style` projection for panes/previews. No changes. |
| `crates/strop-lsp/src/languages.rs`, `registry.rs` | `languages.toml` servers; `file-types` ignored. Add **no** entry; opening `.spl` must spawn nothing. |
| `crates/strop-syntax/src/{tests,languages/tests}.rs`, `tests/sovel.rs` | Test conventions: detection, real-`Highlighter` behavior, fence injection. Mirror as `tests/splicery.rs`; respect file-size ceilings. |

Adding a `LanguageId` variant makes exhaustive matches fail to compile — that
is the cutover checklist. Give every new arm the conservative default
(`ColonPolicy::None`, symbols unsupported) except `comments.rs` above.

Splicery checkout: `/home/tarek/workspace/splicery`, inspected HEAD `c696b6e`
("Correct final SG00 receipt paths") with a dirty working tree (7 files) under
active SG-gate work. Treat it as **read-only**: never reset, commit or "fix"
it; record the revision you actually implement against and recheck
`spec/splicery.ebnf` and `spec/language.md` §Lexer at implementation time.
`MANIFEST.sha256` pins the example recipes. The repository is MIT licensed
("Writfold authors") — small quotes are permitted with notice; still write
original corpus snippets per the 0070 convention and record provenance in the
grammar README.

## 3. Source of truth and grammar scope

Read, in the Splicery checkout: `spec/splicery.ebnf` (normative, 24
productions), `spec/language.md` (§Lexer is load-bearing), `examples/README.md`
plus all eleven `examples/*.spl`, and `crates/splicery-syntax/src/{lexer,
parser}` as reference behavior for token boundaries. No shipped example uses
`frozen`; the generated replay in `tools/check_language.py` shows its shape —
cover it with original corpus snippets. The whole surface:

```splicery
edit "sdk/src/client.rs" {
  @call := one text "transport.send(request)"
  replace @call with ["trace_send(", keep(@call), ")"]
}
```

```splicery
edit "service/config/routes.conf" {
  each @r in line_matches rx"^route[ \t]+(?<verb>[A-Z]+)[ \t]+(?<path>/\S+)$" expect 3 {
    replace @r with [keep(@r.verb), " ", keep(@r.path)]
  }
}
```

- **Documents:** `edit STRING [as bytes] {…}`, `create STRING [as bytes] with
  payload`, `rename STRING to STRING`, `remove STRING`, and a bare fragment of
  statements.
- **Statements:** `@h := selector`; `replace sel with payload`;
  `before sel insert payload`; `after sel insert payload`; `delete sel`;
  `each @h in collection { changes }`.
- **Collections:** `line_matches RX [within ref] expect NAT`; `frozen ref`.
- **Selectors:** reference `@h[.group]`; `one text STRING`;
  `one line STRING`; `one hex"00ff"`; `between ref and ref`;
  `range [NAT, NAT)`.
- **Payloads:** one piece or `[piece, …]`; pieces are `STRING`, `HEREDOC`,
  `hex"…"`, `keep(ref)`, `lines like ref [STRING, …]`,
  `indent(STRING, payload)`.

Load-bearing lexical cases:

- `#` comments run to the line break **outside literals**. Inside an `rx"`
  body, `#` and `\` pass through and any `"` terminates the token (single
  line; `\x22` spells a quote inside the regex, not an escape the lexer eats).
  `rx"…"` must be one tree-sitter token, body raw.
- `STRING` decodes only `\" \\ \n \r \t`; physical newlines and unescaped
  control characters refuse at the language level. The presentation grammar
  may accept a superset; document any deviation in COVERAGE.md.
- `hex"…"`: pairs of hex digits, case-immaterial, no separators. Odd digits
  are a semantic refusal, not a parse strategy — accept at grammar level.
- `:=` is one token. `HANDLE` is `@` + ASCII name. NAT refuses leading zeros
  semantically. Reserved words are barred from NAME/HANDLE by the language;
  the grammar tolerates them and COVERAGE.md says so.
- **Contextual spellings are not global keywords.** `text`, `line`, `like`,
  `within`, `expect`, `and`, `to` exist only inside fixed productions. An
  identifier spelled `text` or `line` elsewhere, keywords inside strings,
  regex bodies, heredocs and comments, and group names in `@r.verb` must stay
  unhighlighted or keep their literal class. Test each explicitly.
- Disambiguation: payload list `[piece, …]` versus `range [N, N)` (leading
  keyword decides); `as bytes` only after an edit/create path; `to` only in
  `rename`; `.` only selects a named regex group on a handle.

**Heredoc — the one external scanner.** `<<'TAG'` + LF/CRLF opens a body that
ends at the first physical line equal to `TAG` (terminator excluded, may end
at EOF, empty body valid, no stripping/interpolation/escapes, tag ≤ 64 ASCII
bytes). A dynamic terminator cannot be a regex; this is the concrete
demonstrated need 0070 required before adding a scanner. Follow the
tree-sitter-bash simple-heredoc precedent: keep `<<'TAG'` a regular grammar
token, and let a small committed scanner (~150 lines) return the body content
and terminator-line tokens. This is the only scanner in the grammar.

Use explicit precedence/conflict rules backed by corpus examples; no
catch-all rule that swallows a file to make the corpus report success.

## 4. Packaging and integration

Mirror 0070 §4. One independently understandable directory,
`crates/strop-syntax/grammars/splicery/`: `grammar.js` (provenance header,
assembly, conflicts; rules split into `rules/` if size demands), a pinned
regeneration `README.md` (`tree-sitter-cli 0.25.10`, `--abi 15`, node ≥ 18 as
a regeneration-time tool only), `COVERAGE.md` (grammar-rule-to-EBNF-family
table plus documented presentation deviations), `tree-sitter.json`,
generated `src/{parser.c, grammar.json, node-types.json, tree_sitter/}`, the
scanner source, and original `test/corpus/` files. Never hand-edit generated
files.

1. Extend `build.rs` to compile `parser.c` and the scanner with `cc` and
   `rerun-if-changed` both. Keep `Cargo.lock` and workspace versioning alone.
2. `src/splicery.rs`: the `sovel.rs` `LanguageFn` bridge with the same SAFETY
   argument (tables compiled into this crate; no dlopen, no version skew).
3. Query at `crates/strop-syntax/queries/splicery/highlights.scm`; no
   divergent copies.
4. `LanguageId::Splicery` + `Entry { name: "splicery", extensions: &["spl"],
   aliases: &["splicery", "spl"], … }`; the same language in `strop-core`;
   `comments.rs` gains `"#"`. Registering the name serves `splicery` Markdown
   fences through the normal injection route — no splicery-specific
   injections file.
5. No `strop-lsp` entry; default `.spl` open starts nothing.
6. The crate stays publishable: generated C, scanner and queries are crate
   files; no path/git dependency on the Splicery repository.

## 5. Highlight contract — existing classes only

`Class::from_capture` maps by capture head: `keyword`, `function`,
`type|namespace|label`, `string|character`, `comment`, `number|float`,
`operator`, `punctuation`, `constant|boolean`, `attribute|property`,
`tag`, else `Variable`. Use those heads only:

| Syntactic role | Capture → class |
|---|---|
| Document heads and verbs: `edit create rename remove as bytes with to replace before after insert delete each in line_matches frozen expect within one between and range` | `@keyword` → Keyword |
| Selector spellings `text`/`line` **inside `one` nodes only** | `@keyword` → Keyword (contextual, never global) |
| Reserved piece heads `keep`, `indent`, `lines`, `like` (reserved spellings, not identifiers — 0070's `drop` precedent) | `@keyword` → Keyword |
| Handle bound at `@h :=` / `each @h in` | `@variable.parameter` → Variable |
| Handle references; `.group` suffix or the whole `@r.group` reference | `@variable` (or `@property` → Attribute for the group suffix — pick one, document in the query header) |
| `STRING`, heredoc body, `hex"…"`, `rx"…"` | `@string` / `@string.special` / `@string.regexp` (all → String; heads kept distinct for future theme work) |
| `<<'TAG'` opener and closing tag line | `@operator` or `@punctuation.special` — pick one, document |
| NAT | `@number` → Number |
| `:=`, the `@` marker | `@operator` → Operator |
| `{ } ( ) [ ]` | `@punctuation.bracket` → Punctuation |
| `,` `.` | `@punctuation.delimiter` → Punctuation |
| `#` comments | `@comment` → Comment |

A color is not a match count or a permission. No keyword leakage into
strings/rx/heredocs/comments; no uppercase-means-type guessing; one query and
one style projection for panes, picker previews and injected fences.

## 6. Execution order and acceptance

1. Record current revisions and working-tree ownership of both checkouts.
   Read plans 0002, 0039, 0066 and 0070; coordinate with any active work; do
   not reset or commit another agent's files.
2. Implement grammar + scanner + corpus against the pinned snapshot; generate
   and validate the static parser (`tree-sitter generate --abi 15`,
   `tree-sitter test` from the grammar dir).
3. Integrate through the §4 owners.
4. Behavior tests, real-editor smoke, docs/changelog, then update this plan's
   Status with landed evidence.

**Done means all of the following, not just a parser build:**

- Every unchanged `examples/*.spl` (read from the sibling checkout, never
  vendored) opens with Splicery highlighting selected automatically via `.spl`
  and parses with **zero ERROR/MISSING nodes**, including the raw-profile
  (`as bytes` + `hex"…"`) and heredoc cards.
- Corpus green, including heredoc edges: empty body, EOF-terminated body, CRLF
  body lines, a lookalike line with leading whitespace (not a terminator), and
  a body line containing `#`/quotes.
- Incremental edits through the real `Highlighter` equal a fresh parse for
  representative edit/undo sequences, including edits inside a heredoc and an
  `rx"` body. 0066's admission and no-flicker guarantees hold (no stale-span
  regressions).
- UTF-8 content in strings/comments/heredocs, CRLF recipes, and multichunk
  ropes produce sorted, in-bounds, nonoverlapping spans.
- Incomplete input stays editable without crash or hang: unclosed string/rx/
  heredoc/`edit {` block, `one text` with no string, `range [1,`, stray `)`;
  recovered regions color sanely.
- A `splicery` Markdown fence goes through the normal injection route;
  `lang:splicery` filters agree with `.spl` detection (strop-core test).
- Opening `.spl` spawns no server, formatter or compiler and claims no symbols.
- Real styled-cell evidence from the actual new build: headless at 100×30 and
  a narrow pane plus one picker preview, with isolated HOME/XDG roots per
  0066 §2; capture a frame after settle and after an edit inside a heredoc
  body.
- The normal locked build needs no node/CLI and stays statically
  linked/publishable. `cargo fmt --check`, strict locked Clippy, locked
  `cargo test -p strop-syntax -p strop-core -p strop-lsp`, then
  `docker compose run --build --rm test` all pass; file-size ceilings
  respected (focused modules, not an inflated `tests.rs`).
- Documentation: a CHANGELOG entry in the 0.43.0 Sovel wording style
  ("**Splicery — experimental syntax highlighting** (0083): `.spl` recipes
  open …"), the in-editor language-info/help surface if it enumerates
  languages, and the public site's language-coverage page at that release's
  website sync if it enumerates coverage. No success/safety badge anywhere.

## 7. Explicit non-goals

No Splicery execution, preview, validation or wire protocol in Strop; no
authoritative-frontend claims; no symbols, indentation, formatter, LSP or
completion for `.spl`; no theme changes; no release cut by this plan.
