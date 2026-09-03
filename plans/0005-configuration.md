# 0005 — Configuration

> In a no-plugin editor the config file *is* the extension surface. This doc decides the
> format, the layering, the failure behavior, the `:set` question, and — most
> importantly — the written list of what will never be configurable.

Status: accepted-in-principle. Blocking before M0 code (with the 0001 §2.2 marks fix and
the §2.5 regex spec, both landed).

---

## 1. Format & layering

TOML. Three layers, later wins:

```
embedded defaults  <  $XDG_CONFIG_HOME/strop/config.toml  <  <project>/.strop.toml
```

Embedded defaults mean the binary is complete with zero files — the zero-config promise
from 0001 §1 is structural, not aspirational. Project-local config is per-directory and
never executes anything (data only — the no-plugin doctrine applies to config too).

LSP server config is the exception with its own file and layering —
helix-style `languages.toml`, project `.strop/` dir over XDG over the
embedded registry (0012); the chain above is editor knobs only.

## 2. Failure behavior (the part everyone gets wrong)

- **Invalid config never bricks the editor.** Parse errors, unknown keys, bad types:
  the offending layer is rejected, the last-good config stays live, and the error
  surfaces as a floating card (what, where, what we did about it) plus a statusline
  warning dot until fixed. Silent ignores and startup crashes are both bugs.
- **Bad keymaps degrade, not brick.** An unbindable/conflicting mapping is dropped with
  an error card; the default binding stays. You can always edit your way out of a broken
  keymap — editing is the one thing the editor must never lose.
- **Hot-reload.** The config file is watched; changes apply live (theme swaps should be
  visible mid-edit — the settings popup's live preview, plan 0003 §5.6, is the same
  mechanism). A broken save mid-edit hits the failure path above.

## 3. The `:set` question

Neovim hands will type `:set number`, `:set ignorecase` — refusing them is a fidelity
papercut. Decided: **a `:set` subset exists, session-scoped only.** It accepts the
common vim option spellings mapped onto config keys (`:set nu`, `:set ic`, `:set
tabstop=4`, …), affects the running session, and never writes the file. Persistence is
the settings popup or editing the TOML — one write path, no two-sources-of-truth drift.
`:help options` carries the mapping table.

## 4. The two lists

**Configurable** (the extension surface): theme/palette + accent color, keymaps
(rebind/unbind — not new operators), tab/indent width, line numbers on/off/relative,
signcolumn, scroll context lines, OSC52 clipboard on/off, file-ignore globs, grep
command line, git remote priority order (plan 0001 pillar 3), session persistence
on/off, which-key delay, cursor shape per mode, indent guides on/off.

**Not configurable, ever** (written down so "make it an option" has an answer):

- The grammar: motion/operator semantics, counts, registers, dot-repeat behavior.
  Fidelity is not a setting.
- The operator-pending preview contract (one resolver, spec footer, overlay precedence).
- The mode inventory and the house style (plan 0003 §1, §5).
- The leader: `Space`. One convention; rebindable keys yes, rebindable philosophy no.
- Async behavior: nothing user-facing ever blocks input. No "sync mode" escape hatch.

## 5. The settings popup is a view, not a store

The settings popup (plan 0003 §5.6) edits the same config model the file loads —
sections in the sidebar are the TOML tables, rows are the keys. `esc` saves by writing
the TOML (comments and unknown keys preserved — round-trip editing is the contract;
a popup that eats your comments is a bug). Hand-edits while the popup is open hot-reload
underneath it.

## 6. How knobs are represented (decided 2026-09-02 — for the settings popup + hot-reload, the rootle lesson applied)

One typed struct (`Config`, serde-Deserialize with `#[serde(default)]`) is the single
source of truth for values; a sibling `KNOBS` table (key, TOML path, kind, one-line
description) is the settings popup's future data source — the popup renders FROM it,
never duplicating descriptions in popup code (the rootle `sections.rs` shape).
Hot-reload: a file-watcher thread posts config-change events onto the event loop like
every other data source — the async invariant (0001 §5.6) means the watcher is a job
posting onto the event loop, never a blocking read in the input path. Writes go through
`toml_edit` for comment-preserving round-trips.

## 7. `strop update` (decided 2026-09-02)

Self-update for tarball installs: `strop update` / `strop update --check`. Channel
detection by exe path (cargo/brew/mise/tarball/other), mandatory sha256 sidecar
verification, staged write + atomic rename over self, curl for the network (no HTTP
stack in the binary).


## 8. Deferred

- Project-config trust prompt (`.strop.toml` in a cloned repo): data-only config is
  inert, so no trust gate needed v1; revisit if any future key gains path/exec power.
- Config schema export for editor tooling (JSON schema from the serde types).
- **Indent-guide fidelity (researched 2026-09-02, helix-term `ui/document.rs`):** Helix
  draws guides as an *overlay pass after* the line renders, positioned from the line's
  computed indent level — not from raw space positions. Our v1 (dim `│` at tab-size
  multiples within leading whitespace) matches on well-formed code but diverges on
  mixed/odd indents and blank lines inside blocks. When guides get their second pass:
  - compute per-line indent *level* (indent width from config/tab detection), draw the
    guide overlay from level 1..line_level — this is what makes Helix feel "smart" about
    not drawing spurious guides;
  - blank lines inside an indented block inherit the block's level (continuation);
  - knobs: `indent_guides.render` (bool, landed), `indent_guides.character`,
    `indent_guides.skip_levels` (skip shallow levels — Helix defaults 0, users who hate
    clutter set 1).
