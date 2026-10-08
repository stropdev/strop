# 0069 — Completion depth: snippets, commit characters, and the hygiene run

Status: **implemented** (2026-10-08) — 0.42.0. The D05 re-entry from 0059 §12:
"a real snippet engine with placeholders, choices, linked tabstops and undo,
before advertising snippet support", plus commit-character acceptance under
its own interaction/safety contract. Bundled with the over-ceiling hygiene
splits the roadmap required before further changes.

## 1. Snippet engine

**Contract honored.** LSP `insertTextFormat: 2` items no longer refuse —
they expand through the validated acceptance plan and open a tabstop
session:

- **Parse** (`strop-lsp::completion::snippet`): pure segment parser —
  `$1`/`${1}`/`${1:default}`/`${1|a,b,c|}`/`$0` and `\$`/`\\`/`\}` escapes,
  malformed constructs as typed errors (no guessed expansion).
- **Expand** (`completion::acceptance`): the plan's replacement text is the
  expanded form (defaults and first choices spliced); tabstops travel as
  typed `(index, per-target-offsets)` — mirrored targets and import shifts
  accounted for. Malformed snippets keep the named `Snippet` refusal,
  never a partial insert.
- **Session** (`editor::snippet`): the first placeholder is selected
  (SELECT_BG, painted like the visual selection in Insert mode, linked
  stops and mirrored targets painted with it); typing replaces it once
  (never re-deletes — spent stops collapse); Tab/Shift-Tab cycle the stops
  with linked indexes moving together; `$0` ends the session with the
  caret there; Esc ends and forwards. Stop offsets follow every edit
  through the same mirrored-edit remap cursors take — verified geometry,
  no journal sniffing.
- **Undo**: the expansion is the acceptance's single undo unit; placeholder
  edits join the insert session's undo unit like any typed text.
- **Choices** expand to the first option with the placeholder selected like
  any tabstop — the dropdown affordance is a recorded boundary, not a fake.
  `allCommitCharacters` (server-level) is out of scope (absent in the
  servers strop supports today).

**Evidence:** 5 parser tests; 3 session tests (expand/cycle/$0/one-undo,
linked mirroring, malformed refusal); a frontend cell test pinning the
placeholder's SELECT_BG; **live capture against a fake LSP server** through
the real binary: menu → accept → expanded text → "name" cells 8–11 painted
with the selection background → typed replace → Tab → "value" replace →
Tab → session ends at $0 → `u` restores the pre-expansion line in one
undo.

## 2. Commit characters

LSP `completionItem.commitCharacters`: typing one of the deliberately
**selected** language item's commit characters accepts it, then the
character types normally. Only a chosen item commits — exactly the Enter
bar; the menu alone never does. Word candidates carry no commit
characters. Tests pin the decision (bound/other-char/no-selection) and the
arm is three lines through the existing admission/apply path.

## 3. Hygiene run (roadmap-required)

- **`editor/notify.rs` (1,109 lines) split by owner** into
  `notify/{mod,queue,subscribe,reconcile}.rs` (50/187/409/525 lines) —
  bounded queue/coalescing; local/remote subscribe and lease identity;
  hint application, guarded reloads, lease observation. `Tape::observe_owned`
  stays at the single drain. All 13 notify acceptance tests pass unchanged.
- **`tests/terminal_editor.rs` (782 lines) split** into
  `terminal_editor/{main,harness,journeys,perf}.rs` (9/326/284/186) —
  real PTY harness, product journey, benchmark cases; completion journeys
  untouched. The real-worker/PTY suite passes unchanged.

## 4. Verification

- Engine 783 + frontend 125 passed; workspace suite green; fmt/clippy
  clean; container gate green.
- Commit characters and snippets are additive; the explicit-acceptance
  experience (Ctrl-Y/Enter after selection, one Esc dismisses) is
  unchanged, as 0059 §6.1 requires.

## Release

0.42.0. The 0.41.0 release's TLAPS lane failed once with the known
prover-backend timeout from 0.39.0 (specs untouched by presentation work;
local `docker compose run --build --rm tlaps` green on the same tree) and
passed on re-run.
