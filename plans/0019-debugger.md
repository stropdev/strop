# 0019 — Debugger (research + scoping; not scheduled)

Status: researched, deliberately deferred. The editor experience comes
first (0016/0017/0018); this plan freezes the research so the future
work starts from decisions, not discovery.

## Scope decision (2026-09-05)

Three toolchains, two adapters, zero ambition beyond them:

- **Rust + C/C++ → `lldb-dap`** (LLVM's native DAP server, ships with
  llvm; CodeLLDB as the fallback). One adapter family covers both
  toolchains.
- **Python → `debugpy`** (the reference adapter; `pip install debugpy`
  is the standard install).
- Everything else: not supported, and the registry says so honestly
  ("no debug adapter for X") rather than pretending.

Helix's DAP has stayed experimental for years because the edge cases
across *many* adapters never end. Two adapters, three languages, tested
hard, beats ten adapters in a drawer.

## Configuration: adopt launch.json, don't invent

- **Read `.vscode/launch.json`** as a fallback (zed does exactly this):
  `configurations[]` entries with `type: "lldb"|"debugpy"`, `request:
  "launch"|"attach"`, `program`, `args`, `cwd`, `env`, plus
  `preLaunchTask` **ignored** (tasks are vscode's subsystem; strop
  doesn't have one and shouldn't grow one for this).
- **Primary: `.strop/debug.json`** — zed's shape (an array of
  `{adapter, label, request, program, args?, cwd?, env?, build?}`),
  layered over the user config dir like `languages.toml` (0005).
- **Zero-config default**: no config → offer "debug current binary /
  current test" via automatic scenario discovery (zed's model: cargo
  test/bin targets for Rust, the current file for Python). This is the
  zero-config doctrine (AGENTS §5) applied to debugging.

## UI design — editor-native, not an IDE sidebar

Research: vscode (activity-bar sidebar + hover evaluate + inline
values), zed (bottom-docked panel, sessions as tabs, pane-tracking
stepping), helix (gutter breakpoints + picker-based variables), nvim-dap
(everything is a floating window — the cautionary tale).

Strop's answer follows the git surfaces (0010) and the commit-tree
sidebar, not the IDE sidebar:

1. **Gutter is the live surface**: breakpoint glyphs (●/○ for
   enabled/disabled, adapter-verified breakpoints get a distinct mark),
   current-instruction arrow (▶) on the stopped line. Both are
   decorations on the *real document*, not a panel.
2. **A stopped session opens a debug layout**: current pane keeps the
   source; a vertical split hosts two stacked readonly buffers —
   `▸ stack` (frames; Enter jumps to the frame's source line in the
   source pane — zed's pane-tracking rule: the debugger reuses the
   pane where the file is already open) and `▸ variables` (tree rows,
   Enter expands children — the same row→dive pattern as the commit
   browser). Program stdout/stderr is a third buffer, a real buffer
   you can search and yank from.
3. **Stepping is motion, not buttons**: `Space D` prefix —
   `c`ontinue, `n`ext (step over), `i` step in, `o` step out,
   `r`estart, `t`erminate, `b` toggle breakpoint, `B` conditional
   breakpoint (modal text field for the condition), `x` evaluate the
   word under the cursor (result lands in the hover card — the LSP
   hover path, not a new widget).
4. **The session has a mode, visibly**: the modeline shows
   `DEBUG ▶ main.rs:42` while stopped; stepping keys only exist while
   a session is live. No idle-state debug UI cluttering the editor.
5. **Everything async** (0001 §3, non-negotiable): DAP is LSP's exact
   shape — a `strop-dap` crate with a `Client` modeled on strop-lsp's
   (tokio task per adapter, jobs post `DebugEvent`s onto the event
   loop, revision-keyed responses so a stale stack frame never paints
   over a moved cursor). The input path never waits on an adapter.

## Explicit non-goals (v1)

- No debug console REPL (evaluate-via-hover covers 90%; a REPL is a
  terminal emulator problem — revisit if it hurts).
- No multi-session UI (one session; restart beats parallel sessions
  for the three toolchains we support).
- No exception-breakpoint matrix UI (adapter default behavior on).
- No inline values during stepping (render-layer cost; the variables
  buffer is the honest version).
- No `tasks.json` subsystem (see launch.json note above).
- No attach-by-process-picker in v1 (attach by pid in config only).

## What must exist first

- 0017's `LineLayout` (gutter decorations and the debug arrow share
  the display-column work).
- 0018's service envelopes (DebugEvent is another envelope consumer).
- Split-pane robustness from daily driving the current layout.

## Acceptance shape (when scheduled)

- Rust: `cargo test` binary debuggable with zero config; breakpoint,
  step, variables tree, stack jump — all verified headless against
  lldb-dap like the LSP clangd fixture tests.
- Python: `debugpy` debugs the current file with zero config.
- C++: the kafka-style clangd fixture debugs with zero config.
- `?` lists every debug key; the differential-style harness drives a
  scripted debug session end to end.
