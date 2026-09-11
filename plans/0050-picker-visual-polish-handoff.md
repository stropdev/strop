# 0050 — Picker visual polish: symbols, files and grep

Status: **implemented in 0.28.0** (11 Sep 2026): per-kind rows (fixed
quiet chips, basename-first files, two-line grep with the real submatch
window), full-row selection band, visible/total counts, explained empty
states, responsive list-first layout, reserved scrollbar column,
filename-first preview headers, and the render/picker/ split. Verified
on TestBackend cell/style tests plus live drives at 100 and 140 columns
with clangd and rg.

Reviewed Strop **0.26.0**, commit
`fc73714feb679ca59c4599eea56cef9144a04c5d`. Freeze/reconcile the implementing
session's newer changes before applying this advice. The 0049 correctness and
navigation work has since shipped in substantial part; do not reopen its old
baseline findings as if nothing landed. Multibuffer cards/source-syntax work
in 0049 remains separate from these quick-picker result rows.

## 1. Design decision

**Keep the kind chips. Make names, matches and selection more prominent than
the taxonomy. Give each result kind a meaningful row layout.**

The current picker is recognizable and its basic navigation works. Its weakness
is information hierarchy, not a shortage of bright colors. It renders nearly
every result as an optional colored badge followed by one clipped string.
That cannot simultaneously serve a symbol, a file path and a source-code hit.

A result should answer four questions without making the user open it:

1. **What is it?** Symbol name, filename, or matching code.
2. **Which one?** Container, parent directory, source location or namespace.
3. **Why did it match?** Visible query evidence in the correct field.
4. **What will Enter open?** An unmistakable selected logical result.

Preserve the floating card, dim backdrop, keyboard behavior, stable ranking,
source identity and real preview. Do not turn this into a theme engine, symbol
tree, new search language or a box around every candidate.

## 2. What I actually drove

Used a frozen copy of the installed binary, SHA-256:

```text
7649b4bbb4953f9cf3d7348ba9f54faf099253395c54042b3a1fb0272a404a09
```

Ran the **real TUI on a Linux PTY**, not only `--headless`. A private C++
workspace contained namespaces, fields, methods, constructors, constants,
enums, qualified definitions, duplicate `client.cpp` filenames, long parent
paths and a `naïve_client.cpp`. Actual **clangd 18.1.3** produced the symbol
results; file discovery and grep used Strop's real sources and `rg`.

Exercised:

- `Space s`: 33 real symbols; filter `retry`, move with Tab, accept a method.
- Symbol filtering with no matches (`zzz`), and Escape's two modal transitions.
- `Space f`: filter `client`, inspect duplicate/long paths; later filter
  `naïve` and open the actual Unicode-named source.
- `Space /`: search `retry_request`, then `retry_request -t cpp`; move and
  open a real grep hit.
- Resize while the pickers were open: **140×40, 100×30 and 80×24**.
- Clean quit: exit 0; final trace record reports `complete: true`.

Source acceptance worked: the selected method opened at source line 24;
a grep hit opened at its recorded match offset; the Unicode file opened as
itself. This review is not claiming the navigation feature is broken.

### Visual evidence

The driver saved the TUI's **actual styled cell observations** from its live
trace and reconstructed them as SVG images using a monospaced font. These
are faithful cell/color captures, **not photographs of Windows Terminal**;
font rasterization and the native blinking cursor can differ.

All fixtures, HOME/XDG state, captures and raw terminal output remain under
`/tmp/strop-picker-polish/`. The completed session is `live2/`, particularly:

| Capture | What to inspect |
| --- | --- |
| `symbols-wide-settled.svg` | Filled chip column, uneven name starts, floating line numbers, long preview title |
| `symbols-standard.svg` | Same hierarchy at 100 columns |
| `symbols-retry-wide.svg` | Nine filtered rows while the header still says 33; match styling versus chip weight |
| `symbols-narrow.svg` | Name/location truncation at 80 columns |
| `symbols-empty.svg` | Blank results with a misleading 33 in the corner |
| `files-client-wide-settled.svg` | Undifferentiated path strings and long-path clipping |
| `files-client-standard.svg` | Selected result's basename/match clipped off the left list |
| `files-unicode-settled.svg` | Real Unicode filename filtering; preserve this correctness |
| `grep-wide-settled.svg` / `grep-standard.svg` | Path prefixes dominate; actual code match has no list emphasis |
| `grep-cpp-narrow.svg` | Nearly identical truncated prefixes and scrollbar over the final text cell |

Each has a `.json` cell/state capture and `.txt` counterpart.
`live2/session.jsonl`, `live2/terminal.bin`, `drive.py`, and
`evidence-summary.json` retain the experiment. Some small search fixtures are
not complete C++ programs; their undeclared-function diagnostics after opening
are fixture diagnostics, not picker regressions. No project-wide gate was run
for this documentation-only review.

## 3. Findings and priority

| Priority | Finding | Evidence / source |
| --- | --- | --- |
| P1 | **Grep does not paint its real match in the result list.** | `Picker::filter_request` marks grep/replace `upstream_filtered`; `rank.rs:90–104` leaves fuzzy columns empty for those rows. `render_results` paints only those columns. The `Payload::Grep` already has original line text, byte column and match length. |
| P1 | **Clipping hides result identity and query evidence.** | All rows use `clip_end(item.text)`. At 100 columns the result list has only 34 cells; `tests/integration/transport/http…` hides `client.cpp`. Grep commonly hides the entire matching expression. |
| P2 | **Kind chips overpower symbol names and create a ragged name column.** | Filled pastel backgrounds plus bold dark text; badge width is label length + 2. `ns`, `class`, `field`, `meth` start the names at different columns. |
| P2 | **Selection is a fragment, not a row.** | Background is applied only to emitted text characters, not the marker, kind area or trailing space. Short names produce short selected islands. |
| P2 | **Filtered counts and empty states are misleading.** | `retry` gives nine symbol rows but still displays 33; `zzz` leaves a blank card also labeled 33. Header count uses catalog size rather than visible ranking size. |
| P2 | **Metadata is undifferentiated.** | Names, owner qualifiers, path segments, punctuation and line numbers share one base foreground and one string layout. |
| P2 | **One unconditional 42/58 split starves the decision list.** | Measured symbol result budgets: 47 cells at 140 terminal columns, 34 at 100. At 80, useful names/code disappear. |
| P2 | **Scrollbar can overwrite the last content cell.** | It paints at `results.x + results.width - 1` after text was allowed to consume that same cell. In narrow grep it hides clipping evidence as well as content. |
| P2 | **Preview identity and hints lose useful information first.** | Symbol preview starts with a long absolute `/tmp/...` prefix; the filename/location is clipped. Long footer prose is cut through hints at narrow widths. |

Syntax in the right preview works when its owned analysis result is ready.
A transient unstyled first preview frame is not proof of missing syntax.
The durable issue here is that the left list does not communicate enough.

## 4. Shared visual language

### Roles, not decorative coloring

Use these roles consistently across symbols, files and grep:

| Role | Treatment |
| --- | --- |
| Primary identity / code | Existing `TEXT`; readable without selection |
| Useful secondary context | A readable secondary foreground, between `TEXT` and `MUTED` (initial palette candidate `#9ba0b1`) |
| Separators / low-value chrome | Existing `MUTED`; do not use this faintest role for the only path disambiguator |
| Query match | Existing amber `ACCENT` + bold; preserve selection background |
| Selected logical result | Existing `SELECT_BG` across the **whole allocated row/block**, including blanks, with `▌` at a fixed gutter position |
| Kind information | Quiet semantic foreground, optional very subtle tinted fill; never the current fully saturated light background |

Keep amber for query/focus/action emphasis. Four quiet kind families are
enough: callables blue, types/enums mauve, data/fields green,
modules/namespaces teal; unknown kinds neutral. Reuse existing palette values
where appropriate, not one new color for every LSP enum member.

Do not dim metadata into illegibility, rely on color alone, require Nerd Fonts,
or paint filenames different colors merely because their extensions differ.
The kind label and path context must still work in monochrome.

### Selection/highlight precedence

Paint the selected row's entire background first, excluding the reserved
scrollbar column. Then apply field foregrounds and match emphasis. On a
selected row, let the kind foreground sit on the same selection band rather
than punching a bright badge-shaped hole through it. Match emphasis changes
foreground/weight only; it must not erase selection or the rest of the row.

This is one focused result, not three unrelated selected spans.

## 5. Symbol picker: retain chips, restore scanning rhythm

### Target row

Schematic; bracket notation below illustrates the badge, not required literal
bracket glyphs:

```text
symbols — request_dispatcher.cpp                              9 / 33
▌ [meth]  retry_request                  RequestDispatcher       :24
  [meth]  retry_request_with_deadline    RequestDispatcher       :25
  [field] retry_policy_                  RequestDispatcher       :30
  [fn]    calculate_retry_backoff       detail                  :61
```

Required behavior:

- A **fixed kind slot**, initially 9 terminal cells including padding, handles
  the current longest label (`variant`) without shifting symbol names. Use
  readable existing labels; do not replace `field`/`ns` with cryptic single
  letters merely to save space.
- Names share one left edge. Kind labels have a consistent footprint and
  quieter weight; the name and query match win the first glance.
- Location occupies a stable right-side slot when present, not punctuation
  appended immediately after every differently sized name.
- Container/qualifier/detail is a separate secondary field. Drop low-value
  detail before truncating the identifying name. Where overloads or repeated
  names need context, retain a disambiguator or use a secondary line rather
  than show indistinguishable rows.
- Preserve namespace/owner information actually supplied by the server.
  Current clangd results include some qualified names and omit some container
  fields. Do not fabricate hierarchy by splitting arbitrary `::` strings or
  infer a different symbol kind from the preview's source keyword.
- Keep the flat fuzzy navigation workflow. A symbol tree, kind-filter UI and
  new LSP queries are not prerequisites for this polish.
- Chips are passive type information, not buttons. Do not style them like
  enabled filters unless a separate, real filtering interaction is implemented.

The preview header should identify `request_dispatcher.cpp` and the selected
source line before parent/root details. Show fuller qualification there when
it could not fit the candidate row. Preserve source syntax and mark the
selected location; add source line numbers through the existing layout path,
not another text renderer.

## 6. File finder: filename first, path as evidence

### Target row

```text
files                                                          9 / 12
▌ client.cpp                 tests/integration/transport/http
  client.cpp                 tests/unit/transport/http
  client.cpp                 src/transport/http
  client.hpp                 include/transport/http
  naïve_client.cpp           src/transport/http
```

- **Basename first in the primary role**, parent/workspace context in the
  secondary role. An extension belongs to the basename; no extra extension
  badge is needed to repeat it.
- Repeated basenames must remain distinguishable. Prefer a useful unique
  parent suffix; middle-elide low-value components, not the only `unit` versus
  `integration` distinction. If the two fields cannot both fit, wrap the
  context to one secondary row instead of hiding identity.
- Preserve fuzzy scoring semantics and canonical path identity. A query may
  match parent directories; visibly emphasize those matches in the context
  field. Do not silently switch to basename-only filtering to simplify colors.
- When a path matched but the entire matching segment would be clipped, favor
  a match-bearing context window/second line. A returned result should explain
  itself rather than appear unrelated to the query.
- The selected preview header exposes the full useful identity, relative to
  the request's workspace where appropriate. Explicit external/remote
  provenance remains visible; no local filesystem probes during formatting.
- Keep paths native in payloads. The display layout is never parsed back into
  the file to open.

Use the existing printable/grapheme-safe clipping machinery. `clip_start`
already exists alongside `clip_end`; do not write another byte-slicing path
shortener or assume an ellipsis is harmless to match indices.

## 7. Grep: show the match, not just a path that contains one

Grep rows need their own presentation, not the file row with `:line · text`
concatenated onto it.

At standard/narrow decision-list widths, use a compact two-line hit:

```text
▌ client.cpp                         tests/unit/transport/http
  1:35  void retries_a_failed_request() { retry_request(2); }
  request_dispatcher.cpp              src/transport
  24:10 bool retry_request(RequestId request_id);
```

The source match is amber/bold in the code line; numbers are secondary;
filename is primary; directory is secondary. The selected background covers
both display rows. These are logical hits, not extra independently selectable
lines.

- Build the code window **around the actual `rg` submatch**, with enough
  surrounding context to explain it and ellipses when clipped. Do not take
  only the first 80 characters of a long line when the match is later.
- Use the original `line_text`, byte-based `col - 1` and `match_len` from the
  payload. Account for indentation removal/window origin before mapping to
  display cells. Never fuzzy-match `retry_request -t cpp` against the snippet
  as a substitute for the real regex result.
- Keep regex semantics, case behavior, filters and one-hit-per-submatch
  navigation unchanged. If multiple submatches share a line, identify the
  current hit precisely; do not silently merge distinct selectable results as
  a cosmetic optimization.
- A wide layout may present a compact single row only if filename/location and
  a useful match-bearing code window all fit. Otherwise keep the two-line
  contract. Do not let preview width consume the evidence on the left.
- Code context can begin as readable neutral text with exact match emphasis.
  Reuse source-owned syntax spans when already available; do not launch parsing
  or filesystem work for every row during render, and do not invent a regex
  syntax-colorizer. The persistent multibuffer's stronger source-syntax
  requirement in 0049 is unchanged.
- Showing a column requires the correct coordinate conversion. The payload's
  byte offset is not a terminal-cell column under tabs/Unicode. Navigation
  keeps the original source offset; display uses the existing line-layout
  contract or clearly labels the coordinate domain.

**Do not add a card around every quick-picker hit.** Rootle-style file cards
are useful in the persistent collection/expanded preview. Inside a floating
picker, compact semantic rows give more usable candidates with less border
noise. Any adjacent file grouping must preserve logical item identity and
ranking; a renderer must not reorder results behind the selection model.

## 8. Layout, count and state behavior

### Give the decision list a usable budget

Replace the unconditional 42/58 split with a small per-kind layout policy.
Keep one floating card over a stable editor backdrop.

Initial targets to verify on the actual surface:

- At **140 columns**, give symbols/files about 60% of the inner width and grep
  about 65%, leaving a useful source preview. This is a starting ratio, not a
  substitute for measuring names and context.
- At **100 columns**, preserve at least roughly 46 cells for the list rather
  than today's 34. Secondary row content may wrap/drop by its priority; kind,
  primary identity and source location must remain usable.
- At **80 columns**, do not force two unreadable vertical slivers. Use a
  full-width result list with a short preview below when the height supports
  both. With very little height, keep results plus a selected-location summary
  rather than a misleading empty/code-fragment pane.
- Maintain minimum visible result rows. Do not resize the card on every query
  character; changing geometry while choosing is distracting.

This deliberately amends the rigid right-preview arrangement in 0003 only
for constrained layouts. It does not convert pickers into editor splits.
Derive hit rectangles, viewport visibility and scrollbar range from actual
rendered row heights. Arrow/Tab/Enter still operate on logical items; headers
or a second display line cannot change which result is accepted.

Reserve the scrollbar column **before** calculating text budgets. Do not
paint it over a filename, match, line number or clipping ellipsis.

### Honest states

- For a completed filtered symbol/file ranking, show **visible / total**:
  `9 / 33`, not just `33`. Keep units clear; grep counts real matches and can
  report distinct files when that count is cheaply maintained at ingestion.
- For zero filtered rows, show `No symbols match “zzz”` / `No files match…`
  and a useful editing hint. Do not leave a blank card with a nonzero count.
- Distinguish no matches, still loading, source failure and cancelled work.
  Reuse existing source/ranking states and the no-spinner-before-100ms policy;
  no new retries or sleeps. A stale ranking count must not pretend to describe
  the newest query while its result is pending.
- Keep errors actionable and visible without squeezing a long error message
  into the count slot. Do not remove useful partial results merely to show it.
- Footer hints depend on actual input ownership: for query input, Escape enters
  result/normal handling; there, Escape closes and `i` returns to filtering.
  Use the actual implemented key semantics. Fit **whole hint groups** by
  priority, never cut through `j/k after esc` or a key chord.
- Always preserve the primary accept/back hints; grep's `Ctrl-O collect` is a
  high-value next hint. Secondary aliases can disappear first at narrow widths.

## 9. Implementation boundaries

### Do not paint by reverse-parsing strings

Current seams:

- `crates/strop-engine/src/editor/lsp.rs:217–275,803–823` flattens symbol name,
  container and line into `Item.text`, and kind into `badge: Option<String>`.
- `crates/strop-picker/src/source/mod.rs:38–79` supplies file paths.
- `source/query.rs:43–85` supplies real grep submatch geometry and a separately
  trimmed/truncated display string.
- `strop-picker/src/lib.rs:336–356` and `rank.rs:74–122` own canonical matching
  indices. These are source-text scalar indices, not final terminal columns.
- `crates/strop/src/render/picker_card.rs` owns card layout, generic rows,
  kind colors, replacement rows and source preview.

Carry structured presentation fields or source ranges at item construction:
primary label, secondary context, typed kind/role, location and real match
geometry. Keep the canonical search text and an explicit mapping from its
match positions to displayed fields. Basename-first ordering, clipping and
inserted ellipses must not move emphasis onto unrelated characters.

Do not parse `name  container · :line`, split arbitrary C++ names on `::`, or
reconstruct a path from its colored/truncated label. Style by a semantic kind,
not by the spelling of a badge that may later change.

An important existing distinction: symbol/LSP-location items also use
`Payload::Grep` as a navigation carrier, sometimes with empty source text and
`match_len = 1`. **That is not an `rg` submatch.** Choose the presentation and
highlight source from the real picker/item kind; do not apply grep coloring
to a fabricated one-byte symbol match.

Keep preparation out of render. Cache immutable field boundaries/presentation
facts with the catalog where useful, borrow source text/spans, and render only
visible rows. Avoid adding per-character Strings or retaining another full
copy of every grep line just for style. Preserve the current owned ranking,
preview and source-job paths, stale-result checks and native path identities.

### Split by responsibility, not a framework

`picker_card.rs` is already about 560 lines. If the new row layouts push it
upward, use one component directory, for example:

```text
render/picker/
  mod.rs       card, input/status and composition
  layout.rs    width/height allocation and logical-row viewport mapping
  rows.rs      symbol/file/grep field and highlight composition
  preview.rs   existing source preview and its header/gutter
```

Keep plain/code-action/remote/replace consumers working through explicit row
variants. Remove the old generic rendering route for migrated kinds rather
than leaving an old/new pair. No new crate, theme subsystem or duplicate
picker state machine. Run symbol references before changing exported item
contracts and migrate all constructors, consumers and applicable trace formats.

## 10. Reference patterns and what to borrow

- **Zed outline** shows type prefixes next to symbol names, helping identify
  kinds without turning each row into a row of bright buttons. Its documented
  outline image reinforces code-like name/context hierarchy. Borrow readable
  type information; do not copy GUI typography or introduce a tree here.
- **VS Code** explicitly separates description foreground, query-highlight
  foreground, selection backgrounds and symbol-icon foregrounds. Borrow those
  role distinctions. Category color and active selection are different signals.
- **Helix** treats picker columns as meaningful fields and distinguishes fuzzy
  pickers from regex global search. Borrow structured fields and honest matching
  semantics; do not add its `%column` query language as part of this polish.
- **Rootle** contributes clear path/metadata separation, source-number gutters,
  match-over-syntax composition and stable selected identity. Reuse these
  principles and existing Strop excerpt/text helpers; do not nest its large
  file cards inside every compact candidate row.

## 11. Concrete implementation handoff

One integration owner should settle presentation-field/highlight mapping and
logical row geometry first. Symbol and file/grep styling can then be developed
independently against that contract; serialize their shared item/renderer
cutover, and run validation after the combined change settles.

Suggested implementation order:

1. Fix real grep match highlighting, identity-preserving file/snippet clipping
   and the reserved scrollbar column. These affect whether a result can be
   understood at all.
2. Add structured field roles, fixed/quiet symbol chips and full-row selection.
   Preserve search ranking and open targets while changing appearance.
3. Add result-dominant responsive layout, honest filtered counts/empty states,
   useful preview headers and width-aware contextual hints.
4. Drive the complete surface again at all three recorded sizes. Do not call
   this done from an isolated chip-rendering test or a wide-only screenshot.

Acceptance:

- With no preview inspection, distinguish duplicate `client.cpp` results,
  identify a long symbol, see where a regex match occurred, and tell exactly
  which logical result Enter will open.
- Name starts align across `ns`, `field`, `meth`, `struct` and `variant` rows.
  Chips remain recognizable without dominating; selected-row backgrounds have
  no ragged gaps, and query highlights survive selected/unselected states.
- A filtered nine-of-33 symbol set reports nine-of-33; a zero set is explained.
- Long paths preserve basename and a necessary disambiguator. Long grep lines
  expose the actual match, including indented/tabbed/Unicode text and multiple
  submatches on one line. Matching directory text remains visibly emphasized.
- Arrow/Tab/Page navigation, Enter, Escape, Unicode query editing and grep's
  `Ctrl-O collect` preserve existing behavior under wrapping/resizing. Scrollbar
  and ellipses never overwrite result evidence.
- Use focused TestBackend cell/style regressions for these contracts, plus a
  real TUI pass with clangd and `rg`. Text-only snapshots do not establish color,
  contrast or a full-row selection band. Do not assert private helper copies.
- Preserve parser/clip/namespace safety, readonly and error states, replacement
  highlighting and remote/code-action picker behavior. Do not re-search text or
  launch services in drawing code.
- Run `docker compose run --build --rm test` for implementation. Record actual
  visual evidence, amended 0003/0047/0049 scope where needed, and the existing
  changelog; do not claim this review ran that gate or implemented these changes.

## Sources

- [0003 house style](0003-modes-leader-and-house-style.md),
  [0047 navigation](0047-symbol-and-jump-pickers.md),
  [0049 multibuffer handoff](0049-product-and-architecture-handoff.md).
- [Zed Outline Panel](https://zed.dev/docs/outline-panel), including its
  [published outline screenshot](https://zed.dev/img/outline-panel/singleton.png).
- [VS Code Theme Color Reference](https://code.visualstudio.com/api/references/theme-color):
  `descriptionForeground`, list/quick-input selection and highlight roles,
  and `symbolIcon.*Foreground`.
- [Helix Pickers](https://docs.helix-editor.com/pickers.html).
- Rootle local references: `../rust/rootle/crates/rootle/src/components/global_search/{render,model}.rs`,
  `global_search/render/results.rs`, `preview/`, and `doc/house-style.md`.
