# 0007 — Project-wide Search & Replace

> The feature Helix never landed (issue #1018, open since 2022) and the
> last thing VSCode users keep open a VSCode for. Two paths, one contract:
> the VSCode-style surface first-class, the vim-native quickfix path
> composed from the same machinery.

Status: planned. Priority: after the M4 wave, before M5 (LSP) — rename
and replace compose later: rename is semantic (LSP), replace is textual
(this plan); `Space r` stays reserved for rename, `Space R` is replace.

---

## 1. UX research (what the giants do)

| Editor | Model | Lesson |
|---|---|---|
| VSCode | sidebar: search field + replace field + match tree + replace-all | two fields, live preview, apply is one action |
| Zed | project search (`g /`), multibuffer excerpts, replace field on top | matches stay *editable*; replace previews in place |
| Neovim | `:grep` → quickfix → `:cdo s///g` (or cfdo) | composable but ceremony-heavy; no preview |
| Helix | — (issue #1018, unlanded) | the gap is the opening |

Strop's answer: the one picker component gains a **replace mode**. Not a
new surface — the same card, two fields, the same streaming rg pipeline.

## 2. The surface

`Space R` opens the picker in replace mode:

- **Two fields**: `search` (rg pattern, streams as today) and `replace`
  (literal by default; `\n` groups when the regex transpiler lands —
  0001 §2.5). `Tab` moves between fields (results nav moves to `↑↓` /
  `ctrl-n/p` while in a field — `Tab`'s cycle role yields to field
  switching in replace mode).
- **Every result row previews the replacement live**: matched span
  strikethrough-dimmed, replacement in accent, inline in the row.
- **Per-row toggle** (`x` on a row, or visual multi-select) excludes a
  match from the apply set. The count chip reads `3/47 excluded`.
- **`enter` applies**: one undo revision per touched buffer (our history
  tree already groups per transaction), message reads `replaced in N
  buffers (u per buffer to undo)`. Unsaved buffer edits never clobber:
  apply edits open buffers in memory; files not open are edited via mmap
  write with a mtime guard, then opened.

## 3. The vim-native path (same machinery)

Grep results can be sent to a quickfix-ish readonly buffer (`:copen`-style
surface — real buffer, `enter` jumps). `:cdo s/pat/sub/g` iterates it.
This is the fallback contract (0001 §2.1 fallback philosophy): the picker
surface is the luxury, the ex pipeline is the grammar-native one. Both
share the match list.

## 4. Correctness contract

- **One undo revision per buffer** for an apply — `u` in any touched
  buffer reverts exactly that buffer's replacements.
- **Never silent-partial**: if any file fails (permission, moved), the
  apply reports per-file and leaves the rest applied; no transaction
  spans files.
- The preview strikethrough is rendered from the same replacement
  computation that applies — preview cannot lie, here too.

## 5. Non-goals for v1

- Semantic rename (LSP, M5, `Space r`).
- Regex capture-group references in replace (needs 0001 §2.5's
  transpiler; v1 is literal + `\n`).
- Undo across files as one step (per-buffer is the vim truth; a global
  "undo the whole apply" needs cross-buffer revisions — the session/history
  work would need to grow a shared revision).

## 6. Fit with the roadmap

M4 wave 2 (with tree-sitter text objects + `Space j` + which-key tables).
Not before: it leans on the picker (M1), per-buffer history (this week),
and ideally the regex transpiler for capture groups (0001 §2.5).
