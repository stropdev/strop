# Plan ledger

One line per plan state, maintained in the same change that alters a plan's
standing (plans/0086 §5, 0085 M1). Plan bodies are history: a status line
inside a plan that disagrees with this ledger is stale, and this ledger wins.

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
| [0095](0095-the-keys.md) | Neovim's keys: the grammar's missing operators, motions and text objects, Vim's range rules, one operator engine for Normal and Visual, a Visual key table, the `:` line with ranges and abbreviations, a generated differential corpus (≥ 600 cases) and a Missing list in `strop compat`. Adopts 0091 §2 and 0092 §3; amends 0001 §5.10, 0016, 0003 §5.7 |
| [0097](0097-collect.md) | the collect flow: `Ctrl-O` waits for the search and lands on the selected hit; `]q` / `[q` step hits in the collection, `Enter` opens the file; collections named for their search; amends 0094 §3.1 |
| [0098](0098-modeline-design.md) | the modeline: one spacing scale, a 10-cell chip, a pending-keys slot, fixed-width position fields, one keep-order table for truncation, card margins and the `:` card's single text column. Amends 0088 §2 and 0032 |
| [0099](0099-caching.md) | caching: one proved core (`strop_kernel::cache`), validity by exact (subject, producer, version), builds that never block; document symbols kept per revision and fetched ahead of the key, with a syntax outline where no server serves the file. Git history designed, deferred. Amends 0047 §1 (a symbols list is revision-exact, never a snapshot) |
| [0100](0100-surfaces.md) | one look for the collection and the git buffers: a block is a ruled heading (`render/block/heading.rs`, one part table), its rows and one empty row; a jump inside a block is a band; the buffer holds words and the renderer draws lines; the gutter is data (`Gutter::{Lines, Source, Pair, List}`), lists have no line numbers; a row's owner is computed as data. Amends 0097 §3.4, 0096 §2.1 and 0010 §4. Log step B (aligned columns, refs, author and age drawn at the right), the commit view with its message, status letters, renames and `binary`, and per-entry `+N -M` in status landed on 0101's typed data (§11.1) |

## Proposed — not implemented

| Plan | Note |
|---|---|
| [0060](0060-debugger-workflow-and-architecture.md) | debugger workflow; supersedes the 0019 sketch |
| [0101](0101-history-and-limits.md) | in force (0.64): a private commit-graph and streamed rows for the log and file history (no row cache, the user's repository untouched); the LSP reply bound restated as a decode-memory budget, an over-budget reply failing one request instead of the server, `UNASKED_LINES` deleted; collections close the files they loaded that were never shown or edited. No kernel change. Amends 0099 §4.2 and §8, 0097 §6, 0094 §5 |

## Retired or superseded

| Plan | By | Note |
|---|---|---|
| [0019](0019-debugger.md) | 0060 | early debugger sketch |
| [0045](0045-verus-pilot.md) | 0086 | Verus retired; edit geometry is now proved in Lean |
| [0055](0055-embedded-terminal-tui-and-gui.md) | 0086 | TUI half implemented; GUI half retired with `strop-gui` |
| [0057](0057-core-verification-and-assurance.md) | 0086 | boundary inventory, evidence archive, TLAPS and Verus lanes deleted |
| [0058](0058-unified-native-worker.md) | 0086 (assurance parts) | the worker itself shipped in 0.36.0 and stays; its Verus/TLAPS/core-assurance gates are retired |
| [0061](0061-gui-windows-and-wsl.md) | 0086 | native Windows GUI deleted (owner decision, 2026-11-09) |
| [0062](0062-distribution-and-wsl-onboarding.md) | 0079, 0086 | Windows installer/distribution retired; Linux/WSL via install.sh |

## Record

| Plan | Note |
|---|---|
| [0085](0085-independent-critique-0.52.md) | independent critique of 0.52.0; adoption decided in 0086 §5 |
| [0087](0087-independent-critique-0.53.md) | independent critique of 0.53.0; adoption decided in 0088 §4 |
| [0089](0089-functionality-critique-0.54.md) | functionality critique of 0.54 (scope, terminal, remote, bugs); adoption decided in 0090 §3 |
| [0091](0091-editor-critique-0.56.md) | independent critique of 0.56 (fidelity, core workflows, claims, roadmap); 0092 adopts B7, B10 and the "stop claiming" items; 0093 (find) and 0094 (change) take the rest |

## Implemented — history

Everything else is implemented and kept as design history: 0002, 0004, 0007,
0009–0018, 0020–0044, 0046–0056, 0059, 0063–0078, 0080–0083.

Stale status lines inside those bodies, corrected here:

- **0037** (devcontainers) says "planned for later": implemented in
  `strop-containers` and `editor/devcontainers`.
- **0046** (engine extraction) says "stage A landing": `strop-engine` exists.
- **0083** is used by two plans (Splicery and Weft syntax highlighting), both
  landed in 0.52.0; the numbers stay because the changelog cites them.
- There is no 0052.
