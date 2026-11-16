# 0107 — The breadcrumb: which class, which method

- Status: **in force** — implemented with every §13 default, step 5
  included.
- Source: the owner — "Can we have a breadcrumb so that I know which
  class > method I'm in?"
- Builds on [0098](0098-modeline-design.md) (the modeline's two halves, one
  spacing scale, fixed-width fields for anything that changes while you move,
  one keep-order table), [0099](0099-caching.md) (document symbols kept per
  revision, built ahead of the key, never blocking; a syntax outline where no
  server serves the file), [0066](0066-stable-syntax-highlighting.md) (what is
  already accepted stays on screen through an edit at mapped coordinates) and
  [0088](0088-the-look.md) (four greys, chrome only when it carries
  information). Amends 0098 §3.2 and §3.3 (one new part on the row) and 0099
  §4.1 (a symbol carries the lines it spans). Adds no row, no colour, no
  setting, no key.

## 1. What exists today

**On screen.** Nothing says which function the cursor is in. The modeline
(one row for the whole frame, about the focused pane) shows the file and
`line:col`:

```
  NORMAL    main  crates/strop/src/render/statusline/layout.rs                                      ●1 ▲2   25%  152:9  
```

A pane has a title rule (`─ 1 src/a.rs ● ───`) only when there are two or more
panes (0103); a single pane has no row above its text.

**In the engine.** strop already knows the symbols of the current file, and
already fetches them before anyone asks (0099 §4.1, `editor/symbols/`):

| Fact | Where |
|---|---|
| One list per (document, source, buffer revision), at most 64 lists of at most 20,000 symbols | `symbols/store.rs` |
| Source: the language server when the file is open on a ready one; else the tree-sitter outline (Rust, Python, C, C++, Lua); else nobody | `symbols/mod.rs::source` |
| Built after every event when missing, one build at a time, never in Insert mode, on a job that posts to the event loop | `symbols/mod.rs::warm`, `symbols/outline.rs` |
| Cost: outline 2.4 ms for 806 lines, 51 ms for 15,400 lines (off the loop); a server round trip 1.4–37 ms | 0099 §1 |

What the lists lack is **extent and nesting**. `ProtoSymbol` holds a name, a
kind, a container string and one position (the name's). The outline's
`Declaration` knows `line..end_line` and drops it in `outline.rs`; the server's
`DocumentSymbol.range` is dropped in `flatten_symbols`
(`strop-lsp/src/client/api.rs`). Without a span nobody can say "the cursor is
inside this one".

Two more facts that shape the plan:

- The Rust outline has no `impl` rows (`queries/rust/symbols.scm`), so without
  rust-analyzer a method has no parent to show.
- Go, Java, JavaScript, TypeScript, C#, PHP and Ruby have a linked grammar but
  no symbols query: without a server they have no list at all, and `Space s`
  says so.

## 2. How others do it

- **VS Code, Zed, JetBrains.** A dedicated breadcrumb row per editor, file path
  then symbols, each segment a clickable drop-down, with a kind icon. *Take:*
  outer to inner, left to right. *Leave:* the row, the icons (a font strop does
  not require), the drop-downs (strop has `Space s`).
- **Neovim.** `winbar` (a row per window) filled by nvim-navic or dropbar from
  `documentSymbol`; just as often navic sits in lualine's statusline. navic
  re-asks the server on text change and looks the cursor up in a kept tree on
  every move. *Take:* the lookup is against a kept structure, never a request
  per move. *Leave:* a row per window.
- **nvim-treesitter-context, Zed's sticky scroll.** The enclosing signature
  lines pinned over the top of the text. Rich, but it covers real rows of the
  buffer with rows that are not at that position — against "everything is a
  real buffer" — and costs one to four rows per pane.
- **Helix.** Nothing by default.

## 3. Verdict

**The breadcrumb is one more part on the modeline: after the file, the names
of the symbols that contain the cursor, outer to inner.**

```
  NORMAL    main  crates/strop/src/render/statusline/layout.rs  impl Modeline › fit                 ●1 ▲2   25%  152:9  
```

- The modeline is where strop already answers "where am I" (file, line,
  column) for the focused pane. The owner's question is the same question one
  level finer. It costs no row and adds no chrome when there is nothing to say.
- **One fact, one source: the breadcrumb is the `Space s` list read at the
  cursor.** Same names, same source rule (server, else outline), same build.
  Where `Space s` has a list there is a breadcrumb; where it has none there is
  none. No second parser, no request per move.
- **Nothing moves when the cursor moves** (0098 M4). The breadcrumb has a
  slot whose width belongs to the file, not to the cursor: the text inside it
  changes, the cells around it do not.
- **It survives typing** the way syntax colour does (0066): the last accepted
  structure stays, carried through each edit by the proved edit map, until the
  next exact list replaces it.
- Names only. No kind icon, no `fn`/`class` word, no file segment (the file is
  already its own group, one group to the left).

## 4. The rules

### 4.1 The part

| | |
|---|---|
| Place | left half, after the file group (name, `●`, `[RO]`, `N×`), two cells from it (0098 §3.1: between groups) |
| Text | `outer › inner`, one cell either side of `›` (inside a group) |
| Separator | `›` (U+203A, one cell in every width table; no ambiguity) |
| Slot | `min(widest breadcrumb this file can show, 40)` cells, left-aligned; fixed until the file's symbols change |
| A segment | a symbol's name as the list gives it, at most 32 cells, cut at its end with `…` |
| At top level | the slot is blank |
| No list for this buffer | no slot; the row is today's row |

"Widest breadcrumb this file can show" is computed once when a list is
accepted: for every symbol, its chain's cells. It is the same idea as the
line field being as wide as the last line's number (0098 §3.2).

### 4.2 Inks (no new colour)

| Part | Ink |
|---|---|
| innermost name | `fg.secondary` — the readout you look for, one step under the filename (`fg.text`), level with `line:col` |
| outer names | `fg.muted` |
| `›` and a leading `…` | `fg.faint` |

Never bold, never the accent (0088 rule 2), never a syntax ink.

### 4.3 What gives way

One row is added to 0098 §3.3's table, and the branch gets a second turn, as
the filename already has. The tail of the table becomes:

| Order (first = last to leave) | Floor | Turn |
|---|---|---|
| … diagnostics, search count, indent, percentage | (unchanged) | |
| **breadcrumb** | 8 cells, or the slot if smaller | up to half of the free cells |
| branch | 8 cells of name + marks (unchanged) | up to half of the free cells |
| **breadcrumb again** | | the rest of its slot |
| branch again | | the rest it wants |
| directory | 4 (unchanged) | |
| the pending slot's padding | 0 | |

So: where you are in the code outranks the branch name and the directory, and
nothing else. Everything on the right half keeps its cells. A long branch
shortens to `feature…` before the breadcrumb is cut; the directory is still the
first thing to go. With no breadcrumb on the row the branch's two turns add up
to exactly what it gets today — every existing modeline golden must stay
byte-identical.

When the slot is narrower than the breadcrumb at the cursor:

1. drop whole outer segments, outermost first; one `…` stands for all of them:
   `… › impl Fixture › a_long_test_name`, then `… › a_long_test_name`;
2. if the innermost alone is still too wide, cut its end: `… › a_lon…`;
3. under the floor the part is not on the row.

The innermost name is the last thing to lose a cell.

### 4.4 Which symbols count

- A symbol contains the cursor when the cursor's byte is on one of its lines:
  from the first line of the declaration (attributes and doc comments
  included when the source includes them) to the line of its closing token.
  Whole lines: `0` and `^` on a `fn` line give the same answer.
- Every kind the list has counts: module, class, struct, enum, trait, `impl`,
  function, method — and whatever else a server reports (a constant, a field).
  The list is the authority; the breadcrumb does not filter it.
- Nesting is by containment of spans, for both sources — one rule. A span
  that half-overlaps its neighbour (a sloppy server) is clamped to the parent
  it starts in.
- The primary cursor decides (Visual: the moving end; several cursors: the
  first).

## 5. Targets

Rulers are omitted; every row below was assembled by a script that asserts its
width. File: `crates/strop/src/render/statusline/layout.rs`, 527 lines, widest
breadcrumb `impl Modeline › hints_shown` (27 cells), branch `main`.

### 5.1 Wide (120): in a method, in a free function, at top level

```
  NORMAL    main  crates/strop/src/render/statusline/layout.rs  impl Modeline › fit                 ●1 ▲2   25%  152:9  
  NORMAL    main  crates/strop/src/render/statusline/layout.rs  allot                               ●1 ▲2   25%  152:9  
  NORMAL    main  crates/strop/src/render/statusline/layout.rs                                      ●1 ▲2   25%  152:9  
```

Chip 0–9, branch 12–15, file 18–61, slot 64–90, then the flexible gap.
`impl Modeline` `fg.muted`, `›` `fg.faint`, `fit` `fg.secondary`. Between the
three rows only cells 64–90 differ.

### 5.2 Narrow (80)

```
before
  NORMAL    main  …s/strop/src/render/statusline/layout.rs  ●1 ▲2   25%  152:9  
after
  NORMAL    main  layout.rs  impl Modeline › fit            ●1 ▲2   25%  152:9  
  NORMAL    main  layout.rs  allot                          ●1 ▲2   25%  152:9  
after, branch feature/modeline-design with untracked files
  NORMAL    feature…*  layout.rs  impl Modeline › fit       ●1 ▲2   25%  152:9  
after, d2 pending
 PENDING    main  layout.rs  impl Modeline › fit        d2  ●1 ▲2   25%  152:9  
```

At 80 columns the directory gives its cells to the slot. The slot is 27 cells
in rows 1, 2; 24 beside the long branch; 25 while keys pend — the breadcrumb
(19 cells) is whole in all of them and starts in the same column when a key is
typed.

### 5.3 Very narrow (48)

```
no diagnostics
  NORMAL    layout.rs  … › fit      25%  152:9  
with ●1 ▲2
  NORMAL    …ine/layout.rs  ●1 ▲2   25%  152:9  
```

Ten cells of slot show `… › fit`. With diagnostics on the row fewer than eight
cells are left and the breadcrumb is not shown.

### 5.4 Deep nesting

`mod tests › impl Fixture › a_long_test_name` is 43 cells:

```
slot 40   … › impl Fixture › a_long_test_name
slot 20   … › a_long_test_name
slot 10   … › a_lon…
```

### 5.5 Split windows (100 columns), pane 1 focused, then pane 2

```
─ 1 src/render/statusline.rs ────────────────────┬─ 2 src/render/statusline/layout.rs ─────────────
  98  │   fn gather(editor: &Editor, t: &Tokens) │ 149  pub(super) fn allot(sizes: &Sizes, cells: us
  99  │   │   let buf = editor.buf();            │ 150      let mut granted = [None; PARTS];
  NORMAL    main  src/render/statusline.rs  impl Modeline › gather                       5%   98:9  

─ 1 src/render/statusline.rs ────────────────────┬─ 2 src/render/statusline/layout.rs ─────────────
  98  │   fn gather(editor: &Editor, t: &Tokens) │ 149  pub(super) fn allot(sizes: &Sizes, cells: us
  99  │   │   let buf = editor.buf();            │ 150      let mut granted = [None; PARTS];
  NORMAL    main  src/render/statusline/layout.rs  allot                                28%  149:5  
```

The title rules do not change. There is one modeline and it is about the
focused pane, breadcrumb included; an unfocused pane shows none (§8).

### 5.6 No symbols

```
  NORMAL    main  docs/editor.md                                    12%   40:1  
```

A Markdown file, a folder, a git view, a terminal, a file whose language has
neither a server nor a symbols query: today's row, cell for cell.

## 6. Data and cost

### 6.1 A symbol carries its lines

`ProtoSymbol` gains `span: LineSpan { first, last }` — the first and last line
(0-based, inclusive) of the whole declaration.

- Outline: `Declaration.line − 1`, `Declaration.end_line − 1` (both exist).
- Server, nested reply: `DocumentSymbol.range`; flat reply:
  `SymbolInformation.location.range`. An end at column 0 of a later line does
  not cover that line (the outline's rule, `symbols.rs`).
- A flat reply whose ranges are only the names gives one-line spans: the
  breadcrumb then shows on declaration lines only. That is honest, and it is
  the server's answer.

No committed tape or fixture holds a serialized `ProtoSymbol`; no `serde`
default is added.

### 6.2 The scope index — `editor/symbols/scope.rs` (new)

One index, for the current document, owned by `SymbolState`
(`Editor::analysis.symbols`; no new `Editor` field).

```rust
pub(crate) struct Scopes {
    document: DocumentId,
    producer: Producer,
    list: Arc<Vec<ProtoSymbol>>,   // the kept list: names live here
    spans: Vec<Span>,              // sorted by start, outer before inner
    slot: usize,                   // cells of the widest chain, capped
    revision: BufferRevision,      // the text these bytes describe
    exact: bool,                   // false once an edit was mapped through
}
struct Span { start: usize, end: usize, parent: u32, symbol: u32 }  // bytes
```

- **Build** (`Scopes::build(list, rope)`), when a list for the current text is
  accepted or found kept: `start` is the first byte of line `first`, `end` the
  end of line `last` before its line break (`Rope::line_to_byte`, no text is
  copied). Sort by `(start, Reverse(end))`, one pass with a stack sets
  `parent` and clamps a half-overlap, one pass sets `slot`. O(n log n) on the
  event loop for n ≤ 20,000 (`SYMBOL_LIMIT`); a list over the limit is not
  kept (0099) and gets no index.
- **Lookup** (`innermost(byte) -> Option<u32>`): binary search for the last
  span that starts at or before the byte, then follow `parent` while the span
  ends at or before it. Spans are nested, so the first one that contains the
  byte is the innermost. At most 64 hops; a deeper chain shows nothing.
  No allocation.
- **Edit** (`apply(&[Change])`): every span is mapped in place through
  `strop_kernel::editmap::map_position` — `start` as it is, `end` as
  `map(end − 1) + 1`, so text typed after a closing brace stays outside and
  text typed in a body grows the span. A span that lay wholly inside the
  replaced bytes becomes empty and contains nothing. The map is monotone
  (proved, `EditMap.lean`), so order and nesting survive and the lookup stays
  valid. A change whose revision is not the next one, or a batch where
  `changes × spans > 2²⁰`, drops the index instead (a `:%s` over a huge file
  must not cost the loop 100 ms for a readout).
- **Follow** (`scope::follow(editor)`, after every event, right after
  `symbols::warm`): O(1) when the document, revision, producer and `exact`
  are what they were. Otherwise: the index is for another document or
  another producer → drop it; the store holds the list for the current
  document, producer and revision and the index is missing or not exact →
  build. Nothing here starts a job; `warm` already does.

### 6.3 What each keystroke costs

| Event | Work on the event loop |
|---|---|
| a motion | none. At paint: one binary search and a parent walk (≤ 64) |
| a typed character | n integer maps, n ≤ 20,000, no allocation (a 500-symbol file: under a microsecond) |
| a list lands, or the focused buffer changes | one build: sort and two rope line lookups per symbol |
| anything else | one comparison |

No request is sent, no parse runs and no rope text is materialized on any of
them. The expensive sources (the server, the outline thread) are 0099's jobs,
unchanged.

### 6.4 When the breadcrumb is exact, and when it is carried

- Normal mode: `warm` starts a build after the event that changed the text;
  the exact list lands 3–60 ms later (0099 §9). In between the carried index
  answers.
- Insert mode: `warm` starts nothing (0099 §4.1), so for the whole insert the
  breadcrumb is the structure as of entering Insert, at positions that follow
  the text. Typing inside `fit` shows `fit`; `o` after its closing brace shows
  the parent; a brand-new function is named when Insert ends.
- The store's rule is untouched: a list is still served only for its exact
  revision, and `Space s` never shows carried rows. The index is a separate,
  derived readout, the same standing as 0066's interim colours.

## 7. Edge cases

| Case | Behaviour |
|---|---|
| No server and no symbols query (Go, TS, … today) | no slot. §9 step 5 adds outlines |
| Server starting or indexing | the outline's breadcrumb where the language has one; replaced when the server's list lands. The slot may change width once |
| File just opened | first frame without the slot; it appears when the first list lands (as the branch does when git is discovered) |
| Symbols over 20,000 | no breadcrumb. Nothing else changes |
| Empty list | no slot |
| Cursor between two functions, or above the first | blank slot |
| Rust without rust-analyzer | `fit` alone until §9 step 5 gives the outline `impl` rows; with rust-analyzer `impl Modeline › fit` |
| A name with control characters or line breaks | made printable through the modeline's `printable`, as every other string on the row |
| Undo, redo | ordinary edits: mapped, then replaced by the exact list |
| Reload after an external change, any revision jump | index dropped; rebuilt when the list lands |
| Picker, which-key, `:` card open | the row is the row; the breadcrumb is still the buffer cursor's |
| Collection (several files in one buffer), diff, commit, status, log, folder, help, terminal, scratch | no list, so no slot. A historical file view gets one if `Space s` works there |
| Remote (SSH) file | same rule as local; whatever serves `Space s` serves this |
| Terminal narrower than chip + position | 0098's bare row; no breadcrumb |
| Buffer switch between two large files | one build per switch. If `--bench symbols` shows it over budget, keep an index per document (at most 8) — not before |

## 8. Considered and rejected

- **A breadcrumb row per pane** (winbar, VS Code, Zed). One text row per pane,
  always — it cannot come and go without moving the text — to show something
  that is blank at top level and absent in every non-code buffer. Against 0088
  rule 4.
- **In the pane's title rule.** Titles exist only with two or more panes, so a
  single pane would need a second home: two places, two truncation rules. In a
  three-way split a title has about 20 cells and the name needs them.
- **A breadcrumb for unfocused panes.** Nobody is "in" an unfocused pane. It
  would also need an index per visible document. Revisit only with the title
  question (§13.4).
- **Sticky context lines.** Rows on screen that are not buffer rows at that
  position; one to four rows per pane.
- **A slot as wide as the current breadcrumb.** Every `j` across a function
  boundary passes through top level; on a full row the directory would pop in
  and out and the filename would hop twice per function. 0098 fixed exactly
  this for `line:col`.
- **Walking the highlighter's tree at the cursor.** A second answer to "what
  are this file's symbols", different from `Space s`, living on the analysis
  actor's thread, and nothing for a file whose structure only the server knows.
- **Asking the server per cursor move** (or per typed character). The async
  invariant allows it as a job, but it is a request per keystroke for an
  answer the kept list already has.
- **Kind icons or kind words.** Icons need a font; words (`fn fit`) double the
  width for what the name's ink and position already say.
- **The file as the first segment** (`layout.rs › Modeline › fit`). The file
  is its own group with its own marks; gluing the two makes `●` and `[RO]`
  ambiguous.
- **`::` or `.` as the separator.** Language-specific; `impl Modeline::fit`
  reads as code and is wrong in Python.
- **A setting.** Zero-config; one look.

## 9. Implementation

Five steps, each green on its own; 1–4 are the feature, 5 widens coverage and
can be a second change.

### Step 1 — symbols carry their lines (`strop-lsp`, `strop-engine`)

| Site | Change |
|---|---|
| `crates/strop-lsp/src/protocol.rs` (`ProtoSymbol`, :277) | add `pub span: LineSpan`; `LineSpan { first: LineIndex, last: LineIndex }` beside it |
| `crates/strop-lsp/src/client/api.rs` (806 lines, over the ceiling) | move `document_symbols`, `flatten_symbols`, `symbol_kind_label` to `client/symbols.rs` in this step; `flatten_symbols` emits `range` lines; the flat arm reads `location.range`; one helper applies the column-0 end rule |
| `crates/strop-engine/src/editor/symbols/outline.rs` (`extract`, :61) | fill `span` from `declaration.line`, `declaration.end_line` |
| every test constructor of `ProtoSymbol` (`symbols/tests.rs`, `race_tests.rs`, `picker/lifecycle_traces.rs`, `strop-lsp` tests) | add the field (a one-line span at the symbol's line) |

Tests: nested and flat replies decode the span; an end at column 0 stops one
line earlier; the outline's span for a Python class and its method.

### Step 2 — the index (`editor/symbols/scope.rs`, new, about 250 lines)

`Scopes::{build, innermost, path, apply}` as §6.2. `path(byte)` returns the
chain's symbol indexes outermost first into a caller-supplied small buffer.
Pure: no editor, no channels (the shape of `store.rs`), so the property tests
drive exactly this.

### Step 3 — the owner wiring (`strop-engine`)

| Site | Change |
|---|---|
| `symbols/mod.rs` (`SymbolState`) | field `scope: Option<Scopes>`; `edited(document, changes)` maps or drops it; `closed` drops it; new `pub(crate) fn follow(editor)` (§6.2) |
| `analysis/mod.rs::edits` (:137) | pass `changes` to `self.symbols.edited` |
| `trace/drive.rs` (:86) | call `symbols::scope::follow(self)` right after `symbols::warm(self)` |
| `editor/lsp.rs`, beside `lsp_document_symbols_pub` | `pub fn scope_path(&self) -> Option<ScopePath<'_>>`: `None` without an index; else the names at `self.head()` outermost first and the slot's cells. A thin door into the component — no new `impl Editor` block and no field (the 0088 ratchet) |

Cells are measured in the engine with `unicode-width` over
`strop_core::layout::printable_text`, the measure `render/clip.rs::width`
uses; a test holds the two equal on the fixture names.

### Step 4 — the row (`crates/strop/src/render/statusline/`)

| Site | Change |
|---|---|
| `layout.rs` | `Part::Scope`; `PARTS` 17; `ORDER` as §4.3 (`(Scope,1,2)`, `(Git,1,2)`, `(Scope,1,1)`, `(Git,1,1)` after `Percent`); `sizes()` sets `Size::elastic(SEP, 8.min(slot), slot)`; constants `MIN_SCOPE = 8`, `SCOPE_CAP = 40`, `SEGMENT_CAP = 32` |
| `scope.rs` (new, about 120 lines) | `fit(names, cells) -> shown segments` (§4.3's three rules, a pure function) and the spans with §4.2's inks |
| `statusline.rs` | `Modeline` gains `scope: Option<(Vec<String>, usize)>` gathered from `editor.scope_path()` through `printable`; `spans()` draws the part after the file group and pads it to its grant |
| `statusline.rs` module doc, 0098's row diagram | add the part |

`layout.rs` is 527 lines and `statusline.rs` 506; both stay under the ceiling.

### Step 5 — more outlines (`strop-syntax`; separable)

- `queries/rust/symbols.scm`: `impl_item` as `@impl.decl`, named as
  rust-analyzer names it (`impl Modeline`, `impl Display for Modeline`), so
  the breadcrumb reads the same with and without a server. New
  `SymbolKind::Impl` (chip `impl`, LSP name `Object`); `Space s` and
  `kind:impl` get it too.
- New `symbols.scm` for Go, Java, JavaScript, TypeScript/TSX, C#, PHP and
  Ruby (functions, methods, classes, interfaces, enums, modules), each with
  its arm in `DeclExtractor::grammar` and its member containers in
  `member_context`. This also ends "no symbols" on `Space s` for those files
  without a server. Research helix's `textobjects.scm`/`tags.scm` for the node
  names before writing them (the licence file for helix-derived queries is
  already in the tree).

### Words

- `plans/0098-modeline-design.md` §3.3: one amendment line —
  "*Amended by [0107](0107-breadcrumb.md): the breadcrumb sits between the
  percentage and the branch; the branch takes two turns.*"
- `plans/0099-caching.md` §4.1: "*Amended by 0107: a symbol carries its line
  span; the modeline reads the kept list at the cursor.*"
- `plans/STATUS.md`: 0107 enters as proposed, then in force with the release.
- `docs/editor.md`: one sentence where the modeline is described.
- `CHANGELOG.md`: "The modeline shows the class and method the cursor is in."
- No key, so nothing to add to `?`.

### Gate

`docker compose run --build --rm test`. `strop-kernel` is not changed, so
`proofs/` is untouched (§11). `strop --bench symbols` gains the two rows of
§10.5.

## 10. Tests

### 10.1 State — the index (`symbols/scope_tests.rs`)

1. **Lookup against the obvious answer.** 2,000 seeded random nested span
   sets (up to 200 spans, depth up to 12, shared starts and ends, empty
   spans); for every byte, `innermost` equals a linear scan for the smallest
   containing span.
2. **A half-overlapping span is clamped**, and the lookup still agrees.
3. **Edits keep the index valid.** Random edits (insert, delete, replace, at
   span boundaries) through `apply`: after each, spans are sorted, nested and
   inside the buffer; a byte typed inside a body is inside its span; a byte
   typed after the closing line is outside.
4. **Carried equals rebuilt where it must.** Rust fixture through the real
   outline: after an edit that adds lines above a function, inside it, and
   after it, the carried index gives the same breadcrumb at every byte of the
   unedited text as an index built from the new outline.
5. Revision gap, the work cap, and close each drop the index.

### 10.2 State — through the editor (`symbols/tests.rs` is 549 lines: put these in `symbols/tests/scope.rs` and move the file to `symbols/tests/mod.rs`)

1. `impl A { fn f() {} }` + `fn g() {}`: the path is `[impl A, f]` in `f`,
   `[impl A]` on the `impl` line, `[g]` in `g`, `[]` between them — with a
   server list, and with the outline (step 5).
2. A motion starts no build and sends no request (`pending()` false, the LSP
   outbox empty) while the path changes.
3. In Insert mode, typing a line inside `f` keeps `[impl A, f]`; `o` on its
   closing brace gives `[impl A]`; after `Esc` and the build, a new `fn h` is
   named.
4. Two panes on two files: `ctrl-w w` changes the path to the other file's;
   back again restores it without a build (the list was kept).
5. Server becomes ready after the outline: the path's producer changes and the
   names are the server's.
6. A collection, a folder, a diff, a terminal: `scope_path()` is `None`.
7. `nothing_waits_for_a_build_that_is_stuck` (0099) still passes with the
   index in place: with the outline held on its gate, keys move and edit and
   the path is the carried one.

### 10.3 The allocator and the cut (`statusline/tests.rs` is 615 lines: new `statusline/scope_tests.rs`)

1. 0098's allocator properties hold with `Part::Scope` in the table (every
   cell accounted for, floor ≤ grant ≤ natural, lowest leaves first).
2. With no breadcrumb, `allot` gives the branch exactly what the one-turn
   table gave it, for every width 1–200 (compare against the old order kept in
   the test).
3. **Moving the cursor moves nothing else**: same file, cursor in `fit`, in
   `allot`, at top level — every cell outside the slot is identical, at 48,
   80, 120 and 200 columns.
4. Typing `d`, `2` leaves the breadcrumb's first column unchanged.
5. `fit(names, cells)`: the ladder of §5.4; the innermost is the last to be
   cut; the result never exceeds `cells`; a 33-cell name is cut to 32 with `…`.
6. The row fits at every width from 1 to 200 with a 43-cell breadcrumb.

### 10.4 Golden frames (`TestBackend`, `render/golden/`)

New family `modeline-scope-*` in `golden_modeline_tests.rs`, hone, 12 rows,
widths 48, 80, 120, 200, each with 0098's long branch and three diagnostics;
the buffer is a small Rust file driven through the real outline
(`recorded_action` + `wait_analysis`, as `golden_tests.rs` already waits):

| Scene | Shows |
|---|---|
| `modeline-scope-method` | `impl Modeline › fit` |
| `modeline-scope-top` | the blank slot |
| `modeline-scope-deep` | a 43-cell chain and its cut |
| `modeline-scope-pending` | `"a3d` pending beside it |
| `modeline-scope-insert` | Insert mode after typing a line: the carried breadcrumb |

Plus `split-scope-{hone,vellum}-{140x40,80x24}`: two files side by side, the
second focused — titles unchanged, the modeline's breadcrumb is the second
file's. Every existing golden must be unchanged; a diff in one is a bug
(`modeline-*` use a file with no declarations).

### 10.5 Budgets (`strop --bench symbols`, the 15,400-line, 2,800-declaration file)

| Operation | Budget (p95) |
|---|---|
| a motion's frame with the breadcrumb on the row, against the same frame without | + 0.2 ms |
| a typed character while the index is carried | 5 ms (0099's "a key while a build is in flight") |
| index build (reported, not budgeted) | — |

### 10.6 Binary journey (`crates/strop/tests/symbols.rs`)

The shipped `strop --headless`, empty `PATH`: open a Rust file, move into a
method, the frame's last row contains `› ` and the method's name; move to top
level, it does not.

## 11. The kernel

**Nothing moves into `strop-kernel`, and no theorem is added.**

- The one piece of arithmetic that must not be wrong while typing — how a
  span's ends move through an edit — is already there and proved:
  `editmap::map_position` (monotone, in bounds). The index calls it; it does
  not copy it.
- The lookup is fifteen lines over integers and would translate cleanly, but
  a wrong answer is a wrong word on a readout: nothing is written, moved or
  deleted by it. Ring 0 is for decisions whose failure corrupts text or
  state. The model test of §10.1 (every byte, against a linear scan) is the
  cheaper check the ring design asks for here.
- If the owner wants it proved anyway (§13.7): move `innermost` to
  `strop_kernel::scope` over three flat slices (`starts`, `ends`, `parents`),
  state `innermost_spec` (the result contains the byte and no contained span
  starts later), add it to `Audit.lean` and one mutant (follow `parent` while
  `end < byte` instead of `≤`). About a day, and `proofs/Extracted/` is
  regenerated.

## 12. Not in this plan

- A breadcrumb inside a collection (the excerpt's own file and function).
  It needs the view→source position map and an index per source file.
- Markdown headings, JSON/YAML/TOML keys as breadcrumbs. Servers give some of
  these today; an outline for them is a query each.
- Building the list during a long Insert (when the loop goes idle), so a
  brand-new function is named before `Esc`.
- Jumping with the breadcrumb (`[[`-style "go to the enclosing function"
  motions read the same index; they are grammar and belong to 0095's list).
- An index per unfocused pane.

## 13. Taste questions for the owner

1. **The modeline, or a row of its own?** Plan: the modeline — no row spent,
   nothing drawn when there is nothing to say. A row per pane is the VS Code
   shape and costs a text row in every pane, always.
2. **`›`, or the `>` you typed?** Plan: `›` in `fg.faint`, one cell either
   side. `>` is ASCII and louder; it is one constant.
3. **A fixed slot, or a tight one?** Plan: fixed per file (at most 40 cells),
   so nothing shifts while you move. The cost shows at 80 columns: the
   directory is hidden even while the slot is blank at top level. A tight slot
   shows the directory then, and makes the filename hop on every function
   boundary.
4. **Should the breadcrumb outrank the branch?** Plan: yes — a long branch
   shortens to `feature…` first. The alternative keeps the branch whole and
   cuts the breadcrumb to `… › fit` sooner.
5. **`impl Modeline › fit`, or `Modeline › fit`?** Plan: the name as
   rust-analyzer gives it, so `Space s` and the breadcrumb say the same words.
   Stripping `impl ` is a Rust-only special case.
6. **Everything the list nests, or only classes and functions?** Plan:
   everything — on a struct field the breadcrumb is `Modeline › chip`; a
   server that reports local variables adds them. Filtering to
   module/class/function kinds is a five-line table if that is noise.
7. **Is step 5 (outlines for Go, TypeScript, Java, …; `impl` rows in Rust)
   part of this, or the next change?** Plan: the next change — the feature is
   complete for every file a server serves and for the five outline languages.
8. **Prove the lookup in Lean?** Plan: no (§11).
