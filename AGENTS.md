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
   When a change pushes a file toward the ceiling mid-flight, fold the split into
   the same patch when it is cheap, or record a roadmap item in `plans/0028` in
   the same commit — never let an over-ceiling file drift uncommented. Test
   modules count toward the ceiling too: a test file past it splits by
   concern into a `tests/` submodule (see `query/plan/tests.rs`). The same
   rule applies to complexity, not just size: an if-chain whose branch order is
   load-bearing is over the limit regardless of its line count; compute typed
   owners as data instead (see `editor/dispatch.rs`).
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

## Verification (plans/0086)

strop follows the ring design: a pure, dependency-free kernel
(`crates/strop-kernel`, `no_std`, no IO) is translated to Lean 4 by
Charon/Aeneas and proved in `proofs/` — theorems are about the shipped code,
never a hand-written copy. Each ring outward (protocol machines, host
adapters, untrusted edges, the product) is checked a cheaper way: property
tests, Loom, real sshd/Docker journeys, the Neovim differential harness.

- **Code moves inward to gain guarantees.** Proving something means moving
  the decision into `strop-kernel` (moved, not copied; production calls it),
  stating the theorem, adding it to `proofs/Audit.lean`'s claims and a
  mutant to `proofs/mutants.py` that must break it.
- **Kernel shape:** plain safe Rust, no dependencies, no closures or iterator
  adaptors in proved code, explicit loops over slices. After changing it run
  `sh proofs/refresh.sh` and commit `proofs/Extracted/` — the gate refuses
  drift between the crate and the committed translation.
- **The survival rule:** an artefact stays only if it is a machine-checked
  proof about shipped code, a test of behaviour a user can see, or a
  counterexample finder with a recorded catch. No evidence inventories,
  digest receipts, freezes or re-pin commits. The two TLC models in
  `specs/` are on probation until 0.55.
- **Every lane blocks.** A gate that may fail without stopping a release is
  deleted, not tolerated.

## Release gates

"Cut a release" means **publicly available**, not "tag pushed" or "draft
created". The release task stays open until every gate below is verified
against the exact local artifact and live public surfaces. Partial progress is
exactly that: partial. The authoritative procedure is `plans/0079` and
`.github/scripts/release-local.sh`.

1. **Local qualification**: a clean committed Linux/WSL checkout whose GitHub
   `ci` run is green (test, model, ssh-pythonfree) passes the
   Compose `test`, `model` and `proofs` gates, the release fixtures, a
   changed-feature smoke and the static artifact checks. `proofs` runs locally
   because its toolchain image does not fit a hosted runner. The supported
   release artifact is `x86_64-unknown-linux-musl`; every other platform is
   out of scope. Never claim an unrun lane passed.
2. **GitHub release**: `gh release view v<x.y.z>` — exists publicly and latest,
   with the Linux tarball, checksum and `catalog.json` built locally from the
   tagged source.
3. **Public installation**: the live catalog matches the local facts and
   `install.sh` installs the public tarball into a fresh directory; the installed
   `strop --version` reports the new version. crates.io and Homebrew are
   historical channels and are not updated by new releases.
4. **Website**: strop.dev serves the new version (version chip, changelog,
   installer). If stale, vendor `CHANGELOG.md`/`install.sh` into
   `stropdev/stropdev.github.io`, push, re-dispatch `rebuild`, then fetch the
   rendered site again.

The release is done when all four checks pass, and not one moment before.

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
- Release: tags `v*`; local BuildKit qualification and manual GitHub publication
  follow `plans/0079` and `.github/scripts/release-local.sh`. Never rebuild while
  publishing a qualified artifact. A version bump is three moves, all in
  the tagged commit: workspace `Cargo.toml` + the member crates' inter-dep pins +
  `cargo check` to sync `Cargo.lock` (the release gate runs `--locked` — a stale
  lock fails the tag's builds). A release is done only when the **Release gates**
  section above passes against the live public surfaces.
- The site must never lie (0004 §4): its frames are captured from the release binary
  (`tools/frames.py` in the site repo); a frame-only code path is a bug.
