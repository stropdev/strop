# 0017 — Text and terminal correctness (0.8)

Status: accepted (2026-09-05 review). Strop is byte-native in paths
that need character/cell semantics. `ByteOffset` stays the canonical
storage coordinate; this plan adds the layout layer around it.

## Scope

- `LineLayout` per visible line: byte↔display-column maps, grapheme
  boundaries, rendered width. Every visible-line consumer uses it:
  renderer, cursor placement, selection overlays, diagnostics, git
  gutter, mouse hit-testing, desired-column, blockwise visual.
- Coordinate types stay distinct: `ByteOffset`, `ScalarIndex`,
  `GraphemeIndex`, `Utf16Column`, `DisplayColumn` — no blanket
  `From<usize>`.
- Rendering walks graphemes; `.chars().enumerate()`-derived byte
  positions go away.
- `replace_char_n` counts characters, not bytes.
- `*`/`#` word classification becomes Unicode-aware.
- Pane-relative cursor geometry verified (pane x/y origin applied
  uniformly).
- Visual block mode (`Ctrl-V`) on the new SelectionShape.
- Bracketed paste handling.
- Zero-width and tiny-terminal geometry guards.

## Non-goals

- Replacing Ropey. Storage stays; layout is the new layer.
- Incremental LSP sync (0018 territory, low priority).

## Tests

UTF-8 property corpus: emoji, CJK, combining marks, ZWJ sequences,
tabs, CRLF, missing final newline — cursor cell placement, selection
highlighting, `r`/`x` counts, `*` on non-ASCII identifiers.
Differential vs nvim for the ASCII corpus to ensure no regressions.
