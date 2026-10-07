# 0067 — New-file saves, large-repo finders, and quiet diagnostics while typing

Status: **implemented** (2026-10-07) — the user reported three issues and asked
for research-backed UX judgment on the third; all three landed with their
evidence below.

## 1. `strop <new-file>` refused its first save

**Symptom:** opening a file that does not exist yet and running `:w` failed with
`write failed: InvalidPath: an absolute non-control resource path is required`.

**Cause:** the CLI open path (`Buffer::open`) keeps the relative name it was
given when `canonicalize` cannot resolve a missing file. `prepare_save` then
targeted that relative path, and the store guard (`strop-fs/guard.rs`)
correctly refuses relative targets.

**Fix:** `request_save_document` resolves a relative plan target against the
editor's cwd at admission (the only place with explicit cwd ownership — no
`:cd` exists, so launch cwd is the only base). Display name and save-acceptance
identity (`origin`) are untouched. Existing files were never affected because
`canonicalize` succeeds for them.

**Evidence:** headless repro of the exact message; regression
`a_cli_opened_missing_file_saves_under_the_editor_cwd` (red before, green
after); live repro now writes the file.

## 2. File finder on large repos showed a hard error

**Symptom:** on repos over the catalog bound, `Space f`/`Space /` showed the
red error "search results exceed 100000 rows or 64 MiB; narrow the query".

**Cause:** the catalog memory budget (100,000 rows / 64 MiB retained, plus the
4 MiB single-batch bound) is correct backpressure — the doctrine forbids a
silent cap — but surfacing it as a source *failure* made ordinary large-repo
use look broken even though the bounded prefix stayed ranked and openable.

**Fix:** a catalog bound is now a **truncation, not a failure** — the source
stops walking, the results that already streamed stay, and the truthful note
("results exceed 100,000 rows or 64 MiB; kept what streamed — narrow the
query") arrives as the picker's quiet warning slot, following the existing
workspace-symbols truncation precedent. Enter keeps working on the bounded
prefix; typing narrows and re-runs; real delivery failures still fail.

**Evidence:** `flow::tests::the_catalog_bound_finishes_with_a_truncation_
warning` and `a_closed_consumer_still_fails_the_source`; the grep end-to-end
test re-pinned from the error contract to rows→warning→success; live headless
run shows the warning, not the red error.

## 3. Diagnostics noise while typing — research and the modal answer

**Symptom:** with LSP enabled, partial words get amber markings while typing —
everything looks broken mid-thought.

**Research (2026-10-07):**

- Language servers *are* that eager: `publishDiagnostics` pushes on every
  change; rust-analyzer computes syntax diagnostics per keystroke and native
  diagnostics on its own cycle. Throttling servers is not the editor's lever.
- **Neovim's built-in default is `update_in_insert = false`**: diagnostics do
  not repaint during Insert mode; they refresh on InsertLeave. That is the
  mainstream modal answer.
- Helix has the identical open complaint
  ([discussion #13596](https://github.com/helix-editor/helix/discussions/13596):
  "of course `#incl` is an error, I'm still trying to type out `#include`")
  and no debounce today.
- Zed underlines everything live and offers severity filtering and an off-by-
  default inline lens — subtlety, not freezing.

**Decision (strop):** adopt Neovim's default semantics, strop-flavored. While
the edited document is in Insert mode, diagnostic publishes for it are
**stashed, not displayed**; the frozen set's revision stops matching on the
first edit, so typing paints no underlines/gutter/EOL note at all (cleaner
than Neovim's stale extmarks, which can sit on shifted text). Publishes for
*other* documents stay live (split panes). At InsertLeave the latest stash
applies when it still names the buffer's revision — the common case, so
underlines return the instant Esc lands; an older stash is dropped because the
next publish for the unedited buffer arrives on its own. No timers, no
debounce — the mode transition is the signal, exactly the modal contract.

**Implementation:** `Editor::record_diagnostics` (the freeze decision) routes
the one ingestion site; `apply_pending_diagnostics` runs at the three
InsertLeave sites (Esc in insert, visual-block insert end, occurrence session
end); the stash is cleared with its document.

**Evidence:** three engine regressions (freeze/stash/leave-apply,
other-document liveness, stale-stash drop); a frontend cell-grid test pinning
that the undercurl actually paints; a **live capture against an eager fake
LSP server**: 3 publishes during Insert typed `xyz` → 0 underlined cells
through every interim frame, 4 underlined cells the frame after Esc.

## Release

Shipped in 0.40.0; container gate, inventory and the committed completion
measurement re-qualified on the musl artifact as before.
