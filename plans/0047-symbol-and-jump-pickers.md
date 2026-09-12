# 0047 — Symbol picker, jumplist picker, and mark discoverability

Status: implemented in 0.23.0. Field feedback: document symbols (VSCode `Ctrl+Shift+O`)
are missing entirely; the jumplist exists (`Ctrl-O`/`Ctrl-I`) but is
invisible; and `m`'s which-key card shows `<a>` — cryptic next to the
leader card's plain-language rows.

Three small features, one theme: navigation state the user can *see*.
All reuse existing machinery (streaming picker, LSP request lifecycle,
jumplist storage, which-key cards); no architecture.

## 1. Document symbols — `Space s`, `:symbols`

The VSCode `Ctrl+Shift+O` shape: a picker of the current file's symbols,
fuzzy-filtered, Enter jumps.

- **LSP layer** (`strop-lsp`): new `RequestKind::DocumentSymbols`.
  `caps.supports` maps it to `documentSymbolProvider`. The wire builds
  `textDocument/documentSymbol` with the document's URI — remote
  workspaces translate through the existing `Workspace` seam (0036), so
  symbols work over `ssh://` unchanged. The reply decodes **both**
  response shapes: hierarchical `DocumentSymbol[]` (flattened with the
  container path: `impl Foo :: fn bar`) and legacy flat
  `SymbolInformation[]`.
- **Editor**: `Space s` with no capable server → the attach-refusal
  message pattern (named reason, never silence). Otherwise the request
  fires and the picker opens streaming (`Kind::Symbols`, title
  ` symbols `); rows land as they arrive.
- **Row text**: `name   container::path · kind · :42` — flat, so fuzzy
  matching works against the container path (indentation trees defeat
  fuzzy).
- **Accept**: zero new code. Local buffers emit `Payload::Grep` (path,
  line, col); remote emit `Payload::Remote`. Both accept paths exist,
  including the remote-hit routing. The jump records a jumplist entry
  first, so `Ctrl-O` returns.
- **Staleness**: the list is a snapshot of the request revision; a
  buffer edited between request and accept jumps to the recorded line
  (same contract as `gd` replies). No live re-query in v1.
- **Keymap**: `space s` becomes a live row (`document-symbols`);
  `space S` gains a muted soon-row reserving workspace symbols.

## 2. Jumplist picker — `Space j`, `:jumps`

Vim `:jumps` as a real picker. `space j` is already reserved in the
table (muted "jumplist picker (soon)") — this flips it live.

- **Items**: `jumplist_past` (newest first), a highlighted `>` row for
  the current position, then `jumplist_future`. Entries naming dead
  documents are filtered at build time (the same rule `jump_back`
  applies when walking).
- **Row text**: `filename:42  fn main() {` — path, computed line, line
  text preview.
- **Payload**: new `Payload::Jump { document: DocumentId, offset: usize }`.
  Accept pushes the current position (`push_jump`), switches document
  when needed, and lands on the entry — the same steps as `jump_to`,
  so `Ctrl-O` after a picker jump returns to where the picker was
  opened.
- Under 0051, jumplist entries and temporary-surface return points share one full
  `JumpRecord`: document incarnation, caret/anchor, extra selections, byte-anchored
  viewport top and horizontal display origin. Source journals remap those records.
  New targets use deliberate placement; Ctrl-O/Ctrl-I and closing temporary
  outputs restore the recorded view, clamped to the live document and geometry.

## 3. Mark cards — `m`, `'`, `` ` ``

The `m` card today: one row, `<a>  set mark at cursor`. Two fixes, both
in the which-key renderer (`render/which_key.rs`); dispatch untouched.

- **Label**: `<a>` renders as `a-z` in the card (display-layer mapping;
  the table token stays `<a>`).
- **Live marks list**: below the static hint, one muted row per existing
  mark, sorted by name: `a   :42  fn main() {`. For the jump cards
  (`'`, `` ` ``) the list *is* the menu — that's where discoverability
  pays. No marks yet → a single muted `no marks set` row.

Register absorb cards (`q`, `@`) can follow the same pattern later; out
of scope.

## Testing

- **Symbols**: wire-level decode test for both reply shapes; engine
  test with the fake-server fixture (0.22.1's spawn pattern): `Space s`
  → rows land → Enter jumps and records the jumplist entry; no-server
  case names the reason.
- **Jumps**: engine test — build a jumplist across two documents, open
  the picker, accept a middle entry, `Ctrl-O` returns; dead-document
  entries are absent.
- **Mark cards**: golden cell-grid snapshot (`TestBackend`) of the `m`
  card with two marks set, and the `'` card; `a-z` label pinned.
- **Keymap contract**: the dispatchable-sequence enumeration gains
  `space s` / `space j` rows; which-key children tests updated
  (0003 §5.7 coverage rule applies to both new pickers' bindings).

## Sequencing and non-goals

One release (0.23.0). Independent slices, landed in order: mark cards
(pure render), jumplist picker (no LSP), symbols (LSP + picker).

Non-goals: workspace symbols (`Space S`, soon-row only), syntax-tree
symbol fallback for non-LSP files, register absorb cards, mapping the
jumplist through the edit gateway.
