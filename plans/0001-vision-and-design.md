# 0001 — Strop: Vision & Design

> **Strop** — a modal text editor in Rust. Neovim's hands, Helix's spine, rootle's eyes, GitLens' memory.
> Tagline: *see the cut before you make it.*
>
> Naming (checked 2026-08-31): a strop is the leather strap where a blade gets its final edge — the last pass of sharpening. `strop.dev` is secured (Porkbun, 2026-08-31; no routes yet — eye-candy site à la rootle.dev/gripsack.dev comes later, GitHub Pages); crates.io `strop` is taken, publish as `strop-editor`; GitHub org `stropdev` (mirrors the domain; repo: `stropdev/strop`).

Status: pre-prototype. No code yet. This document is the design contract for the prototype phase.

---

## 1. Why

Helix is the best-engineered editor core available (rope, tree-sitter, async LSP, zero plugins, instant startup) but its selection-first Kakoune grammar diverges from Neovim muscle memory, it has no file tree, and its grep UX is an afterthought. Neovim has the right grammar but requires a Lua config ecosystem to reach feature parity, with the attendant startup cost and fragility.

The opening: **Neovim grammar on a Helix-class core, with a world-class search/git/tree UX baked in.** No plugin runtime. If Helix's plugin API (Steel) stabilizes later, adopt it as a shim over our internal event bus — never as a core dependency.

## 2. The grammar

Verb-object, Neovim-faithful: operator-pending mode, counts, registers, marks, dot-repeat, macros. Fidelity is the entire reason the project exists; deviations are bugs.

Four deliberate additions, all additive to the grammar:

### 2.1 Operator-pending preview (the soul)

Every pending operation renders its target range live, before execution.

**Core invariant:** the preview is a dry-run of the same resolver that executes.

```
resolve(buffer, cursor, pending_keys) -> Option<Target>
execute = resolve() then apply
preview  = resolve() then render overlay
```

One resolver, two consumers. The preview cannot lie; a lying preview is a motion bug, which makes the preview a dogfooding harness for the motion engine itself.

Coverage matrix:

| Input class | Preview quality |
|---|---|
| motions + text objects + counts (`dw`, `ci[`, `y3j`, `d%`) | perfect, updates per keystroke |
| `f/F/t/T` char motions | perfect; after `df`, candidate chars get leap-style labels |
| search motion `d/pat⏎` | perfect; composes incsearch highlight into cursor→match range |
| offscreen targets (`d'a`, `dG`, `d/{far}`) | degraded: transient peek split showing target context |
| tree-sitter objects (`dif`, `dac`) | async; may land a frame late, never blocks input |
| dot-repeat (`.`) | flash recorded change region before re-applying (novel) |
| macros | none; execute blind |
| `:s` ex commands | inccommand-style live split |

**Fallback contract:** anything not previewable degrades silently to plain Neovim behavior. Coverage beats completeness; lag is never acceptable.

Motion semantics must be visible: exclusive vs inclusive vs linewise ranges highlight exactly the affected bytes. This doubles as the correctness spec and as a teaching tool for the vim grammar.

### 2.2 `m` — the match escape hatch

`m` + text object enters visual mode with that selection (`mi[` ≈ `vi[`). From visual mode everything composes: operators, multi-cursor split-by-regex, extend/shrink. This expresses Helix's match mode as one normal-mode command reusing existing visual machinery — no parallel selection system.

### 2.3 Surround, first-class

`ys`/`cs`/`ds` operators (sandwich lineage), core features: `ysiw"`, `cs"'`, `ds(`. Plus `%` jump-to-pair, and `%` in visual mode extends to the matching end.

### 2.4 Tree-sitter text objects

`if`/`af` (function), `ic`/`ac` (class), parameter objects. The parse tree is already resident; these are queries.

## 3. The four pillars

Everything is a **real buffer**: motions, marks, search, and registers work everywhere. No special-purpose widgets that forget how to be a text editor.

### Pillar 1 — Search (rootle-grade, the centerpiece)

One picker component (input / results / preview pane), four data sources: file finder, buffer switcher, live grep, symbols. Consistency is the luxury nobody ships.

- Ripgrep subprocess, streaming results, match-per-line with context.
- Preview pane: syntax-highlighted, scrollable without leaving the results list.
- `enter` opens the file at the match in a **real editable buffer** — not a preview-you-can't-touch.
- Results are navigable and operable: `dd` dismisses, filter-as-you-type with fzf-style scoring.
- Grep → quickfix pipeline: send results to quickfix, `cdo`-style replace across matches. The Neovim-native workflow, done beautifully.

### Pillar 2 — File tree (dired/oil.nvim lineage)

- A text buffer: motions work; rename = edit the line, delete = `dd`, copy = yank/paste with confirmation.
- Cross-filters with grep: "grep only under this directory."

### Pillar 3 — Git (GitLens-class, modal-native)

Three surfaces:

1. **Working surface:** gutter signs (add/change/delete), `]c`/`[c` hunk nav, hunk stage/unstage/undo/preview operators.
2. **Commit browser (`gl`):** graph-rendered log as a buffer — ASCII graph, author, age, message; `/` search, fzf filter. `enter` → changed-files view: file list with +/- stats beside per-file unified delta, syntax-highlighted on both sides (tree-sitter on the post-image, delta-style). Read-only but motion-complete. `gu` resurrects a hunk into an editable comparison.
3. **Line level:** blame popup on demand (`gb`) — commit card, `enter` dives into the commit browser at that commit; toggleable blame column for archaeology. Permalinks (`gy` yank, `gO` open): remote priority `upstream` > `origin` > rest (configurable), SSH→HTTPS normalization, host detection (GitHub/GitLab/Bitbucket/Gitea), **branch always resolved to commit SHA** for immutable links, dirty-buffer lines anchored by content-match with a warning if the line isn't in HEAD.

Implementation: `git2`/libgit2 for gutter + hunks (no process spawn per keystroke); shell out to `git` for log/graph/blame (matches user config); `similar` crate for diff rendering.

### Pillar 4 — The editor core

- Rope buffer (`ropey` or `crop`). Never strings. Snapshots for async readers.
- Tree-sitter: incremental parsing fed edit diffs, injections from day one, highlighting on visible ranges only.
- Undo **tree**, visualized as a browsable branch graph popup (`U`). Neovim users expect branches.
- Multiple cursors, opt-in power not paradigm: select-next-match, split selection by regex.
- Sessions: buffers, cursor positions, jump lists, undo history serialize per project (serde on editor state). Cheap early, brutal to retrofit.
- Jump list + change list with a subtle breadcrumb trail in the statusline — invisible state made felt.
- Command palette + which-key hybrid: `:` ex-line with real completion; `Space` opens a key-hint overlay that expands as you type (Helix's one undeniably better idea).

## 4. Design language

Restraint, not decoration. Snappiness is a perceived property as much as a measured one.

- **One accent color.** Desaturated gray base, warm white text, single saturated accent reserved for cursor mode / active match / current selection. Mode = accent color change, not colored bars.
- **Statusline:** one slim line, always. Mode chip (colored), file + dirty dot, git branch + dirty count, breadcrumb trail, position. Never a second line of chrome.
- **Floating panes:** rounded single-line borders, 1-cell inner padding, depth faked by dimming the border column.
- **Picker matches:** accent + bold on matched characters, never background blocks (muddy in truecolor terminals).
- **Nerd Font optional, never required.** Clean Unicode defaults (`│ ─ ╮ ▏ ●`), glyph detection upgrades. Must pass the ssh-into-a-server test.
- **Zero animation, zero latency theater.** No spinners before 100ms. 60fps render loop; input-to-echo under one frame.
- **Whitespace discipline:** muted gray line numbers, current line number in accent, one empty column between gutter and text.
- **Help that looks designed:** `Space` overlay and `:help` as centered floating cards with aligned key tables.

## 5. Architecture

### Stack

| Concern | Choice |
|---|---|
| TUI | `ratatui` + `crossterm` |
| Buffer | `ropey` or `crop` (evaluate; crop's cursor API is nicer, ropey is battle-tested) |
| Syntax | `tree-sitter` + injection queries |
| Async | `tokio` |
| Git | `git2` (hot paths) + shell `git` (log/blame) |
| Diff | `similar` |
| Serialization | `serde` (sessions) |
| LSP (later) | hand-rolled JSON-RPC or `async-lsp` |

### Crate layout (initial sketch)

```
strop-core      buffer, rope, edit ops, undo tree
strop-grammar   motions, text objects, operator-pending resolver (pure)
strop-syntax    tree-sitter: parse, highlight, injections
strop-render    render loop, overlay layers, layout
strop-picker    input/results/preview component + data sources
strop-git       gutter, hunks, log, blame, permalinks
strop           binary, modes, keymaps, ex commands, glue
```

### Decisions made now to avoid pain later

1. **Internal positions are UTF-8 byte offsets, everywhere.** LSP speaks UTF-16 code units; convert strictly at the LSP boundary, and assume servers lie about their encoding. This is the single ugliest refactor class if deferred.
2. **The operator-pending resolver is pure and lives in `strop-grammar`.** Preview and execute consume the same function. No UI code in the resolver.
3. **Render overlay layer** is a first-class render concept: selections, previews, search highlights, diff backgrounds all draw as ordered overlays, not ad-hoc cell mutations.
4. **Internal command/event bus** from the start. Not a plugin API — just clean seams so that adopting Helix's Steel API later is a shim, not surgery.
5. **Selection model: Neovim semantics** (inclusive char in visual mode), even though Helix's anchor-range model is cleaner internally. Fidelity beats elegance here; it leaks into every motion.

## 6. Roadmap

**M0 — proof of feel (the weekend prototype):**
rope buffer, normal/insert/visual modes, ~30 verbs, counts, registers, ex subset (`:w :q :e`), file open/save, tree-sitter highlighting for one language. **Plus the operator-pending preview for basic motions** — if `ci[` with live preview feels magical, the project is real; if it feels gimmicky, we learned it for one weekend's cost.

**M1 — picker:** file finder + live grep with the rootle-grade preview pane. Pane layout dictates render-tree structure, so this comes before git.

**M2 — git working surface:** gutter, hunks, stage/undo.

**M3 — git memory:** commit browser, changed-files/delta view, blame, permalinks.

**M4 — daily-driver gap-fill:** undo tree UI, sessions, surround, tree-sitter text objects, which-key overlay, tree buffer.

**M5 — LSP:** goto-def, hover, diagnostics, completion. Diagnostics gutter reuses the git gutter column-merge logic.

**Explicit non-goals for the foreseeable future:** plugin runtime, GUI frontend, collaborative editing, AI integration. Each is a valid project; none is this project.
