# 0046 — Engine extraction: one `strop-engine`, TUI and headless as consumers

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
