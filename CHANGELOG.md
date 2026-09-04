# Changelog

## 0.3.0 — 2026-09-04

Multicursor lands, help becomes a buffer, the demo's LSP section is
real again.

### Added

- **Multicursor** (plan 0013, nvim-0.13 interaction over a cascade
  executor): `Q` toggles a cursor at point, `Space c` stacks one onto
  the next line (helix's `C`). Motions, `n`/`N`, operators, yank, paste,
  and insert mode all cascade — deletes apply bottom-up, mirrored edits
  shift-remap, stacked cursors edit once. Normal-mode Esc collapses to
  the primary cursor; `u` reverts a whole cascade as one unit. Secondary
  cursors render as solid blocks. v1 deferrals (visual-mode multi-range,
  mouse placement, select-next-match) are documented in 0013.
- **`:help`** — the keybinding table as a real readonly buffer: `/`
  searches it, motions walk it, `q` closes. `Space ?` opens the same.
  The floating keybinds popup is gone (a buffer you can search beats a
  card you can only scroll).
- Headless `state` now reports `message` and `extra_cursors`.

### Fixed

- **Demo LSP section**: the vhs image now installs a rustup toolchain
  with the `rust-analyzer` + `rust-src` components (apt's rust lacks
  rust-src; the standalone binary half-worked and then errored).
- **Graceful LSP shutdown**: quit sends the `shutdown`/`exit` sequence —
  every session used to end with the server dying "client exited
  without proper shutdown" and a fake failure on the statusline.
- **vim fidelity: `cw`/`cW`** resolve like `ce`/`cE` — the trailing
  whitespace is no longer eaten, and at a word's last char only that
  word changes (pinned in the grammar contract tests).
- Commit-diff sidebar width fits the file list (clamped 12–24) instead
  of a fixed 28 columns.

## 0.2.2 — 2026-09-03

Hardening + daily-driver release. The crash-on-quit class is dead, the
editor opens directories, the system clipboard works, and the git
surfaces got the rootle-grade navigation treatment.

### Fixed (trust)

- **Quit crash**: `:q`/`:q!`/`:wq` on the last buffer with an LSP
  attached panicked in the post-feed drain (`lsp_sync_changed` indexed
  an emptied buffer list). The TUI breaks on `should_quit` before the
  drains now, and the sync is empty-list-safe. This was visible in the
  demo tape.
- Panic hook restores the terminal (raw mode + alt screen) on any crash.
- Hunk-restore underflows at file top (`Space g u` on a hunk at line 0).
- Undo cursor lands at the start of the undone change (vim semantics),
  not the tail of the replayed op list; redo distinguishes insert/delete
  placement. History replay clamps both bounds to char boundaries.
- `n`/`N` actually work now — the keymap advertised search repeat with
  no dispatch behind it (found by the new coverage test).

### Added

- **Directory open**: `strop dir/` cds and lands on the file picker
  (helix's `hx .`) instead of dying on EISDIR.
- **System clipboard**: `Space y` / `Space p` / `Space P` (helix-style)
  on top of vim's `"+` register — yank stages OSC52 (works over ssh),
  paste reads via wl-paste/xclip/xsel/pbpaste off the input path.
- **Global replace** (plan 0007): `Space R` — two fields, live
  replacement preview per row, `ctrl-x` excludes a match, Enter applies
  with one undo revision per buffer, span-verified and mtime-guarded,
  atomic file writes. Grep queries take rg filters (`-t rs`,
  `--glob '!target/*'`). Grep and replace render full-frame.
- **Undo-tree browser**: `Space u` — the revision tree as a real
  readonly buffer; Enter restores any revision (branches included), q
  closes.
- **Blame gutter** (0011): `Space g b` toggles a left-margin blame
  column (`sha · author · age`); Enter on a line dives into that line's
  commit. The blame card stays as the loading fallback.
- **Commit diff sidebar**: a commit's file delta shows the changed-files
  list in a left sidebar (tuicr-style); `]f` / `[f` step through the
  commit's files in place.
- **Surface stack** (0011): q/close on any git surface restores the
  origin buffer unconditionally, works in splits, and stale job results
  are generation-guarded.
- **Project LSP config** (0012): `.strop/languages.toml` over XDG
  `languages.toml` over the embedded registry — helix-flavored
  `[language-server.NAME.config]` passthrough (pyright `extraPaths`
  works), absolute commands skip the PATH probe, server capabilities now
  gate hover/goto.
- **Syntax**: fish, lua, sql grammars + vendored highlight queries; cpp
  uses the vendored query now; extensionless shell scripts resolve by
  basename (`.bashrc`, `PKGBUILD`) and shebang.
- **Commit graph lanes**: the log surface colors each graph lane
  distinctly, nodes bold in their lane color.

### Changed

- The keybinds popup (`Space ?`) and all which-key cards render from the
  one `keymap.rs` table; a coverage test pins every dispatchable
  sequence to a row (0003 §5.7). "(soon)" rows render muted as planned,
  never as live.
- Picker previews read files on worker threads — no more blocking IO in
  the render path (0001 §3).

### Plans

- New: 0011 surface stack, 0012 project config, 0013 multicursor
  (nvim-0.13 interaction over helix machinery — the next big rock).
- Amended: 0005/0009 (config filename reconciliation), 0007 (status +
  form-factor notes).
