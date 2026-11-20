---
rfc: "2026-11-17-one-filter-language"
title: "Use one filter language and kind table across lists"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-11-17"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/205"
milestone: "v0.71.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0109"
---

# RFC: Use one filter language and kind table across lists

PR [#45](https://github.com/stropdev/strop/issues/45) shipped kind: as the only kind key, one table for all 26 protocol kinds, declared schemas, located query errors and background admission over row facts. As built, repo: applies only in Search, plain choice lists do not parse filters, and path-bearing lists without kinds reject kind: rather than silently keeping everything; docs/search.md publishes the per-list table. 0108 deleted the obsolete stored-query reader instead of keeping a type: compatibility path.

[Tracking issue #205](https://github.com/stropdev/strop/issues/205) · [Published v0.71.0](https://github.com/stropdev/strop/releases/tag/v0.71.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0109-one-filter-language.md).

Follow-up issues: [#62](https://github.com/stropdev/strop/issues/62).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

- Status: **in force** (0.71).
- Source: the owner — "filtering symbols (space + s) … are we filtering with
  type: or kind:; both don't seem to work for me … it needs to work
  consistently and correctly across the board and preferably have a
  canonical implementation." The audit is appended as §A; this header is the
  decision record and wins where it differs.
- Builds on 0051, 0053, 0063, 0077. Amends 0063 (`type:` retired).

## Decisions (owner, 2026-11-17)

1. **`kind:` only.** `type:` is not a key; typing it is an unknown-key error.
2. **Kind values**: full words, the chip abbreviations (`fn`, `meth`,
   `const`, `iface`, …) and common language words (`trait` → interface,
   `func`, `def`, …), case-insensitive, unique prefixes accepted. One kind
   table (`strop-core`) covering all 26 LSP kinds feeds chips, values,
   Ctrl-Space suggestions and help.
3. **Unknown key or value**: a visible located error under the token and a
   one-line message; the list stays as it was until the query is fixed.
4. **Coverage**: every list picker declares a schema of the fields its rows
   have; Buffers, Diagnostics, Locations, Jumps take `path:`/`kind:`/`case:`
   where their rows carry the field. A key outside the schema is (3).
5. `kind:`/`repo:` no longer switch the query into boolean mode: bare words
   beside them stay fuzzy.
6. One `Admission` over borrowed row facts, evaluated in the ranking worker,
   not on the input thread; no per-row allocation per keystroke.
7. The hint row, footer, Ctrl-Space and `?` are generated from the schema,
   and `docs/search.md` says exactly what each list accepts.

---

## §A. Audit


Read-only audit. Nothing in the repo was edited, built or committed.

**Baseline.** File:line references are for the checked-out `main` (ca6caa2, 0.69.0).
The installed binary is `strop 0.70.0` (branch `feat/0.70`, 8282278, not merged into local
`main`). 0.70 changes nothing in the logic below; it adds an `impl` kind
(`"Object" => Impl` in `lsp_symbol_kind`, `"impl"` in `kinds_for_value`, the suggestion
list and `KIND_VALUES`) and shifts `editor/lsp/routing.rs` by +2..+5 lines. Implement on
top of `feat/0.70`.

**Evidence.** Every behaviour marked "ran" was reproduced with the installed 0.70.0
binary in `strop headless` against a scratch project with a hermetic `$HOME`. Scripts
and the project are in
`<audit scratch>/repro/`
(`q.sh '<open keys>' '<query>'` prints the picker frame). Rows marked "code" were
derived from reading only. Not run: Diagnostics, Locations, Actions, TabSize,
RemoteHosts, and no cargo test was run.

---

## 1. The owner's bug: `Space s` with `kind:` / `type:`

### 1.1 The path, keystroke to rows

1. Key: `feed_picker` inserts the char and calls `picker_input_changed`
   (`crates/strop-engine/src/editor/picker/keys.rs:206-208`).
2. `picker_input_changed`, `Kind::Symbols` branch
   (`crates/strop-engine/src/editor/picker/query.rs:334-362`) calls
   `picker_query_eval` (`query.rs:27`), which runs `SearchQuery::parse`
   (`crates/strop-picker/src/query/parser.rs:178`).
3. Lexer: `kind` and `type` are both in the global `QUALIFIERS`
   (`crates/strop-picker/src/query/lexer.rs:66-69`), so both lex as qualifiers.
4. Parser: a non-empty `kind:`/`type:`/`repo:` flips the whole query into Boolean mode
   (`parser.rs:186-195`). In that mode bare words are no longer collected as free text
   (`parser.rs:214`), so `query.content` is `None` and the fuzzy needle becomes `""`
   (`query.rs:85-90`). `type` is renamed to `kind` (`parser.rs:237`,
   `parser/boolean.rs:188`).
5. Plan: `rebuild_document_symbols`
   (`crates/strop-engine/src/editor/lsp/routing.rs:468-498`) calls
   `BooleanPlan::compile_retained` (`crates/strop-picker/src/query/plan/boolean.rs:194`).
6. Match: `document_symbol_items` (`routing.rs:509-566`) builds an `Evidence` per row
   with `kind: lsp_symbol_kind(&symbol.kind)` and calls `plan.admits`. The `kind:`
   decision is `boolean.rs:333-350`.

### 1.2 Why it "does nothing": three causes, all reproduced

**A. Only nine exact lowercase words are values; anything else is silently ignored.**
`SymbolIndex::kinds_for_value` (`crates/strop-picker/src/source/symbols.rs:246-258`)
accepts `function method class struct enum interface module constant` (+ `impl` in
0.70). Any other value returns `None`, which becomes `Decision::Unknown`
(`boolean.rs:335-339`), and Unknown admits (`boolean.rs:53-55`). There is no
diagnostic. This is deliberate and pinned by tests ("unknown evidence admits":
`editor/lsp/tests/navigation.rs:327-333`, `editor/picker/query_tests.rs:598-604`).

The picker itself teaches the wrong words: the chip column shows `fn`, `meth`, `iface`,
`const`, `mod`, `var`, `field`, `variant`, `T` (`routing.rs:599-618`,
`crates/strop-syntax/src/symbols.rs:34-45`). Ran:

| Typed | Result |
|---|---|
| `kind:function`, `type:function` | 5 of 11 rows: works (syntax outline) |
| `kind:struct` | 1 row: works |
| `kind:fn`, `kind:Function`, `kind:field`, `kind:trait` | all 11 rows, no message |

It also means the list stays whole while the value is being typed: `kind:f` through
`kind:functio` change nothing, and only the last character filters. Before that,
`kind` empties the list (fuzzy, no match) and `kind:` shows
"kind: expects a value" over "No symbols match". The test comment "rows stay while
typing" is true only for a paste.

**B. With a language server, rows of an unmapped kind pass every `kind:` filter.**
`lsp_symbol_kind` (`routing.rs:583-596`) maps 11 of the 26 LSP names. `Field`,
`EnumMember`, `Variable`, `Property`, `TypeParameter` and the rest return `None`,
which is Unknown, which admits (`boolean.rs:341-349`). Ran with rust-analyzer on a
19-symbol file:

- `kind:struct` kept 6 rows: the struct, 2 fields, 2 variants, 1 type alias.
- `kind:function` kept 10 rows, the first four being fields and variants.
- `-kind:function` has the same leak in reverse.

In a real Rust file fields and variants are most of the list, so the top of the list
does not change. This is the likeliest thing the owner saw with a correct value.

**C. Any `kind:` turns fuzzy matching off for the words beside it.**
Because of step 4, the words become one literal phrase atom
(`parser/boolean.rs:214-231`) matched as a case-smart substring of the name. Ran:

| Typed | Result |
|---|---|
| `pcfg` | finds `parse_config` |
| `kind:function pcfg` | "No symbols match" |
| `kind:function parse config` | "No symbols match" (phrase with a space) |
| `kind:function wrp` in `Space S` | 0 rows; `wrp` alone finds 2 |

### 1.3 Documented vs parsed vs advertised

| | `kind:` | `type:` |
|---|---|---|
| Parsed | yes (`lexer.rs:66`) | yes, alias (`parser.rs:237`, `parser/boolean.rs:188`) |
| `docs/search.md` | lines 57, 72-73: "`kind:function`, `kind:struct`, …"; the value list is never given | absent |
| `Space ?` help | absent from "Query language" (`crates/strop-engine/src/editor/help.rs:47-71`); its "surfaces" row (`help.rs:50`) omits `Space s` | absent |
| Ctrl-Space | key and 8 values (`query/suggest.rs:106-120`) | key offered (`suggest.rs:73`), no values: the value match is on `"kind"` only |
| Picker footer | no filter hint on Symbols (`crates/strop/src/render/picker/hints.rs:136-149`); `ctrl-space suggest` only on Search (`hints.rs:89-94`) | same |
| Empty-field hint | Files and Search only, hardcoded, no `kind:` (`crates/strop/src/render/picker/rows.rs:47-63`) | same |

`parser/boolean.rs:186` calls `type:` "the documented input alias"; it is documented
only in `plans/0063-canonical-search-and-symbols.md:95`.

### 1.4 Vocabulary: six parallel tables

1. `kinds_for_value`, value to kinds: `strop-picker/src/source/symbols.rs:246`
2. Suggestion values: `strop-picker/src/query/suggest.rs:107-116`
3. `KIND_VALUES` in the property corpus: `strop-picker/src/query/properties.rs:158`
4. `lsp_symbol_kind`, LSP name to kind (partial): `strop-engine/src/editor/lsp/routing.rs:583`
5. `short_kind`, LSP name to chip: `routing.rs:599`
6. `SymbolKind::chip` / `lsp_name` / `kind_of`: `strop-syntax/src/symbols.rs:34`, `:49`, `:239`

A seventh, `symbol_kind_label` (`strop-lsp/src/client/api.rs:776`), turns the LSP
number into the string that 4 and 5 parse back. Matching is case-sensitive; chips and
abbreviations are not accepted; tree-sitter capture names reach the user only through
`kind_of`.

### 1.5 Outline and LSP

Both go through the same path. The outline turns each declaration into a `ProtoSymbol`
with `kind: declaration.kind.lsp_name()`
(`strop-engine/src/editor/symbols/outline.rs:63`), and `show` puts either list in
`glue.symbols_candidates` (`editor/symbols/mod.rs:438-461`). The outline emits only
mapped kinds, so cause B cannot occur there; causes A and C occur on both. `Space S`
filters the syntax tier in the source worker (`strop-picker/src/source/mod.rs:149-266`)
and LSP replies in `merge_workspace_symbols` (`routing.rs:313-466`) with the same
evaluator, and has the same three defects.

---

## 2. Inventory

### 2.1 Popups

Fourteen picker kinds exist (`strop-picker/src/lib.rs:117-146`). Help (`Space ?`), git
status/log/history, the undo tree, registers and marks are buffers, not pickers; they
use buffer search (`/`, which takes a leading `case:`).

| Popup | Key | Parses the query? | Where filters apply |
|---|---|---|---|
| Files | `Space f` | yes, `query.rs:263-294` | `FileSelectionPlan` in the walk (`source/mod.rs:68`); ranker for words |
| Search / Replace | `Space /`, `Space R` | yes, `parse_with` (`query.rs:33`) | `FileSelectionPlan` + `ContentPlan` in the grep worker (`source/grep.rs:23-31`) |
| Document symbols | `Space s` | yes, `query.rs:334` | `compile_retained`, engine thread (`routing.rs:468`) |
| Workspace symbols | `Space S` | yes, `query.rs:296` | walk + `ContentPlan` only if Boolean (`source/mod.rs:149`); LSP merge (`routing.rs:383-398`) |
| Jumps | `Space j` | yes, `query.rs:363` | `compile_retained`, engine thread (`picker/open.rs:146`, `:227`) |
| Buffers | `Space b` | no | raw input is the fuzzy needle (`query.rs:377-385`, `lib.rs:376-381`) |
| Diagnostics, Locations | `Space d`, `grr` | no (code) | same |
| Actions, Theme, TabSize, Branches, RemoteHosts | `Space a`, `:theme`, `:tab-size`, `Space g b`, `Space o` | no | same |
| RemoteAddress | | no, by design: an address | none |

### 2.2 Popup × filter

Legend: **ok** works; **silent** accepted, no effect, no message; **lit** the text is
fuzzy-matched literally, so the list empties with no diagnostic; **err** a diagnostic
is shown; **leak** filters partly. "Other" is Buffers, Diagnostics, Locations, Actions,
Theme, TabSize, Branches, RemoteHosts.

| Filter | Files | Search | Symbols `s` | WSymbols `S` | Jumps | Other |
|---|---|---|---|---|---|---|
| bare words | fuzzy path | literal phrase | fuzzy | fuzzy | fuzzy | fuzzy |
| bare words beside `kind:`/`repo:`/operators | **dropped** (ran) | phrase | substring phrase (ran) | substring phrase (ran) | phrase on the line text | n/a |
| `path:` | ok (ran) | ok (ran) | ok, one file (test `navigation.rs:374`) | ok (ran) | ok (ran) | lit (ran: Buffers, Theme) |
| `-path:` | ok (ran) | ok | ok | ok (ran) | ok | lit |
| `fpath:`, `glob:` | ok (ran `glob:`) | ok | ok, different glob engine (2.3 #6) | ok | ok, same | lit |
| `language:` | ok (ran) | ok | ok (test `navigation.rs:293`) | ok (ran) | ok (ran) | lit |
| `case:` | ok (ran) | ok (ran) | ok (code, `query.rs:91-101`) | ok (ran) | ok (ran) | lit (ran) |
| smart case | ok (ran) | ok | ok | ok | ok | ok (default `MatchMode`, `rank.rs:71-75`) |
| `kind:` good value | **silent**, and drops every other filter (ran) | ok per enclosing declaration; files with no outline leak (ran) | ok on the outline; **leak** on LSP (ran) | ok on the syntax tier (ran); leak on LSP (code) | **silent** (ran) | lit (ran) |
| `kind:` bad value | silent | silent (ran `kind:fn wrap`) | **silent** (ran) | **silent** (ran) | silent | lit |
| `type:` | as `kind:` | as `kind:` | as `kind:` (ran) | as `kind:` | as `kind:` | lit |
| `repo:` | silent (ran) | ok with the catalog (code) | silent (no catalog) | syntax tier ok, LSP merge silent (code) | silent | lit |
| `hidden:`, `ignored:` | ok | ok | parsed, no effect | affects the walk | parsed, no effect | lit |
| `text:"…"`, quotes | exact match on the path (ran `"lib"`) | literal | literal becomes the fuzzy needle (`query.rs:85-87`) | same | same | lit |
| `regex:` | ok on the path (ran) | ok | pattern text used as a fuzzy needle (`query.rs:87`) | same; not sent to LSP | same | lit |
| `AND` `OR` `NOT` `( )` | **silent: all files** (ran) | ok | ok, substring atoms | ok | ok (ran) | lit |
| `!x` | literal | literal | literal | literal | literal | literal |
| unknown key `foo:bar` | err (ran) | err | err (ran) | err | err (ran) | lit, no message (ran) |
| `lang:` | err, no suggestion (ran) | same | same | same | same | lit |
| `ext:` | err "did you mean text:?" (ran) | same | same | same | same | lit |

`lang:` and `ext:` do not exist. `ext:rs` suggesting `text:` is a wrong correction
(`parser/diagnostics.rs:46-51` picks any key within edit distance 2).

### 2.3 Inconsistencies

1. **Files ignores every Boolean query.** `kind:`, `type:`, `repo:` or any operator
   sets `boolean` (`parser.rs:186-195`). The flat scope fields are then not filled
   (`parser.rs:233`), there is no free text (`parser.rs:214`), and the Files worker
   reads only the flat fields (`source/mod.rs:68`). Ran: `NOT path:src`,
   `path:tests OR path:src`, `lib AND NOT t` and `path:tests kind:function` each list
   every file. The last one loses the `path:` filter because a `kind:` is beside it.
2. **`kind:` means three things or nothing**: the row's own kind (symbol pickers), the
   declaration enclosing a hit line (Search), nothing (Files, Jumps).
3. **Unknown is indistinguishable from "matched everything".** No picker reports a key
   it cannot judge. `docs/search.md:92` documents "a row a filter cannot judge is
   kept"; that is reasonable per row and wrong when no row can ever be judged.
4. **Two compile entry points.** Symbols and Jumps use `compile_retained`, which
   handles flat and Boolean. Workspace symbols uses `FileSelectionPlan` for flat and
   `ContentPlan::compile` only when Boolean (`source/mod.rs:149`, `routing.rs:388`).
   Files uses `FileSelectionPlan` only.
5. **Five hand-built `Evidence` sites**: `routing.rs:409-417`, `routing.rs:529-537`,
   `picker/open.rs:243-247`, `source/mod.rs:254-263`, and the grep worker. Each decides
   on its own what the path, line and kind of a row are.
6. **Two glob engines.** Files and Search use `globset` (`plan/file.rs:45-58`); the
   evaluator uses a hand-written `glob_literal_match` (`plan/boolean.rs:492`). The same
   `glob:` can decide differently in `Space f` and `Space s`.
7. **Search with only `kind:` matches every line, several times.** Ran: `kind:function`
   alone gives "492 hits · 6 files", the one line of `.gitignore` listed seven times.
   The provider pattern for a metadata-only Boolean query is `()`
   (`plan/content.rs:74-78`). `path:tests` alone correctly says "content search needs a
   text or regex expression" (`query.rs:456-462`), because that check tests
   `content.is_none() && boolean.is_none()`.
8. **Suggestions ignore the surface.** Ctrl-Space offers all twelve keys in every
   parsing picker (`suggest.rs:73-83`) and is not available in the others
   (`query.rs:575-581`).
9. **Docs overclaim.** `docs/search.md:3-5` says "the same filters work in every search
   field". The help's "one language" row (`help.rs:51`) says the same.
10. **Filtering runs on the input thread.** `rebuild_document_symbols` and
    `rebuild_jump_rows` run inside `picker_input_changed`. Per keystroke and per row
    they allocate a `format!("{}::{}")` qualified name (`routing.rs:526`), a
    `path.clone()` (`routing.rs:521`), the row text and the chip `String`
    (`routing.rs:554-562`), after compiling regexes on the same thread. On the bench
    file (2,800 declarations, `crates/strop/src/bench/symbols.rs:4`) that is about
    10,000 allocations per character of a `kind:` value, against AGENTS.md's hot-path
    rule.

---

## 3. Canonical design

### 3.1 What exists

One lexer, one parser and one evaluator already exist: `strop-picker/src/query/`
(`lexer.rs`, `parser.rs`, `parser/boolean.rs`, `plan/boolean.rs`). The defect is not a
second parser. It is that:

- eight pickers never call the parser (`query.rs:377-385`);
- the grammar does not know which surface it is on, so there is nothing to validate a
  key against;
- the evaluator is reached through three plans and five evidence builders;
- Files does not reach it at all for Boolean queries;
- the kind vocabulary is seven tables;
- Unknown-admits hides all of it.

### 3.2 The four pieces

**(a) One kind table, in `strop-core`** (both `strop-lsp` and `strop-syntax` depend on
it). New `crates/strop-core/src/symbol_kind.rs`:

```rust
pub enum SymbolKind { Function, Method, Constructor, Class, Struct, Enum, Variant,
    Interface, Module, Constant, Variable, Field, Property, TypeAlias, Impl, Other }

pub struct KindSpec {
    pub kind: SymbolKind,
    pub value: &'static str,            // "function": what kind: takes and Ctrl-Space offers
    pub chip: &'static str,             // "fn": the row badge
    pub aliases: &'static [&'static str],
    pub lsp: &'static [&'static str],   // "Function"; Module also takes Namespace, Package
    pub includes: &'static [SymbolKind],// function includes Method and Constructor
}
pub const KINDS: &[KindSpec];
impl SymbolKind {
    pub fn from_lsp(name: &str) -> SymbolKind;   // total: an unlisted name is Other
    pub fn chip(self) -> &'static str;
}
pub fn kinds_for(value: &str) -> Option<&'static [SymbolKind]>; // ASCII case-insensitive
pub fn kind_values() -> impl Iterator<Item = &'static str>;
```

Because `from_lsp` is total, every row has a kind, so `kind:` is never Unknown on a
symbol row and cause B is gone. Deleted in the same change: `kinds_for_value`, the
suggestion literal list, `KIND_VALUES`, `lsp_symbol_kind`, `short_kind`,
`SymbolKind::{chip, lsp_name}` and the enum in `strop-syntax`. `ProtoSymbol.kind`
(`strop-lsp/src/protocol.rs:281`) becomes `SymbolKind`; `symbol_kind_label`
(`api.rs:776`) becomes a `lt::SymbolKind -> SymbolKind` match and the outline stops
going through a string. This changes the recorded trace shape; say so in the changelog.

**(b) A per-surface schema: the one declaration.** New
`crates/strop-picker/src/query/schema.rs` (about 200 lines):

```rust
pub enum Field { Path, FuzzyPath, Glob, Language, Kind, Repo,
                 Hidden, Ignored, Case, Text, Regex }
pub enum Bare { FuzzyPath, Fuzzy, Phrase }
pub struct Schema {
    pub fields: &'static [Field],   // empty: a plain list, the field is not parsed
    pub bare: Bare,
    pub operators: bool,
    pub noun: &'static str,         // "files": used in "kind: does not filter files"
}
impl Field { pub fn key(self) -> &'static str; pub fn help(self) -> &'static str;
             pub fn example(self) -> &'static str; }
impl Kind  { pub fn schema(self) -> &'static Schema; }   // exhaustive match, no `_`
```

`lexer::QUALIFIERS` is derived from `Field` (plus the `type` alias, if kept) and the
`unreachable!` at `parser.rs:402` goes. Proposed table:

| Kind | fields | bare |
|---|---|---|
| Files | Path FuzzyPath Glob Language Repo Hidden Ignored Case Text Regex | FuzzyPath |
| Search | all of the above + Kind | Phrase |
| Symbols | Kind Case Text Regex (+ Path Language, taste Q4) | Fuzzy |
| WorkspaceSymbols | Kind Path FuzzyPath Glob Language Repo Hidden Ignored Case Text Regex | Fuzzy |
| Jumps, Buffers, Diagnostics, Locations | Path FuzzyPath Glob Language Case Text Regex | Fuzzy (Buffers: FuzzyPath) |
| Actions, Theme, TabSize, Branches, RemoteHosts, RemoteAddress | none | Fuzzy |

Plain lists stay unparsed on purpose: action titles contain colons, and the Branches
field is a branch name. They show no filter hint, so nothing is promised.

**(c) One parse entry, with feedback.**
`SearchQuery::parse(input, &Schema, SearchModes)` replaces both `parse` and
`parse_with`. New rules:

- A key outside `schema.fields` gives a diagnostic at its span:
  `kind: does not filter files` (`QueryState::Invalid`, like a misspelt key today).
- An unknown `kind:` value gives `unknown kind: fn — did you mean function?`, the same
  shape as `unknown language:` (`parser.rs:296-300`). If abbreviations are accepted
  (taste Q2) `fn` is simply valid.
- `kind:` and `repo:` stop switching the parse mode. Delete `parser.rs:186-195`. They
  lower flat, like `language:`, into `SearchQuery.kinds`, `exclude_kinds`, `repos`,
  `exclude_repos`. Bare words stay `content`, so they stay fuzzy on fuzzy surfaces.
  Boolean mode means the user typed an operator. This removes cause C and half of
  inconsistency 1.
- Operators where `schema.operators` is false give a diagnostic.
- Total: never panics; every input ends `Ready`, `Incomplete` or `Invalid`.

**(d) One matcher over one row view.** `plan/boolean.rs` becomes `plan/admission.rs`;
`BooleanPlan` becomes `Admission`, with one constructor (`compile_retained` renamed
`compile`; `ContentPlan::compile` calls it for its Boolean half). New
`crates/strop-picker/src/query/facts.rs`:

```rust
pub struct RowFacts<'a> {
    pub path: Option<&'a str>,      // workspace-relative
    pub line: Option<usize>,
    pub kind: Option<SymbolKind>,
    pub text: &'a str,              // what content atoms match
    pub container: Option<&'a str>, // no allocated "A::b"
}
impl Item { pub fn facts<'a>(&'a self, root: &'a Path) -> RowFacts<'a>; }
```

`Item` (`lib.rs:107-116`) gains one `Copy` field,
`symbol: Option<SymbolFacts { kind: SymbolKind, name: Range<u32>, container: Range<u32> }>`,
with ranges into `text`. This replaces the text-splitting `item_symbol_name`
(`routing.rs:573`). `facts` takes the path from the payload (`File`, `Grep`, `Remote`)
by borrowed `strip_prefix`. Jump and Buffer rows carry a `path: Option<Arc<str>>`,
because their payload has none.

`Evidence` keeps only what a worker owns (`catalog`, `symbols`).
`Admission::admits(&RowFacts, &Evidence) -> Decision` is the only decision. The
evaluator's `glob_literal_match` is deleted and a `globset` matcher is compiled into
the `glob:` atom, so there is one glob engine.

Where it runs:

- **Retained lists** (Symbols, Jumps, Buffers, Diagnostics, Locations) are filtered in
  the ranking worker. `FilterRequest` (`rank.rs:77`) gains
  `query: Arc<SearchQuery>` and `root: Arc<Path>`; `rank()` compiles the `Admission`
  once per pass, as it already compiles `MatchMode::Exact` (`rank.rs:103-108`), tests
  each item and then scores the survivors. The catalog is never rebuilt.
  `rebuild_document_symbols`, `rebuild_jump_rows`, `document_symbol_items`'s admission
  parameter and `PickerGlue.symbols_candidates` are deleted.
- **Streaming sources** (Files, Search, the workspace-symbol syntax tier) keep
  `FileSelectionPlan` as a sound pruner of the walk, and call the same `Admission` per
  row. Files evaluates Boolean queries with the path as `text`.
- **LSP workspace replies** are appended unfiltered; the ranking pass filters them.
  The admission block in `merge_workspace_symbols` (`routing.rs:383-420`) is deleted.

**Unknown, after the change.** A key the surface does not declare is a parse error. A
declared key that no row can be judged by reports once, in the existing
`picker.warning` line (`lib.rs:205-209`): `kind: no symbol information for these
files`. A row that cannot be judged while others can is still kept, as
`docs/search.md:92` says, and the count reads `N kept unjudged` (taste Q3).

**The hint cannot lie.** All four are generated from `kind.schema()`:

- the empty-field teaching lines (`render/picker/rows.rs:47-63`, today a hardcoded
  string for Files and Search), shown for every non-plain picker;
- the footer: `ctrl-space filters` whenever `fields` is non-empty (`hints.rs:136`);
- Ctrl-Space: `suggest(query, caret, schema)` offers only that surface's keys and takes
  kind values from `kind_values()`; `open_suggestions`' hand list of kinds
  (`query.rs:575-581`) becomes `!schema.fields.is_empty()`;
- the "Query language" section of `Space ?` (`help.rs:47-71`): one row per `Field`,
  with the surfaces that declare it.

### 3.3 Hot-path constraints

- Opening `Space s` from a kept list has a 10 ms p95 budget (`OPEN_KEPT`,
  `crates/strop/src/bench/symbols.rs:23`; 2,800 rows; the release binary is musl, where
  malloc is slow). The design adds one `Copy` field per row and no allocation.
  Optional gain: derive the badge from `symbol.kind.chip()` and drop
  `Item.badge: Option<String>` for symbol rows, which removes one allocation per row.
- Per keystroke: no row is built, no regex is compiled on the input thread, and the
  input-to-render path does parse plus one `Arc` clone. The qualified-name `format!`
  is replaced by two matches (name, then container), or one scratch `String` reused
  across the pass.
- `Space f` per-key budget (`crates/strop/src/bench/find.rs`): the admission test is
  skipped when the query has no row-level atom, so a plain fuzzy needle costs what it
  costs today.
- The ranking worker already polls cancellation every 128 rows (`rank.rs:121`); the
  admission test sits inside that loop.

### 3.4 Files to create, change, delete

Create:

- `crates/strop-core/src/symbol_kind.rs` (~150 lines)
- `crates/strop-picker/src/query/schema.rs` (~200)
- `crates/strop-picker/src/query/facts.rs` (~120)
- `crates/strop-picker/src/query/schema_tests.rs`
- `crates/strop-engine/src/editor/picker/filter_tests.rs` (~400; split by picker if it passes the ceiling)

Rename and reshape:

- `query/plan/boolean.rs` (546 lines) to `query/plan/admission.rs`, about 450 once
  `glob_literal_match` and the duplicate kind arm go.
- `query/plan/tests.rs` is at 788 lines. It must split in this change into
  `plan/tests/{admission,content,file}.rs`, the shape AGENTS.md cites.

Change:

- `query/lexer.rs`, `query/parser.rs`, `query/parser/boolean.rs`,
  `query/parser/diagnostics.rs`, `query/suggest.rs`, `query/modes.rs`,
  `query/properties.rs`, `query/store.rs` (flat `kind:` changes the canonical form:
  bump `SYNTAX_VERSION_CURRENT` to 4 and add the v3 reader)
- `rank.rs`, `lib.rs` (`Item`, `filter_request`)
- `source/mod.rs`, `source/grep.rs`, `source/symbols.rs`
- `strop-syntax/src/symbols.rs`, `strop-lsp/src/protocol.rs`, `strop-lsp/src/client/api.rs`
- `strop-engine/src/editor/picker/{query,open,mod}.rs`,
  `editor/lsp/routing.rs`, `editor/symbols/{mod,outline}.rs`, `editor/help.rs`
- `crates/strop/src/render/picker/{rows,hints}.rs`
- `docs/search.md`, `CHANGELOG.md`, a new numbered plan amending 0051 §3 and 0063 §3/§4

Delete outright (no shims): `SearchQuery::parse_with`,
`BooleanPlan::compile_retained`, `kinds_for_value`, `lsp_symbol_kind`, `short_kind`,
`item_symbol_name`, `glob_literal_match`, `rebuild_document_symbols`,
`rebuild_jump_rows`, `PickerGlue.symbols_candidates`, `SymbolEvidence` (folded into
`RowFacts`).

### 3.5 Order of work

Each step leaves the tree green.

1. **Plan first** (AGENTS.md): the new plan records the answers to section 5.
2. **Kind table.** Add `symbol_kind.rs`; move the enum; migrate the seven tables;
   `ProtoSymbol.kind` typed. The only behaviour change is that LSP rows of every kind
   are judged. Fixes cause B.
3. **Kind values diagnose.** Unknown value is an error with a suggestion;
   case-insensitive; aliases per Q2. Update the three "kind:bogus admits" tests.
   Fixes cause A.
4. **Flat `kind:`/`repo:`.** Remove the mode switch; add the flat fields; extend the
   flat lowering. Fixes cause C and the Files filter loss. Bump the stored syntax
   version.
5. **Schema.** `schema.rs`, `Kind::schema`, the one `parse`; per-surface key
   diagnostics; schema-aware suggestions.
6. **RowFacts and admission in the ranker.** Add `Item.symbol`; move Symbols and Jumps
   filtering into `rank()`; delete the rebuild functions. Run `strop bench symbols`
   and `strop bench find` before and after.
7. **Files and Search.** Files evaluates Boolean queries; the metadata-only Search
   check uses "no positive content atom" instead of `boolean.is_none()`.
8. **Buffers, Diagnostics, Locations join.** Their branch of `picker_input_changed`
   calls `picker_query_eval`; delete the fallthrough for them.
9. **Hints, help, docs** generated from the schema; `docs/search.md` rewritten around
   one table.
10. **Gate:** `docker compose run --build --rm test`, plus the two benches.

Steps 2-4 fix the owner's report and are small. Steps 5-9 are the consolidation.

---

## 4. Tests that pin the contract

Coverage by contract (AGENTS.md): state assertions for grammar, one golden per visual.

**Kind table: `crates/strop-core/src/symbol_kind.rs`**

- `every_lsp_symbol_kind_name_has_a_kind`: all 26 names
- `every_chip_and_alias_names_its_own_kind`
- `kind_values_are_unique_and_lowercase`
- `function_includes_method_and_constructor`

**Schema: `query/schema_tests.rs`**

- `every_picker_kind_declares_a_schema` (the exhaustive match is the real guard)
- `every_field_key_lexes_as_a_qualifier`
- `a_key_outside_the_schema_is_a_located_error`, table-driven over Kind × Field
- `plain_lists_do_not_parse_qualifiers`
- `suggestions_offer_only_declared_keys`
- `type_suggests_the_kind_values` (or `type_is_not_a_key`, per Q1)

**Parser properties: extend `query/properties.rs`** (fixed-seed LCG, no new dependency)

- `arbitrary_input_never_panics_and_settles_into_a_known_state`: exists at
  `properties.rs:82`; run it for every schema.
- `canonical_formatting_is_a_fixed_point_for_ready_queries`: exists at `:102`; add
  flat `kind:`/`repo:`.
- `every_ready_query_uses_only_declared_fields`: new.
- `a_qualifier_never_changes_how_bare_words_match`: new. For a random fuzzy needle `w`
  and any always-true filter `f` on that surface, rows(`f w`) equals rows(`w`). This is
  the property the owner's bug violates.
- `admission_agrees_with_the_reference_interpreter`: exists at `:326`; port its
  evidence to `RowFacts`.

**Per picker: `strop-engine/src/editor/picker/filter_tests.rs`**

One state test per supported key, named `<picker>_<key>_narrows`, and one
`<picker>_unsupported_key_says_so`, driven from a table that iterates
`Kind::schema().fields`. A key added to a schema without a case fails
`every_declared_field_has_a_narrowing_case`. Named cases that must exist:

- `symbols_kind_filters_every_lsp_kind`: Field, EnumMember, TypeParameter rows drop
  under `kind:function`. The regression for cause B.
- `symbols_kind_keeps_fuzzy_words`: `kind:function pcfg` finds `parse_config`. Cause C.
- `symbols_unknown_kind_value_is_an_error_with_a_suggestion`: cause A.
- `symbols_kind_typed_one_key_at_a_time`: feeds characters, not a paste; asserts the
  diagnostic while the value is partial and the filter at the end.
- `symbols_type_alias_equals_kind` (or its removal).
- `symbols_outline_and_server_lists_filter_alike`: one source through both producers.
- `workspace_symbols_server_rows_are_filtered_like_syntax_rows`
- `files_boolean_query_filters_paths`: `NOT path:src`, `path:a OR path:b`.
- `files_kind_says_it_does_not_filter_files`
- `files_filter_survives_an_unsupported_neighbour`: `path:tests kind:function`.
- `search_metadata_only_query_asks_for_text`: `kind:function` alone.
- `jumps_kind_says_it_does_not_filter_jumps`
- `buffers_path_narrows`, `buffers_language_narrows`, `buffers_case_sensitive_narrows`
- `diagnostics_path_narrows`, `locations_path_narrows`
- `theme_field_is_plain_text`, `branches_field_is_plain_text`

Replace `document_symbols_broken_query_keeps_rows_and_shows_the_diagnostic`
(`navigation.rs:319`) and `workspace_symbols_unknown_evidence_and_invalid_query_stay_honest`
(`query_tests.rs:587`): both assert that `kind:bogus` shows every row with no error.

**Hint and help: one golden each**

- `symbols_empty_field_lists_its_filters` (a `TestBackend` cell grid)
- `help_query_language_rows_come_from_the_schema`: every `Field` appears once, with
  the surfaces that declare it.

**Journey:** extend `crates/strop/tests/symbols.rs` with one headless run on a Rust
file: `Space s`, `kind:function`, frame.

**Bench:** `strop bench symbols` stays under `OPEN_KEPT`. Add an op `filter_key` (one
character of a `kind:` value on the 2,800-row list) with a budget, so filtering cannot
move back onto the input thread.

---

## 5. Taste questions for the owner

1. **`kind:` or `type:`?** Today both parse and only `kind:` is documented. Keep
   `kind:` and delete `type:` (one name; alpha; `type:` then errors with "did you mean
   kind:?"), or keep `type:` as a full alias with values and docs?
2. **Abbreviations.** Should `kind:fn`, `kind:meth`, `kind:const`, `kind:mod`,
   `kind:var` (the chips the list shows) work? Should Rust words (`trait` for
   interface, `variant`, `type`)? Recommended: accept every chip and a short alias
   list, case-insensitively, and make the chip and the value the same word wherever
   possible.
3. **A key this picker cannot use.** (a) an error that names the picker and freezes
   the list until fixed, as a misspelt key does today; (b) a warning line while the
   rest of the query still applies; (c) today's silent ignore. Recommended: (a). The
   same question for a per-row gap, such as Search with `kind:` on files that have no
   outline: keep the row and say "N kept unjudged", or drop it?
4. **`path:`/`language:`/`glob:` in `Space s`.** One file, so they keep or drop the
   whole list. Keep them (same keys everywhere) or reject them (nothing useless)?
5. **Does `kind:function` include methods?** Today yes
   (`source/symbols.rs:249`). Constructors too? In Rust, rust-analyzer reports `new` as
   Function and `&self` functions as Method.
6. **Operators on fuzzy lists.** In `parse AND NOT wrap` in a symbol picker, are the
   words fuzzy terms (consistent with typing them bare) or exact substrings (today)?
7. **Buffers, Diagnostics, Locations.** Should they take the filters (recommended:
   they are lists of files and places), or stay plain? Should Diagnostics get
   `severity:error`?
8. **Plain lists.** Is "no filters, typed text is literal" right for Actions, Theme,
   TabSize, Branches, RemoteHosts?
9. **`ext:rs` and `lang:rust`.** Add them as aliases (`ext:` is a common habit), or
   only fix the suggestion (`ext:` should not suggest `text:`)?
10. **`!` as negation.** Today only `-key:` and `NOT` negate and `!x` is literal
    (0063 §3). Keep?
11. **Where the filter list shows.** In the empty field only (as Files and Search
    today), a permanent row, or only behind Ctrl-Space plus the footer hint?

---

## B. As built (0.71)

What landed, and where it differs from §A. The decisions header was
followed throughout; where §A's proposal and the header disagree, the
header won.

**The pieces.**

- `strop-core/src/symbol_kind.rs`: one table of 21 kinds (`Function`
  `Method` `Constructor` `Class` `Struct` `Interface` `Impl` `Enum`
  `Variant` `Module` `File` `Field` `Property` `Variable` `Constant`
  `Type` `Event` `Operator` `Key` `Value` `Other`), each with its value,
  chip, other words, LSP names and what `kind:<value>` admits. All 26
  protocol kinds have exactly one row; `strop-lsp` maps the protocol's
  constants straight to it, `strop-syntax` classifies into it, and
  `ProtoSymbol.kind` is typed. Deleted: `kinds_for_value`, the
  suggestion list, `KIND_VALUES`, `lsp_symbol_kind`, `short_kind`,
  `symbol_kind_label`, `SymbolKind::lsp_name` and the syntax crate's enum.
- `strop-picker/src/query/schema.rs`: `Field`, `Bare`, `Schema`, one
  const per surface and `Kind::schema()` (exhaustive). The lexer's key
  list is `Field::from_key`; `type` is not a key.
- One `SearchQuery::parse(input, schema, modes)`; `parse_with` is gone.
  A key outside the schema is `"{key}: does not filter {noun}"` with
  `"{key}: works in …"` naming the surfaces that take it. `kind:` and
  `repo:` lower flat (`kinds`, `exclude_kinds`, `repos`,
  `exclude_repos`); only an operator or a paren makes a query Boolean.
- `plan/admission.rs`: `Admission` replaces `BooleanPlan`, with one
  constructor (`Admission::compile(query)`: the Boolean AST and the flat
  families as one conjunction) and one decision,
  `admits(&RowFacts, &Evidence)`. `Evidence` holds only what a worker
  owns (the project catalog, the symbol index). `glob:` compiles a
  `globset` matcher; `glob_literal_match` is gone. Two flat exclusions
  now both hold (the old lowering OR-ed them), and `-repo:` excludes
  (the old evaluator negated twice).
- `query/facts.rs`: `RowFacts` (borrowed) and `ItemFacts` (owned once per
  row: a shared `Arc<str>` path, the kind, the ranges of the text that
  name the row). `Item::symbol` is the one symbol-row builder for the
  outline, document symbols, workspace symbols and the syntax tier.
- The ranking pass compiles the `Admission` once per pass from
  `FilterRequest.filter` and tests each row before scoring. Kept lists
  (Symbols, WorkspaceSymbols, Jumps, Buffers, Diagnostics, Locations)
  are never rebuilt per keystroke: `rebuild_document_symbols`,
  `rebuild_jump_rows`, `symbols_candidates` and the LSP merge's admission
  block are deleted. Files keeps the walk's `FileSelectionPlan` for flat
  filters and gives the ranking pass only an operator query.
- Search keeps filtering per hit in its workers; `ContentPlan` holds the
  Boolean `Admission`, or for a flat query an `Admission` only when it
  has `kind:`/`repo:` (the file selection already applied the rest).
  Search now needs a positive content atom (`has_positive_content`).
- Hints, footer, Ctrl-Space and `Space ?` read the schema; the error
  marker (`^^^` under the word and the message) is drawn on the row under
  the field. `docs/search.md` has the per-list table and the kind table,
  and a test holds both to the code.

**Answers taken for §5's questions.**

1. `type:` retired (header). A stored version 3 query's `type:` reads as
   `kind:` (syntax version 4).
2. Values: the value, the chip and the other words in the table, ASCII
   case-insensitive; a prefix is accepted when every word it starts
   names one kind. A prefix naming several (`kind:f`) is Incomplete —
   typing in progress, the list stays — not an error.
3. (a): an error, and the list stays as it was. A row a filter cannot
   judge (no path, no kind, no index) is still kept; the "N kept
   unjudged" count was not built.
4. `Space s` keeps the path family (one file: keep or drop all).
5. `kind:function` admits methods and constructors.
6. Words joined by operators stay exact substrings (unchanged).
7. Buffers, Diagnostics and Locations take `language: path: fpath:
   glob: case: text: regex:`; none of their rows carry a kind, so `kind:`
   is an error there. No `severity:` yet.
8. Plain lists (Actions, Theme, TabSize, Branches, RemoteHosts,
   RemoteAddress) are not parsed.
9. `ext:`, `lang:`, `type:`, `dir:`, `file:` are not keys; the correction
   names the key the list takes (`ext:rs` → `did you mean language:?`).
10. `!` stays literal.
11. The empty field's row lists the everyday filters when the list has
    rows; empty Files and Search keep their teaching lines (generated);
    the footer shows `ctrl-space filters`.

**Deviations from §3.**

- `repo:` is declared by Search only. The ranking pass has no project
  catalog, so on Files and workspace symbols `repo:` could never judge a
  row (Files never applied it; workspace symbols applied it to the
  syntax tier only). Declaring it there would advertise a filter that
  keeps everything.
- `text:`, `regex:` and wholly quoted input rank exactly (substring or
  pattern) on every fuzzy list, not only Files: §A's audit row "pattern
  text used as a fuzzy needle" is gone.
- Content atoms on kept rows match the row's subject (a symbol's name, a
  jump's line, a diagnostic's message, a location's source line) or a
  symbol's container; the qualified `Container::name` string is not
  built.
- The `filter_key` bench op (one character of a `kind:` value on the
  2,800-row list, input to frame) has a 5 ms budget.
