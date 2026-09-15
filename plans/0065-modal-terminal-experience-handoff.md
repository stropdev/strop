# 0065 — Modal terminal experience: handoff and feasibility

Status: handoff only — **no source changes are authorized by this document**.
Written 2026-09-15 against the current worktree (post-0055 TUI milestone:
`strop-terminal` with the vendored ghostty-vt emulator, real PTY sessions and
engine integration in `editor/terminal/`). Another agent is active on this
worktree: the implementing session MUST `git status`/`git diff` first and
reconcile in-flight work before touching any file named here.

The request (user, 2026-09-15): the built-in terminal should be a **modal
terminal** — a terminal buffer that behaves like a strop buffer. Normal mode:
vim motions, moving up and down through the terminal's text. Insert mode: focus
lands on the terminal's input line and typing goes to the shell, which then
acts like a regular terminal (Ctrl-C and everything else). And the terminal
should be themed like strop.

Verdict up front: **all of this is feasible, and most of the machinery already
exists.** What is missing is (a) honest modal presentation, (b) one real key
decision (Esc), (c) a strop-owned color palette for terminal content, and
(d) inspection-mode polish and discoverability. Nothing below touches the VT
engine, the PTY supervisor or the 0055 T01–T10 contracts.

## 1. What already exists today (verified by code reading)

The terminal is further along than the request assumes. Grounded facts:

- **Real modal buffer.** `:terminal` creates a real document
  (`DocumentSource::Terminal`, `editor/terminal/lifecycle.rs:55-65`) backed by a
  read-only rope projection of the VT screen+scrollback
  (`strop-terminal/src/projection.rs`). It is listed in the buffers picker and
  participates in splits, jumplist and MRU like any buffer.
- **Two interaction states already exist**, both internally `Mode::Normal`:
  - *Terminal-input* (`view.terminal_input = true`,
    `editor/terminal/input.rs:170-217`): `InputOwner::Terminal`
    (`editor/dispatch.rs:49-51`) routes raw physical input to the PTY before
    any editor normalization. Ctrl-C is a byte to the child, Esc goes to the
    child, Ctrl-R/Ctrl-L/arrows work. The statusline shows a `TERMINAL` chip
    (`render/statusline.rs:157-165`) and the cursor sits at the child's own
    cursor (`render/terminal.rs:89-94`). Entered with `i`/`a` from Normal on a
    terminal buffer (`editor/dispatch.rs:130-135`).
  - *Terminal-Normal inspection* (`enter_terminal_normal`,
    `editor/terminal/input.rs:138-143`): reached via `Ctrl-\ Ctrl-N` or the
    `Ctrl-W N` window prefix (Vim's `t_CTRL-W` grammar). Mode becomes Normal;
    motions, `/`/`?`, visual selections and yanks operate on the projected
    text. The buffer is `readonly` (`Document::output`,
    `editor/document/mod.rs:114-124`), so text-changing operators refuse
    rather than corrupting the projection.
- **Pinned-snapshot semantics.** While any pane inspects a terminal, live
  output keeps arriving but the inspected buffer stays pinned
  (`editor/terminal/lifecycle.rs:191-199`); a statusline "new output"
  indicator appears (`render/statusline.rs:690-692`). This is deliberate —
  0055 §7 forbids yanking bytes from a row that moved under an old highlight.
- **Child colors pass through verbatim.** The live view maps each cell's VT
  style through the frame's `Palette` (`render/terminal.rs:14-48`), and that
  palette is **ghostty's compiled-in default**, read back from the native
  engine (`strop-terminal/src/vt/frame.rs:175-178`). Strop's own palette
  (`render/mod.rs:32-56`: BASE `#16161e`, TEXT `#e8e4da`, ACCENT `#f0a35e`,
  …) is not consulted for terminal content. Only chrome (statusline, the
  "terminal starting" placeholder) is strop-themed today.

So the user's model — Normal navigates, Insert types into the shell — is the
implemented architecture. The gap is presentation and polish, not feasibility.

## 2. The actual gaps

| # | User expectation | Reality today | Severity |
| --- | --- | --- | --- |
| G1 | Insert mode = typing into the terminal | Terminal-input is `Mode::Normal` internally with a statusline override; it is not the editor's Insert mode | UX honesty |
| G2 | Esc leaves "insert" back to Normal | Esc goes to the child by design (0055 §7, T02); the exit is `Ctrl-\ Ctrl-N`, which is undiscoverable in practice | **Decision required (D1)** |
| G3 | Scroll up/down in terminal text | Works, but only after the hidden `Ctrl-\ Ctrl-N`/`Ctrl-W N` escape, and the view is a pinned snapshot with no obvious refresh path | Discoverability + polish |
| G4 | Terminal themed like strop | Content uses ghostty's default palette; strop's BASE/TEXT/ANSI language is absent | Theme seam missing |

## 3. Decisions for the user (record outcomes in 0028)

### D1 — What Esc does in terminal-input

This is the one genuine conflict and it contradicts a load-bearing requirement,
so it MUST NOT be decided silently by the implementing agent.

- **Option A (recommended): keep Vim's grammar.** Esc stays child-bound (nested
  `nvim`/`htop`/REPLs keep working); `Ctrl-\ Ctrl-N` exits to Normal. This is
  what Vim (`t_CTRL-\_CTRL-N`) and Neovim do, what 0055 §7/T02 mandate, and
  what the fidelity doctrine (AGENTS.md §1) favors. The fix for the user's
  friction is discoverability (W5), not rebinding.
- **Option B: Esc exits to Normal; literal Esc needs the prefix.** Matches the
  user's modal mental model; breaks nested modal applications' primary key and
  requires amending 0055 §7 and its T02 evidence with the user's explicit
  approval recorded in 0028. If chosen, `Ctrl-\ Esc` (or `Ctrl-V Esc`) must
  deliver the literal byte, and the hint must render in the message line.

Default to A unless the user explicitly confirms B.

### D2 — Inspection view: pinned (keep) plus a refresh affordance

Keep the pinned snapshot (it is a correctness invariant, not a limitation).
Add an explicit, documented refresh: when the statusline shows new output, a
documented key (proposal: `R` is taken; use `:terminal-refresh` plus showing
the hint in the new-output message) re-installs the latest frame through the
existing `install_terminal_frame` path. Do not auto-refresh under a live
selection; auto-follow when the cursor sits on the last row and nothing is
selected is acceptable and matches "returning to input follows the live
cursor" (0055 §7).

### D3 — Palette ownership

Terminal default colors must come from one strop-owned source, not be
duplicated. Today the strop palette constants live in the frontend
(`crates/strop/src/render/mod.rs`), which the engine cannot import. Introduce
a small shared theme seed in `strop-core` (base fg/bg plus the ANSI-16
derivation) consumed by both the existing render constants and the terminal
palette construction. This is a move, not a second convention: the render
constants re-export or derive from the seed. Config-file overrides (0005) are
a follow-on, not part of this change.

## 4. Work plan

Ordered; W1 must precede everything, W2–W4 are independent after it.

### W1 — Record the current truth before changing it

Drive the real binary (never only TestBackend): open `:terminal`, run a
command with colored output (`ls --color`, `git log`), enter/exit
terminal-input, enter inspection via both escapes, yank a line, run output
while inspecting, split the terminal, exit the shell in a visible pane. Record
exactly which of G1–G4 reproduce as felt UX problems — the user's report is
qualitative, and the implementing session needs a concrete before/after
baseline. Headless-drive equivalents (`headless/driver.rs`) for each step
become the regression spine.

### W2 — Make terminal-input an honest mode presentation

Goal: the user can tell at a glance which of the two states they are in, and
mode-dependent editor behavior stops lying.

- Decide the presentation: either keep `Mode::Normal` internally and make the
  statusline chip/message/cursor style fully distinct (mostly true today), or
  set `Mode::Insert` while terminal-input is active so the chip reads
  `INSERT`. The latter matches the user's mental model but requires auditing
  every `mode == Mode::Insert` conditional (~15 sites; `dispatch.rs`,
  `cursor.rs`, `block.rs`, `view.rs`, `registers.rs`, `normal/execute.rs`,
  `editor/mod.rs`) for behavior that must not activate while the InputOwner is
  Terminal — physical input already bypasses `feed_document`, so the exposure
  is display and overlay-forwarding paths, not grammar. `InputOwner::Terminal`
  still wins key ownership either way; this is presentation, not a second
  input path.
- `i`/`a` already enter terminal-input; also confirm `A`/`I` semantics are not
  silently different (they currently are not wired — `i`/`a` only,
  `dispatch.rs:131`). Keep it minimal: `i` and `a` only, matching Vim's
  terminal job mode.
- On an exited terminal, `i` already shows the final snapshot with a truthful
  message (`input.rs:178-196`). Keep.

### W3 — Esc and escape-prefix discoverability (implements D1)

- Option A: no routing change. Make the existing escape discoverable: the
  entry message already names `Ctrl-\ Ctrl-N`; additionally surface it in the
  `?` help (0003 §5.7 requires every dispatchable keybinding to render there —
  audit that `Ctrl-\ Ctrl-N`, `Ctrl-W N` and `Ctrl-W .` are listed) and in
  the statusline hint for terminal buffers.
- Option B (only with explicit user sign-off): route Esc to
  `enter_terminal_normal` in `feed_terminal`, add the literal-Esc prefix
  sequence, amend 0055 §7/T02 wording and re-run the nested-application
  evidence (real `nvim --clean` inside the terminal, Esc verified reaching it
  via the prefix).
- Either way: focus loss already cancels prefix state (`input.rs:263-276`);
  keep that invariant and its tests.

### W4 — Strop-themed terminal content (implements D3)

- Add the shared theme seed in `strop-core` (per D3): base foreground
  (`#e8e4da`), background (`#16161e`) and a 16-color ANSI set harmonized with
  the existing accent/secondary/diagnostic colors. Indexed colors are the
  whole harmonization lever — `ls`, `git`, prompt themes all draw from them.
- Thread a `Palette` into session start: `Service::start` /
  `launch::Launch` gains the palette; the vt layer configures the native
  engine at creation instead of accepting ghostty defaults
  (`strop-terminal/src/vt/ffi.rs` already declares the `Palette` struct that
  crosses the ABI; `vt/frame.rs:175-178` already reads it back, so configured
  values flow to every frame with no projection changes).
- Rendering needs no structural change: `render/terminal.rs` already resolves
  `TerminalColor::Default` through the palette and passes explicit RGB
  through untouched — child-chosen truecolor stays truthful, defaults become
  strop's. Verify the pane background fill (`render/terminal.rs:65-68`) and
  the inspection path (`render/buffer/content.rs:68-70`,
  `terminal_style_at`) both follow the new palette.
- The theme seed replaces the render constants' values in place; do not leave
  two color definitions drifting.

### W5 — Inspection polish (implements D2)

- Add the refresh affordance (command + message hint) from D2 through the
  existing `install_terminal_frame`; keep the pinned invariant and its
  selection-integrity guarantees.
- Confirm the full motion surface on the projection with tests, not
  assumption: `j/k`, `gg/G`, `Ctrl-D/Ctrl-U`, `/`/`?` + `n/N`, visual yank,
  and cursor placement over wide/CJK cells (the projection's cell↔byte mapping
  is the risky seam, not the motions).
- Keep "output does not drag the view" behavior; the statusline indicator
  stays the signal that refresh is available.

### W6 — Docs, help, changelog, plan bookkeeping

- Update terminal docs and the `?` help for whatever D1/D2 land; changelog
  entry.
- Register this plan in `plans/0028-roadmap-and-review.md` and add a one-line
  cross-reference from 0055 §7 pointing here for the modal-experience layer.
  If D1 lands as Option B, that same change amends 0055 §7 with the user's
  recorded approval.

## 5. Verification

- Every W2–W5 behavior gets a headless-drive or engine test that fails before
  and passes after; golden `TestBackend` cells for the statusline chips,
  palette mapping (indexed/default/RGB through the new seed) and the refresh
  indicator. Follow the existing `render/terminal_tests.rs` vt100 outer-screen
  pattern for at least one end-to-end colored-output case.
- Real-terminal evidence (the user's Windows Terminal → WSL path included):
  a full-screen nested app, `ls --color` under the new palette, resize with
  the terminal in a split, and the Esc behavior chosen in D1.
- Gate: `docker compose run --build --rm test` (fmt, locked clippy
  `--workspace --all-targets -D warnings`, locked tests). No `unwrap`/`expect`
  additions outside tests; the input→render path stays allocation-clean — the
  palette is one `Arc` clone per frame, never per cell.

## 6. Non-goals (already owned elsewhere)

Remote/SSH/container terminals, mouse forwarding, session persistence,
terminal graphics, and all GUI work remain with 0055/0061. Do not touch the
emulator, the PTY supervisor, the input encoder or the projection format —
this plan is the editor-experience layer only. Do not add a theme engine or
config surface beyond the single strop-core seed; 0005 owns configuration.
