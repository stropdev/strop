---
rfc: "2026-09-05-text-and-terminal-correctness"
title: "Text coordinates, block selections and paste"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-05"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/113"
milestone: "v0.8.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0017"
---

# RFC: Text coordinates, block selections and paste

v0.8.0 separated stored byte offsets from grapheme/display-cell layout, used cell columns for the caret and visual blocks, and inserted bracketed paste as literal text and one undo unit ([c145dbf](https://github.com/stropdev/strop/commit/c145dbf31143bea33f8a2cd0dad878dfc41a3d02)). This was a layout and input milestone, not a claim of universal Unicode motion parity.

[Tracking issue #113](https://github.com/stropdev/strop/issues/113) · [Published v0.8.0](https://github.com/stropdev/strop/releases/tag/v0.8.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0017-text-and-terminal-correctness.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

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
