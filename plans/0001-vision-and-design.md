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

Pitch: **verb-first grammar, selection-first visibility.** flash.nvim labels destinations from operator-pending but never previews the range; Kakoune/Helix get preview by construction by making selection the grammar. Strop keeps vim's grammar and previews anyway. And the combo nobody can chase: that grammar + a zero-config single static binary (plan 0002) + the preview. LazyVim approximates the pillar stack; it cannot approximate that sentence.

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

The preview overlay carries a **spec footer**, always on once the resolver has a target: `inner [, inclusive, 14 bytes` — motion type, inclusivity, byte count. Teaching and debugging with zero keys. (`?` stays search-backward: `d?pat⏎` is a real motion, that key was never free.)

### 2.2 Visual mode is the escape hatch

Everything Helix's match mode does, strop does from visual mode: enter with `v` + object (`vi[`), then compose — operators, counts, **split selection by regex (`s`, multi-cursor)**, select-next-match, extend/shrink. One selection system, no parallel commands.

(`m` was considered for this role and rejected: `m` is vim's mark-set — `ma`–`mz` — and shadowing it would be a fidelity bug in the most sensitive spot in the grammar. The keys stay vim's.)

### 2.3 Surround, first-class

`ys`/`cs`/`ds` operators (sandwich lineage), core features: `ysiw"`, `cs"'`, `ds(`. Plus `%` jump-to-pair, and `%` in visual mode extends to the matching end.

### 2.4 Tree-sitter text objects

`if`/`af` (function), `ic`/`ac` (class), parameter objects. The parse tree is already resident; these are queries.

### 2.5 Search & substitute regex — vim-flavored, one engine

One engine: the Rust `regex` crate, linear-time — which is what makes per-keystroke incsearch and `:s` previews safe (backrefs would reintroduce exponential blowup into a per-frame path). A thin transpiler covers muscle-memory sugar only: `\| \( \) \< \> \{n,m} \{-} \v \c/\C`; `\zs`/`\ze` via capture-group spans (no lookaround). Backrefs, lookaround, `\%V`, `\M`/`\V`, `~` are rejected with typed errors and a `:help regex-divergences` table. Smartcase is `(?i)` at the editor layer. The rope is searched in line chunks, never materialized to a `String`. The fidelity clause covers the modal grammar; regex is vim-flavored with documented divergences. Since `rg` speaks Rust regex natively, picker grep and internal search share one dialect — the transpiler exists regardless; this only chooses how much it covers.

## 3. The four pillars

Everything is a **real buffer**: motions, marks, search, and registers work everywhere. No special-purpose widgets that forget how to be a text editor.

### Pillar 1 — Search (rootle-grade, the centerpiece)

One picker component (input / results / preview pane), four data sources: file finder, buffer switcher, live grep, symbols. Consistency is the luxury nobody ships.

- Ripgrep subprocess, streaming results, match-per-line with context.
- Preview pane: syntax-highlighted, scrollable without leaving the results list.
- Results are navigable and operable: `dd` dismisses, `u` resurrects the row (it is a real buffer — that is the payoff), filter-as-you-type with fzf-style scoring.
- Grep → quickfix pipeline: send results to quickfix, `cdo`-style replace across matches. The Neovim-native workflow, done beautifully.

### Pillar 2 — File tree (dired/oil.nvim lineage)

- A text buffer: motions work; rename = edit the line, delete = `dd`, copy = yank/paste with confirmation.
- Cross-filters with grep: "grep only under this directory."

### Pillar 3 — Git (GitLens-class, modal-native)

Three surfaces:
Surface-openers live under the `Space g` leader namespace (codified in plan 0003); context verbs (`gu`, `gy`, `gO`) are git-buffer-local, and `]c`/`[c` stay bracket motions. Vim's `g` namespace stays vim-traditional (`gd`, `gg`, …) — git gets exactly one home.
1. **Working surface:** gutter signs (add/change/delete), `]c`/`[c` hunk nav, hunk stage/unstage/undo/preview operators.
2. **Commit browser (`Space g l`):** graph-rendered log as a buffer — ASCII graph, author, age, message; `/` search, fzf filter. `enter` → changed-files view: file list with +/- stats beside per-file unified delta, syntax-highlighted on both sides (tree-sitter on the post-image, delta-style). Read-only but motion-complete. `gu` resurrects a hunk into an editable comparison.
3. **Line level:** blame popup on demand (`Space g b`) — commit card, `enter` dives into the commit browser at that commit; toggleable blame column for archaeology. Permalinks (`gy` yank, `gO` open): remote priority `upstream` > `origin` > rest (configurable), SSH→HTTPS normalization, host detection (GitHub/GitLab/Bitbucket/Gitea), **branch always resolved to commit SHA** for immutable links, dirty-buffer lines anchored by content-match with a warning if the line isn't in HEAD.

Implementation: `git2`/libgit2 for gutter + hunks (no process spawn per keystroke); shell out to `git` for log/graph/blame (matches user config); `similar` crate for diff rendering.

### Pillar 4 — The editor core

- Rope buffer (`ropey` or `crop`). Never strings. Snapshots for async readers.
- Tree-sitter: incremental parsing fed edit diffs, injections from day one, highlighting on visible ranges only.
- Undo **tree**, visualized as a browsable branch graph popup (`Space u`). `U` stays vim's undo-line; Neovim users expect branches.
- Multiple cursors, opt-in power not paradigm: select-next-match, split selection by regex (visual `s`).
- Daily no-plugin verbs: `gc` comment toggle, `gq` format operator (internal hard-wrap until LSP format arrives at M5).
- Clipboard: `+`/`*` registers over OSC52 — the ssh-into-a-server test has no clipboard daemon.
- Sessions (per-project, on by default): buffers, cursor positions, jump list, undo history serialize per project directory, restored on next open; undo history depth-capped (full trees per project bloat fast). Named sessions (`strop --session NAME`) are a thin layer on top — deferred until the per-project loop proves itself.
- **Splits are core** (`:vs`/`:sp`, `C-w` navigation — vim grammar, M4). **Tabs are deferred**: vim tabs are layout containers, not buffer groups; Helix skips them; terminal multiplexers (herdr/tmux) cover the use honestly. Revisit only if daily driving begs for them.
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
| LSP (later) | `async-lsp` (decided; no hand-rolled JSON-RPC transport) |

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
6. **Async invariant, as law:** input→render never crosses an `await`. Tokio lives only behind data-source jobs (rg, git, LSP) posting results onto the event loop. This is what "input-to-echo under one frame" means mechanically.
7. **crossterm lives only at the binary edge.** Everything above renders into a ratatui `Frame`; ratatui's `Buffer` *is* the cell grid, so golden cell-grid snapshot tests come free via `TestBackend` — no custom render abstraction. The differential harness (#10) drives editor state, not rendering.
8. **Overlay precedence, decided pre-M2** (diff backgrounds arrive at M2 and start fighting): tree-sitter highlight < LSP semantic tokens < search/incsearch < operator preview < cursor.
9. **Char-width policy:** grapheme-cluster cursor steps, `unicode-width` for display width, CJK ambiguous-width = narrow (vim's default). Same leak class as decision 1 — deferring it means refactoring every motion.
10. **Fidelity is falsifiable:** a differential harness against headless `nvim --clean` — `:normal!` keystroke corpora, diffing cursor, mode, registers, and resolved ranges — gates CI. Decision 2 (the pure `strop-grammar` crate) makes it cheap; without it "deviations are bugs" is marketing.
11. **Tree-sitter runtime and grammar crates are pinned together in the lockfile.** Queries are data: parsers statically linked (plan 0002), `.scm` queries ship as embedded defaults with runtime overrides — highlight fixes without rebuilds, consistent with the no-plugin doctrine.

## 6. Roadmap

**M0 — proof of feel (the weekend prototype):**
rope buffer, normal/insert/visual modes, ~30 verbs, counts, registers, ex subset (`:w :q :e`), file open/save, tree-sitter highlighting for one language. **Plus the operator-pending preview for basic motions** — if `ci[` with live preview feels magical, the project is real; if it feels gimmicky, we learned it for one weekend's cost. Macros are explicitly out of the M0 bar: registers × counts × dot-repeat × macros interplay is the time sink, and the coverage matrix already declares macros preview-blind.

**M1 — picker:** file finder + live grep with the rootle-grade preview pane. Pane layout dictates render-tree structure, so this comes before git.

**M2 — git working surface:** gutter, hunks, stage/undo. The gutter's sign-column merge logic lands here; diagnostics plug into it later.

**M3 — git memory:** commit browser, changed-files/delta view, blame, permalinks.

**M4 — daily-driver gap-fill:** undo tree UI, sessions, surround, tree-sitter text objects, which-key overlay, tree buffer, **strop tutor** (built-in onboarding where the preview overlay is the feedback loop — nobody ships TUI onboarding, and it tapes into demo.tape for free).

**M5 — LSP:** goto-def, hover, diagnostics, completion. Diagnostics gutter reuses the git gutter column-merge logic. If daily-driving demands diagnostics sooner, the pulled-forward unit is a diagnostics-only LSP slice (initialize / didOpen / didChange / publishDiagnostics — the cheapest LSP feature), not the whole milestone.

**Explicit non-goals for the foreseeable future:** plugin runtime, GUI frontend, collaborative editing, AI in core — anything speaking LSP is welcome at M5+ with zero core work. Each is a valid project; none is this project.
