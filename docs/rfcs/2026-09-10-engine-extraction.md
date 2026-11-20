---
rfc: "2026-09-10-engine-extraction"
title: "Extract the editor engine from the terminal frontend"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-09-10"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/142"
milestone: "v0.22.0"
supersedes: ["2026-09-09-handoff-adoption-and-roadmap"]
superseded_by: []
blocked_on: []
legacy_id: "0046"
---

# RFC: Extract the editor engine from the terminal frontend

v0.22.0 moved editor orchestration, service routing, sessions and replay into crates/strop-engine, independent of Ratatui and Crossterm. The strop binary kept the terminal, rendering and headless frame output; a GUI was not required for the extraction.

[Tracking issue #142](https://github.com/stropdev/strop/issues/142) · [Published v0.22.0](https://github.com/stropdev/strop/releases/tag/v0.22.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0046-engine-extraction.md).

Supersedes: [0041](2026-09-09-handoff-adoption-and-roadmap.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: planned; stage A landing. This is the handoff's S2, scoped to what
the current consumers need — no speculative GUI API.

## 1. The cut

- **`strop-engine`** owns everything semantic: the full `Editor`
  orchestration (editor/), keymap, session, config, files, replay
  (`trace/drive` + `replay.rs`'s `run_full`), and the headless
  script-driver minus its frame rendering. No Ratatui/Crossterm types in
  its dependency graph (exit criterion).
- **The binary** (`crates/strop`) keeps: `main.rs`, `cli.rs`,
  `terminal.rs`, `render/`, `update.rs`, `bench.rs`, and the headless
  `frame` rendering — the physical terminal and cell-grid production.

## 2. The seams (decided)

1. **Editor view access is read-only and borrowed.** `render()` today
   takes `&mut Editor` only to launch hunk refresh — that job moves out
   of render (refresh on view/edit events). Rendering reads through
   `&Editor` accessors; no new snapshot-clone layer (the handoff forbids
   clone-per-frame).
2. **Frame capture is injected.** The engine's trace/drive records
   logical observations; cell grids come from a `FrameRenderer` the
   composition root injects (the binary's implementation renders via
   TestBackend). TUI cell replay and future GUI replay share the logical
   observations, not pixels.
3. **Headless splits at the same seam.** Script parsing, input driving,
   and state/buffer directives live in the engine; `frame` output is the
   binary's. The script vocabulary and `--headless` behavior are
   unchanged — the demo must never lie (0004 §4).
4. **Replay stays whole.** `--replay` is a binary command driving the
   engine plus the binary's frame renderer; recorded schedules replay
   natively-free, as today.

## 3. Stages and gates

- **A: physical move.** `editor/**`, `keymap/`, `session*/`, `files.rs`,
  `config.rs` move to `crates/strop-engine`; `crate::editor::…` becomes
  crate-local paths; the binary consumes `strop_engine::`. No API change
  beyond visibility (pub where the binary reads). Gates: full compose
  test + model + verify green; bench before/after on the stress
  fixtures; zero behavior change is the claim — the differential corpus
  and the replay suite are the proof.
- **B: Ratatui-free engine.** `editor/trace/frame.rs` moves to the
  binary behind the injected `FrameRenderer`; engine Cargo graph loses
  ratatui/crossterm (dev-deps excepted). Gate: `cargo tree -p
  strop-engine` shows neither; replay of an old supported trace still
  passes or migrates loudly.
- **C: API tightening.** Narrow `pub` to what the binary actually uses;
  no `&mut Editor` escape for frontends (S2's anti-bypass rule).

## 4. Explicitly not in S2

GUI prototypes, toolkit evaluation, `strop-gui` scaffolding, a plugin
boundary. The engine boundary exists so a GUI *can* attach later; nothing
here is GUI work.

## 5. Architecture, verification, native worker, then feature consumers

[0056](2026-09-13-architecture-prerequisites.md) owns the remaining stage-C/read-only
render/action/view cutover and migrates the real TUI/headless consumers. Dependency
separation is retained; mutable rendering/job admission is not a finished boundary.
Its non-graphical protocol/server and recovery/lifecycle contracts are delivered
and tested first. [0057](2026-09-13-core-verification-and-assurance.md) then verifies the
stable pre-worker core. [0058](2026-09-13-unified-native-worker.md) subsequently unifies
native services/protocol and requalifies their claims before 0059 completion and
0060 debugger. The worker does not own editor buffers/grammar/history/rendering.

[0061](2026-09-13-gui-windows-and-wsl.md) then consumes that released boundary for native
Windows presentation over WSL. It does not close stage C itself, clone Editor per
frame, fork grammar/services or introduce another generic protocol/recovery owner.
