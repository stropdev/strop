# 0003 — Modes, Leader Map, Git Surfaces, House Style

> Codifies the interaction layer: the mode inventory stays vim-sized, git gets surfaces
> (not a mode), `Space` is the leader namespace, and every pane obeys the house style
> established by rootle.

Status: accepted-in-principle. Amends plan 0001 pillar 3 keybindings (done: `gl`/`gb` →
`Space g l` / `Space g b`).

---

## 1. Mode inventory (closed set)

```
normal · insert · visual{char,line,block} · operator-pending · ex/command · replace(M4+)
```

Not modes, never modes:

- **Operator-pending preview** — a render state of operator-pending, per plan 0001 §2.1.
- **Which-key overlay / transients** — key-hint layer over pending key state (§3).
- **Pickers** — a layout of real buffers; the input line is insert mode in a prompt
  buffer, the results list is a normal-mode buffer (§2).
- **Git** — surfaces with buffer-local keymaps (§4). No git mode exists; "modal-on-modal"
  would force redefining every operator (`dw`, `dd`) per mode and breaks plan 0001's
  fidelity promise.
- **Visual mode is the escape hatch** (plan 0001 §2.2): `v` + object, then real
  operators (`c`/`d`/`y`) and visual-mode commands (split-by-regex, extend/shrink).
  There is no `Space m` match-replace tree: it would duplicate `vi[` + `c`/`d` with
  worse preview, no dot-repeat, and a second selection system. And `m` itself stays
  vim's mark-set (`ma`–`mz`) — the original §2.2 design shadowed it, a fidelity bug in
  the most sensitive spot in the grammar.

## 2. The `Space` leader namespace

Helix-parity leader; the which-key overlay (plan 0001 §4) makes it self-discovering.
`Space` mappings are normal-mode prefixes, not a mode.

| Binding | Surface | Source availability |
|---|---|---|
| `Space f` | file finder (picker) | M1 |
| `Space b` | buffer switcher, MRU-ordered | M1 |
| `Space j` | jumplist picker — jump history as a navigable list | M4 (jump list core) |
| `Space u` | undo-tree browser (branch graph popup) | M4 |
| `Space /` | live grep (picker) | M1 |
| `Space s` | symbols in current buffer | tree-sitter outline first; LSP-backed at M5 |
| `Space S` | workspace symbols | M5 (LSP) |
| `Space g` | git namespace (§4) | M2/M3 |
| `Space d` | diagnostics picker | M5 |
| `Space k` | hover docs | M5 |
| `Space r` | rename | M5 |
| `Space a` | code actions | M5 |

Notes on the proposals that shaped this table: `Space f`/`Space s` adopted as-is.
"Jump to recent buffers" splits in two, both kept: `Space b` is MRU-ordered so recent
buffers are the top rows (Helix behavior), and `Space j` is the jump *history* picker —
the jump list is already a plan-0001 core feature, and a picker over it pairs with the
statusline breadcrumb trail.

Deliberately absent: `Space w` (splits stay on vim's `C-w` — one convention), a `Space m`
tree (§1), `Space ?` (`?` is global, §5).

### Are pickers popups? Yes — one picker component, floating

Every picker — files, buffers, jumplist, grep, symbols — is the plan-0001 pillar-1
component rendered as a **centered floating card** (input top, results left, preview
right), never a split layout. Rationale: splits reflow the text you're editing — the
editor jumps while you're deciding; a card floats over a stable background. Zed and
Helix made the same call; JetBrains' Recent Files popup is the same instinct.

Three deliberate upgrades over the Helix/Zed baseline:

1. **Live dimmed backdrop (JetBrains Ctrl+E lineage).** The buffer behind the card
   renders dimmed but readable, and the highlighted result's location is previewed *in
   the backdrop* as you move — you see where you'll land before you commit. The preview
   pane still exists for content; the backdrop answers *where*.
2. **`Tab` cycles without re-opening.** In the buffer switcher and jumplist, `Tab` /
   `Shift-Tab` walk the MRU/jump chain with the backdrop updating per step; `enter`
   commits. If the terminal speaks the kitty keyboard protocol (key-release events),
   Space-held cycling upgrades to JetBrains-style hold-and-tap; without it, `Tab` inside
   the open card is the same idea one keystroke slower.
3. **The jumplist picker shows the trail, not just rows.** Each row carries its
   breadcrumb (plan 0001's statusline trail) and a `▲3`/`▼2` marker — distance and
   direction relative to the current position — so the list reads as your path through
   the code, not an anonymous file list. `ctrl-o`/`ctrl-i` still work blind; the picker
   is for when you want to *see* the cut before you make it.

## 3. Which-key overlay + transients

Pressing a prefix (`Space`, `g`, `]`, `[`, …) opens the overlay
after the plan-0001 no-latency-theater threshold; it expands/narrows as you type.
Transients are the magit-style extension: a prefix opens a persistent card whose keys
are flags and verbs — `Space g c` opens the commit transient (message line is a real
insert buffer; flags like amend are toggles). Transients are additive to the grammar and
never shadow operators.

## 4. Git surfaces (no git mode)

Openers from any buffer, all under `Space g`:

| Binding | Surface |
|---|---|
| `Space g l` | commit browser (graph log) |
| `Space g h` | file history (log scoped to current file) |
| `Space g b` | blame popup at cursor line; `Space g B` toggles blame column |
| `Space g s` | status/changes surface |
| `Space g c` | commit transient |
| `Space g y` | copy remote URL (permalink; branch resolved to commit SHA — plan 0001 rules) |
| `Space g o` | open remote URL in browser |

Inside git buffers, buffer-local verbs (single keys, fall through to normal-mode
motions for everything unbound): `enter` dive, `q` climb out, `s`/`u` stage/unstage,
`dd` dismiss a row, `gu` resurrect hunk, `gy`/`gO` permalink yank/open. The statusline
chip reads buffer type (e.g. `GIT · log`) — context is visible without a mode.

## 5. House style (rootle lineage, contract for every surface)

Applies to every pane, popup, and picker strop ever ships:

1. **Every result pane is `/`-searchable.** It is a real buffer, so `/` works: picker
   results, git log, changed-files, blame view, help, keybinds, jumplist. The picker
   input's fzf-style filter is additive, never a replacement.
2. **Popups are centered cards** (rootle dimensions ~72×62, clamped to viewport):
   rounded-or-plain border per theme, bold title, **key hints in the bottom border**
   (` tab/h/l section · j/k row · esc save `), `● unsaved` dirty dot top-right when
   applicable.
3. **Selection marker is `▌`** in the gutter; sidebar items get `▸ name` + one-word
   blurb; the active row carries the selection background full-column.
4. **Chips:** mode chips render filled when active, dim colored outlines when not;
   keybindings render as keycap chips in a fixed column with descriptions after.
5. **Scrollbars live in the border column**, not in the content.
6. **Settings popup mirrors rootle exactly:** left sidebar of sections (`▸` + blurb),
   active section's rows as text / bool dots / radio lists, in-place edit cursor,
   theme changes preview live (popup renders with the previewed palette), `esc` saves.
7. **Keybinds popup on `?`** (global, works from any mode's buffer): sidebar of mode
   chips with binding counts, active mode's bindings as keycap rows, app version in the
   title. **The popup is generated from the same binding tables as dispatch** — rootle
   tests this property ("coverage is the contract"); strop adopts the test: every
   dispatchable binding renders in `?`, no hand-maintained help text.
8. Plan 0001 §4 invariants still govern everything above: one accent color, matches get
   accent+bold (never background blocks), no spinners before 100ms, Nerd Font optional.

## 6. Deferred

- Replace mode keymap edge cases (M4).
- `Space` map entries gated on LSP (`S d k r a`) ship at M5; the binding slots are
  reserved now so no M1–M4 binding has to move later.
- Which-key threshold tuning (start at plan 0001's 100ms spinner rule; measure).
- Kitty-protocol hold-and-tap cycling (§2): gate behind terminal capability detection.
