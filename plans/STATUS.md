# Plan ledger

One line per plan state, maintained in the same change that alters a plan's
standing (plans/0086 §5, 0085 M1). Plan bodies are history: a status line
inside a plan that disagrees with this ledger is stale, and this ledger wins.
The live roadmap (deferred work, over-ceiling files) is
[0028](0028-roadmap-and-review.md).

## In force — contracts every change answers to

| Plan | Contract |
|---|---|
| [0001](0001-vision-and-design.md) | vision and design contract (doctrine) |
| [0003](0003-modes-leader-and-house-style.md) | modes, leader, keymap house style; `?` help coverage (§5.7) |
| [0005](0005-configuration.md) | configuration model |
| [0006](0006-e2e-harness.md) | e2e tiers and the Neovim differential harness |
| [0008](0008-input-layer-keymap.md) | the input walker and the one keymap table |
| [0079](0079-local-linux-release.md) | local Linux release procedure (its completion freeze is retired by 0086) |
| [0086](0086-verified-foundation.md) | verified foundation: rings, Lean-proved kernel, survival and release rules, gates |
| [0088](0088-the-look.md) | design language, token set and the six original palettes; echo row; golden snapshots |
| [0090](0090-one-editor.md) | containers cut, nvim terminal model, remote over plain SSH behind an ergonomics gate (0.55–0.56) |
| [0092](0092-registers-and-pinned-project.md) | one register file with Neovim's rules and one read path for every text input; project root pinned at launch; library files owned by the project's server; crashed servers restart within a budget. Amends 0074 (field registers), 0053 (Search toggle is `ctrl-t`), 0049 §4.6 |
| [0093](0093-find.md) | find (0.60): the first row is the file you meant (exact names first, remembered opens), project search as fast as `rg`, the three toggles every editor has, readable reference lists. Adopts 0091 §4.1, §4.2, §4.6, §5.4 |
| [0094](0094-change.md) | change: one surface for changing many places and one undo — editable collections, a quickfix list, `:s` preview from the change's own column. Adopts 0091 §4.3, §4.4, §5.1, §5.3, §5.4 (B2–B6) |
| [0095](0095-the-keys.md) | Neovim's keys: the grammar's missing operators, motions and text objects, Vim's range rules, one operator engine for Normal and Visual, a Visual key table, the `:` line with ranges and abbreviations, a generated differential corpus (≥ 600 cases) and a Missing list in `strop compat`. Adopts 0091 §2 and 0092 §3; amends 0001 §5.10, 0016, 0003 §5.7 |
| [0096](0096-git.md) | git (0.62): status, stage, commit, switch branch in real buffers — the loop closed without a new kind of widget. Adopts 0091 §4.7 |
| [0097](0097-collect.md) | the collect flow: `Ctrl-O` waits for the search and lands on the selected hit; `]q` / `[q` step hits in the collection, `Enter` opens the file; collections named for their search; amends 0094 §3.1 |
| [0098](0098-modeline-design.md) | the modeline: one spacing scale, a 10-cell chip, a pending-keys slot, fixed-width position fields, one keep-order table for truncation, card margins and the `:` card's single text column. Amends 0088 §2 and 0032 |
| [0099](0099-caching.md) | caching: one proved core (`strop_kernel::cache`), validity by exact (subject, producer, version), builds that never block; document symbols kept per revision and fetched ahead of the key, with a syntax outline where no server serves the file. Git history designed, deferred. Amends 0047 §1 (a symbols list is revision-exact, never a snapshot) |
| [0100](0100-surfaces.md) | one look for the collection and the git buffers: a block is a ruled heading (`render/block/heading.rs`, one part table), its rows and one empty row; a jump inside a block is a band; the buffer holds words and the renderer draws lines; the gutter is data (`Gutter::{Lines, Source, Pair, List}`), lists have no line numbers; a row's owner is computed as data. Amends 0097 §3.4, 0096 §2.1 and 0010 §4. Log step B (aligned columns, refs, author and age drawn at the right), the commit view with its message, status letters, renames and `binary`, and per-entry `+N -M` in status landed on 0101's typed data (§11.1) |
| [0101](0101-history-and-limits.md) | 0.64: a private commit-graph and streamed rows for the log and file history (no row cache, the user's repository untouched); the LSP reply bound restated as a decode-memory budget, an over-budget reply failing one request instead of the server, `UNASKED_LINES` deleted; collections close the files they loaded that were never shown or edited. No kernel change. Amends 0099 §4.2 and §8, 0097 §6, 0094 §5 |
| [0102](0102-folder-view.md) | 0.65: a folder is one editable buffer of names (line N is entry N); vim edits plus `:w` change files through a confirm buffer (`y` applies, `q` cancels) that renders the prepared batch itself; size and age are virtual text; `g.` dotfiles; `:fs`, `:browse`, marks, receipts and the copy/delete policies deleted; unconfirmed changes verified by themselves; lock files removed on release. Amends 0054, 0089 item 2, 0091 §5.1 |
| [0103](0103-seams-and-splits.md) | 0.66: a collection's file heading is a heavy `━` seam in `fg.muted`; pane titles are rules with no corners; side-by-side panes share one full-height `│` divider meeting the titles in `┬`; stacked panes lose the closing row (the lower title is the boundary). Amends 0088 rule 4, 0100 §3.2 |
| [0104](0104-window-tree.md) | 0.67: panes are vim's window tree — `:sp`/`:vs` divide the current pane and nest; close gives the cells and focus to the previous neighbour and collapses single children; numbers follow reading order; `ctrl-w h/j/k/l` go to the pane on that side and stop at the edge; one split's cells divided by `strop_kernel::layout::tile` (proved in `proofs/Kernel/Layout.lean`); junctions `┬ ├ ┤ ┼ ┴`; the differential harness compares the window tree with Neovim. Amends 0103 §4.1 |
| [0105](0105-the-number-gap.md) | 0.69: two blank cells between a line number and its text (`NUMBER_GAP`), in file buffers, collections and diffs; the picker preview drops its `│` and takes the buffer's gutter; sign column, alignment and inks unchanged. Amends 0100 §3.5 |
| [0106](0106-find-alignment.md) | 0.70: the file finder's folders stand at one column per list, `clamp(widest name + 2, W/3, W/2)` over the first 256 ranked rows, never per window, so nothing moves while scrolling; a name too wide for the column is shown alone and whole. Renderer only. Amends 0088 File names |
| [0107](0107-breadcrumb.md) | 0.70: the modeline's breadcrumb — the names of the symbols containing the cursor, outer to inner (`impl Modeline › fit`), after the file group in a per-file slot of at most 40 cells; read from the kept `Space s` list through a scope index (`editor/symbols/scope.rs`) carried through edits by the proved edit map; outer segments leave behind `…` first; outranks the branch (two turns each). Amends 0098 §3.2–3.3, 0099 §4.1 |
| [0108](0108-lean-and-correct.md) | 0.71: lean and correct — crash and data-hazard fixes each with a regression test, gates that block, proofs calibrated on properties, and the clutter sweep (no CLI flag synonyms, no legacy wire readers, no dead platform branches, no path-dep version pins; a version bump is the workspace `Cargo.toml` + `cargo check`). Amends AGENTS.md, 0086 |
| [0109](0109-one-filter-language.md) | 0.71: one filter language for every list — `kind:` only (`type:` is an unknown key), one kind table in `strop-core` for chips, values and filters. Amends 0063 |
| [0110](0110-command-line.md) | 0.71: the command line is vim's bottom row; `Esc`/`Ctrl-C` cancel; Normal-key editing is the command-line window (`Ctrl-F`, `q:`, `q/`); Vim's editing keys, each in `?`. Amends 0003 §1 and §5.8, 0098 §3.4, 0088 |
| [0111](0111-highlighting.md) | 0.71: highlighting that stays right — failed analyses remembered per key, deep injections uncoloured past the cap, compiled queries shared per language, combined injections collected document-wide |

## Proposed — not implemented

| Plan | Note |
|---|---|
| [0060](0060-debugger-workflow-and-architecture.md) | debugger workflow; supersedes the 0019 sketch |

## Retired or superseded

| Plan | By | Note |
|---|---|---|
| [0019](0019-debugger.md) | 0060 | early debugger sketch |
| [0037](0037-devcontainers-and-workspace-contexts.md) | 0090 | devcontainers were implemented (0081) and then cut with `strop-containers` |
| [0045](0045-verus-pilot.md) | 0086 | Verus retired; edit geometry is now proved in Lean |
| [0055](0055-embedded-terminal-tui-and-gui.md) | 0086 | TUI half implemented; GUI half retired with `strop-gui` |
| [0057](0057-core-verification-and-assurance.md) | 0086 | boundary inventory, evidence archive, TLAPS and Verus lanes deleted |
| [0058](0058-unified-native-worker.md) | 0086 (assurance parts) | the worker itself shipped in 0.36.0 and stays; its Verus/TLAPS/core-assurance gates are retired |
| [0061](0061-gui-windows-and-wsl.md) | 0086 | native Windows GUI deleted (owner decision, 2026-11-09) |
| [0062](0062-distribution-and-wsl-onboarding.md) | 0079, 0086 | Windows installer/distribution retired; Linux/WSL via install.sh |
| [0041](0041-handoff-adoption-and-roadmap.md) | later plans | handoff; what was adopted landed, its roadmap is history |
| [0049](0049-product-and-architecture-handoff.md) | later plans | handoff; landed in 0.24–0.25 |
| [0050](0050-picker-visual-polish-handoff.md) | 0088 | handoff; landed in 0.28, the look is now 0088's |
| [0065](0065-modal-terminal-experience-handoff.md) | 0090 | handoff; landed, the terminal model is now 0090's |
| [0078](0078-paper-theme-handoff.md) | 0088 | handoff; landed, palettes are now 0088's |
| [0080](0080-tui-hierarchy-and-density-handoff.md) | 0088 | handoff; landed, the hierarchy is now 0088's |
| [0081](0081-product-coherence-and-workspace-experience-handoff.md) | 0090 | handoff; landed in 0.48, scope cut by 0090 |
| [0085](0085-independent-critique-0.52.md) | 0086 §5 | critique of 0.52.0; adoption decided there |
| [0087](0087-independent-critique-0.53.md) | 0088 §4 | critique of 0.53.0; adoption decided there |
| [0089](0089-functionality-critique-0.54.md) | 0090 §3 | functionality critique of 0.54 (scope, terminal, remote, bugs) |
| [0091](0091-editor-critique-0.56.md) | 0092–0096 | critique of 0.56 (fidelity, core workflows, claims, roadmap); 0092 adopts B7, B10 and the "stop claiming" items; 0093 (find), 0094 (change), 0095 (keys) and 0096 (git) take the rest |

## Implemented — history

Everything else is implemented and kept as design history: 0002, 0004, 0007,
0009–0018, 0020–0036, 0038–0040, 0042–0044, 0046–0048, 0051, 0053, 0054,
0056, 0059, 0063, 0064, 0066–0077, 0082, 0083.

Stale status lines inside those bodies, corrected here:

- **0028** (roadmap and review) is the live roadmap; its 0.14–0.18 review
  body is history.
- **0046** (engine extraction) says "stage A landing": `strop-engine` exists.
- **0083** is used by two plans (Splicery and Weft syntax highlighting), both
  landed in 0.52.0; the numbers stay because the changelog cites them.
- There is no 0052.
