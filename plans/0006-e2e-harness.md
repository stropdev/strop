# 0006 — E2E Test Harness

> Rootle's e2e (Python + pyte + real PTY) worked but proved flaky. Strop's architecture
> (0001 §5 decisions 6/7/10) lets the PTY stop being the test interface — that is where
> the flakiness lived. This plan moves e2e in-process and leaves the PTY a thin shell.

Status: accepted-in-principle. Lands with M0 (the harness is the M0 safety net, not an
afterthought).

---

## 1. Why rootle's harness flaked (the postmortem in one paragraph)

The PTY was the test interface: spawn the binary on a pseudo-terminal, inject bytes,
reconstruct the screen with pyte, assert on text. Every flake class follows from that:
timing (settle windows are heuristics — the "quiet window" pump adapts but still races
slow CI), VT-emulation fidelity (pyte is not the terminal; divergence looks like app
bugs), and input-byte ambiguity (ESC merging into Alt+<key> — rootle's own rules list).
The app under test was never wrong often; the *observation channel* was.

## 2. The strop design: three tiers, each deterministic or thin

```
Tier 1  differential harness (0001 §5.10)   grammar fidelity vs nvim      pure, CI gate
Tier 2  headless e2e (this doc)             full editor, scripted keys    pure, CI gate
Tier 3  PTY smoke                           crossterm wiring only         2–3 tests
```

### Tier 2 — headless e2e, in Rust, no PTY

The editor core already renders into ratatui `Frame`, and crossterm lives only at the
binary edge (0001 §5.7). So the binary grows a headless surface:

```
strop --headless [--script FILE | --keys "ci[..."] [--state-json]
```

- Synthetic `crossterm::event::KeyEvent`s enter the *same* input path real bytes reach
  (the byte→event parse is the only thing skipped; it is Tier 3's job).
- Frames render to `ratatui::backend::TestBackend`; assertions are on the cell grid —
  text, styles, cursor — golden-snapshot style.
- `--state-json` dumps cursor/mode/registers/jumplist — the same state the differential
  harness diffs against nvim.
- Zero timing: one keystroke in, pump the event loop to quiescence (deterministic —
  the async invariant means input handling never awaits; data-source jobs are fed by
  the test harness injecting completion events, with fake clocks where needed).

Test files are plain Rust `#[test]`s in `strop/tests/`, one concern per file
(`preview.rs`, `picker.rs`, `git_surfaces.rs`, …), each test a scripted session:
open fixture → keys → assert screen/state. Fixture files live in `e2e/fixtures/`
(shared with demo tapes where sensible — 0004 §4: a demo that lies is a bug, and so is
a fixture).

What Tier 2 kills vs. rootle: no pyte, no settle heuristics, no HOME/XDG sandboxing
(the process never touches real state — headless mode takes explicit paths), no flake
budget, runs in `cargo test` in the docker `test` stage (0002 §4).

### Tier 3 — PTY smoke (deliberately tiny)

What headless cannot prove: crossterm byte parsing, alt-screen enter/leave, raw mode,
SIGWINCH resize, OSC52 emit, terminal capability detection (kitty protocol). A handful
of tests drive the real binary on a real PTY (Rust `portable-pty`, not Python/pytest —
one toolchain, no second test stack to keep green) and assert only wiring-level facts:
alt screen entered, first frame non-empty, `:q⏎` exits 0, resize re-renders. If one of
these flakes, the harness is wrong, not the test — the assertions never depend on
timing-sensitive content.

### Tier 1 cross-reference

The differential harness (nvim) owns *grammar* truth; Tier 2 owns *product* truth
(pickers, git surfaces, previews, sessions). Same scripted-session shape, different
oracle. Don't blur them.

## 3. What we deliberately do NOT build

- No Python/pytest layer. Rootle needed it because Python was already the e2e host;
  strop's harness is Rust end to end — one toolchain, one CI gate (`cargo test` in the
  docker `test` stage), no second lockfile.
- No asciinema recording in the test path. Demo capture is VHS (0004), driven by tapes,
  not by the test harness doing double duty.
- No coverage % target. Coverage is enforced by contract tests (keybinds-popup coverage,
  0003 §5.7) and the differential corpora, not a number.

## 4. CI wiring

Tier 1 + Tier 2 + Tier 3 all run inside `cargo test` in the Dockerfile `test` stage
(0002 §4) — one gate, hermetic, no network (fixtures vendored; nvim for Tier 1 is
`apk add neovim` in the image, pinned). The release verify steps (0002 §5) remain
tarball-level smoke; they never duplicate the harness.

## 5. Deferred

- Fake-clock story for git/LSP data sources (needed at M2/M5; the event-injection seam
  is built from M0).
- Property tests for the rope/undo core (proptest) — M4, when the undo tree lands.
- PTY smoke on macOS CI runners (Tier 3 is Linux-first; macOS adds `script(1)`
  quirks — defer until the darwin release path exists and needs it).

## 6. Architecture, whole-core verification and later feature extensions

[0056](0056-architecture-prerequisites.md) extends the shared engine/headless
contracts with bounded protocol/stdio/WSL driving, recovery/lifetime schedules and
real native evidence before any GUI. It preserves the existing differential/TUI/
PTY tiers. [0060](0060-debugger-workflow-and-architecture.md) adds DAP-specific
ordering tests and real adapter/transport journeys through those contracts.

[0061](0061-gui-windows-and-wsl.md) adds deterministic GPUI components plus actual
native Windows rendering/capture, input, IME/accessibility and Windows+WSL integration.
Headless semantic data does not prove pixels or OS input, and a missing Windows
headless renderer is not a successful screenshot gate. Its native driver/capture
extends the proven semantic protocol; it does not invent another engine test path.

Permanent harness/driver code stays Rust-first. Disposable research probes are
not a second maintained Python/PowerShell stack, and no fixture skip substitutes
for a required implementation/release lane.

The separate [0057](0057-core-verification-and-assurance.md) release follows
architecture immediately, before 0058 worker, 0059 completion, debugger or GUI. It covers the
whole core: editing/input/projections, local and remote filesystem authority, exact
Rust/Python helpers through editor reconciliation, containers/processes/queues,
terminal/recovery/UI-stdio, privacy and install/publication effects—not only protocols.

VF01–VF20 require reviewed contracts, calibrated TLA+/TLC, generalized TLAPS safety,
same-source Verus and actual-code Loom/correspondence/helper/native campaigns.
The new `tlaps` and `core-assurance` gates supplement existing `test`/`model`/`verify`
and required native lanes. PR/tag evidence binds the exact source/helper/artifact
candidate and declared targets. Zero/subset proofs, missing tools or parser errors
cannot pass. Models/tests remain valuable without being labelled universal proofs
of the implementation, CPython or the OS. Later features extend affected claims.

[0058](0058-unified-native-worker.md) follows the accepted baseline. Its WK16–WK20
explicitly migrate VF claims, models/theorems/production kernels and actual native
correspondence to the new worker; the already-dispatched 0057 agent must finish
verifying its own actual implementation, not future code. Local tests cross the
same real codec/client/worker as SSH/container tests. Add deployment/activation/cache-
lease and child/control-stream cases to the same required gate framework. No legacy
Python job silently skipped, toy local bypass or inherited badge qualifies the cutover.
