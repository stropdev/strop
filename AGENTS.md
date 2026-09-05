# AGENTS.md — working rules for strop

Strop is a modal text editor in Rust (see `plans/0001-vision-and-design.md` — the design
contract; numbered plans in `plans/` are load-bearing decisions). These rules govern any
agent or human working in this repo.

## Doctrine (never trade these away)

1. **Fidelity**: the vim grammar is the product. Deviations are bugs, and bugs are found
   by the differential harness (0001 §5.10), not by vibes.
2. **The preview cannot lie**: preview and execute consume the same pure resolver in
   `strop-grammar`. Never special-case the preview.
3. **Async invariant**: input→render never crosses an `await`. Data sources (rg, git,
   LSP) are jobs posting onto the event loop. No exceptions, no "temporary" blocking
   call in the input path.
4. **Everything is a real buffer.** No widget that forgets how to be a text editor.
5. **Zero-config single static binary.** No required setup, no openssl in the tree
   (rustls-only rule — a dependency dragging in openssl is a bug), `git2` with
   `default-features = false` (0002 §2.3).

## Code quality

- **Refactor aggressively.** Big refactorings are welcome when they raise quality —
   rename, split, delete, restructure. Never leave shims, aliases, deprecated paths, or
   "old way / new way" pairs behind: migrate every caller, delete the old code, same
   change.
- **File size discipline.** ~800 lines is the ceiling for a source file unless the file
   is a well-established single-pattern artifact (a generated table, a keymap listing).
   Past the ceiling, split by responsibility — rootle's `settings_popup/{mod,render,
   sections}.rs` shape is the pattern: one component, one directory, files by concern.
   Readability is the test, not the number.
- **Boring > clever.** No needless abstractions, no speculative generality, no
   dependency for a twenty-line function. Delete weightless code.
- **Fix sources, not symptoms.** No warning suppression, no special-cased inputs, no
   swallowed errors. If an invariant is load-bearing, `debug_assert` it.

## Testing

- Coverage is by contract, not percentage: every observable behavior a user can hit has
   a test; every dispatchable keybinding renders in `?` (0003 §5.7); every motion in the
   corpora diffs clean against nvim.
- Test behavior, boundaries, invariants — not plumbing, not source text, not incidental
   defaults. Deterministic and hermetic: no network, no real `$HOME`, no wall-clock
   sleeps. The e2e tiers live in `plans/0006-e2e-harness.md`.
- Golden cell-grid snapshots via `TestBackend` for anything visual; state assertions for
   anything grammatical.

## Code structure

- **File size is a design signal.** ~400 lines is healthy, ~800 is the
  ceiling, past that split by responsibility (helix's `commands.rs` and
  zed's crates are the cautionary tales). An editor subsystem file that
  grows past the ceiling becomes a directory of modules by concern —
  `editor/normal/{mod,dispatch,operators,ex}.rs`, not
  `editor/normal.rs` at 1.5K. When a pattern's structure is unclear,
  research how helix/zed/neovim structure the equivalent before
  inventing one.
- After landing a feature that pushed a file past the ceiling, the
  follow-up split is part of the same change — not someday.

## Rust specifics

- `cargo fmt --check` + `cargo clippy --locked --workspace --all-targets -- -D warnings`
  + `cargo test --locked` are the gate (docker `test` stage, 0002 §4). All three green,
  always, before a change is done.
- No `unwrap`/`expect` outside tests and `main`'s top edge; libraries return typed
  errors (`thiserror`), the binary aggregates (`anyhow` at the edge only).
- No `unsafe` without a comment naming the invariant it relies on and why safe Rust
  can't express it.
- Hot paths (input→render, per-keystroke preview): no avoidable allocation, no
  `String` materialization of rope content, no per-keystroke process spawn (0001 §3:
  libgit2 for git hot paths, not shell-outs).
- Plans before code for anything architectural: add/amend a numbered doc in `plans/`.
- **Docker compose is the default build/test path.** `docker compose run --build --rm test`
  is how changes get validated — layer caching keeps it fast, and the source tree is
  never clobbered with build assets (no host `target/`, no stray tool installs). Host
  `cargo` is acceptable for rapid iteration mid-change, but "done" means the compose
  gate passed; never commit build artifacts.
- Release: tags `v*`; the workflow (0002 §5) owns crates.io, tarballs, homebrew tap,
  site redeploy. Never hand-publish. A version bump is three moves, all in the
  tagged commit: workspace `Cargo.toml` + the member crates' inter-dep pins +
  `cargo check` to sync `Cargo.lock` (the release gate runs `--locked` — a stale
  lock fails the tag's builds).
- The demo must never lie (0004 §4): tapes drive the real binary; a demo-only code path
  is a bug.
