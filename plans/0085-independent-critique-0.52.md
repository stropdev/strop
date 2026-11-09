# 0085 — Independent critique of 0.52.0 (code, architecture, verification, build/CI, docs, plans)

- Status: **record** (input to [0086](0086-verified-foundation.md), which decides what is adopted)
- Date: 2026-11-09
- Author: an independent reviewer agent (Claude Fable), commissioned by the owner;
  read-only against `8bb3800` (v0.52.0) with `c309ef8` noted where it already fixes a finding.
- Labels: **[V]** = read in code/data; **[S]** = inferred, not executed. Line numbers are at `8bb3800`.

The text below is the reviewer's report, unedited except for this header. The
adoption decisions — including the two points the owner's agent disagreed with — are in
0086 §2, not here.

---

## 1. Executive summary

1. **The release process and CI have decoupled.** Every `ci.yml` run on `main` since 2026-11-04 is red — eight consecutive releases (0.47.4 → 0.52.0) shipped on a failing gate. 0.52.0 failed in TLAPS after 70 min; 0.51.2 failed in the plain `Gate` (fmt/clippy/test) step; 0.51.1 failed the VF01 inventory digest check and needed a follow-up "Re-pin" commit; the `native-linux` job has failed on every run. AGENTS.md says "never claim an unrun lane passed" — the lanes run, fail, and are ignored. A gate nobody obeys is worse than no gate. **[V]**
2. **The verification programme is ~22k lines of artefacts that have never found a bug in Rust.** 13.3k lines TLA+, 2.3k lines of TLC cfg, 880 lines of gate shell, 3.9k lines of Python, a 422 KB `inventory.json` whose check is a sha256-drift detector, nine of ten Verus "kernels" of the form `a==b ensures a==b`, one verified function with zero production callers, one called with two literal `true` premises. The one real proof is `editmap.rs`. 74 of 347 commits touch `inventory.json`; 16 commits are literally "Re-pin". No plan or changelog entry credits TLC, TLAPS or Verus with a defect. **[V]**
3. **Two hand-maintained copies of the keymap table have already drifted.** `crates/strop-core/src/commands/bindings.rs` (1242 lines) and `crates/strop-engine/src/keymap/bindings_adapter.rs` (1234 lines) are the same table with different handler columns; row `terminal-input` is `"i a"` in one and `"i I a A"` in the other. After `c309ef8` the core copy has **zero consumers**. This is exactly the "old way / new way pair" AGENTS.md forbids, in the one table the `?` help contract depends on. **[V]**
4. **`strop-engine` is a 97k-line crate wrapped around a 106-field god object.** `Editor` has 106 fields and 162 `impl Editor` blocks spread over 137 files. The ~800-line file ceiling was honoured by spreading the same type thinner, not by decomposing it. Four bespoke `mpsc` channel pairs live on the struct beside the "ONE channel" event bus; the longest methods are 404, 321 and 263 lines. **[V]**
5. **`strop-core` is not a kernel; it is a grab bag that ships the Verus runtime.** Its own header says "the buffer… No UI, no modes, no grammar", yet it holds themes, the keymap table, frontend input, worker deploy policy, process ownership and file IO (`tempfile`), depends on `strop-trace`, and pulls `vstd` + two `verus_builtin` crates (19 transitive crates) into the release binary. Any "ring 0" story has to start by taking this crate apart. **[V]**
6. **The test suite is large (2,144 tests) and better than the plans' prose, but the doctrine claims are oversold.** 40 env-gated tests pass vacuously outside Docker (17 silently); default `cargo test` needs host `git`, `sh`/PTY, `python3`, `strip`, `nvim`, `sftp-server`; ~40 wall-clock poll loops; the nvim differential harness is one test with 67 hand-written cases covering roughly half the motion grammar and silently skipping when nvim is absent; the preview invariant (doctrine #2) has exactly **one** range-parity test; no golden cell-grid snapshots exist at all. **[V]**
7. **Build/CI is structurally slow.** The Dockerfile has no cargo cache mounts or dependency layer, so every CI run and every local `docker compose run test` after a source change recompiles the whole workspace and all 750 lockfile packages; the serial `gate` job takes 70–125 min; `native-linux` rebuilds a pinned 0.36.0 baseline binary from source on every push. **[V]**
8. **Verdict on rings/Lean (section 4):** adopt the *principles* (sans-IO kernel, strict inward dependency direction, survival rule, release rule, aggressive trim) now — they are the right medicine for items 2–5. Adopt Lean/Aeneas only for a deliberately narrow ring 0 where a theorem is worth having (edit-map algebra, resolver properties, protocol admission). Do **not** import gripsack's protocol-proof ambition (P1–P4 crash safety) — strop's core claim is vim fidelity, and its oracle is Neovim, not a theorem. Retire TLAPS and the tautological Verus kernels immediately under the survival rule; keep TLC for at most three models for one release and delete them if they catch nothing.

---

## 2. Findings, ranked

### CRITICAL

**C1. Releases are cut on a red CI; the deep lanes are decorative.** [V]
- Evidence: `gh run list --workflow ci.yml` — conclusions `failure` for runs on 2026-11-04 (0.47.4), 11-05 (0.48.0, 0.49.0, 0.50.0, 0.51.0 + "Re-pin"), 11-06 (0.51.1, 0.51.2), 11-07 (0.52.0). Per-job: run 42411981730 (0.52.0) `gate` failed at step "Inductive safety proof (TLAPS)" after 70 min (1/301 obligations in `WorkerSessionProofs`), `native-linux` failed at "Benchmark matched native product artifacts" (`perf::native_terminal_output_under_load_samples`, a 60 s wall-clock poll); run 42307295763 (0.51.2) `gate` failed at step "Gate" (the compose test stage itself) at minute 16; run 42268309030 (0.51.1) failed at "Boundary inventory completeness (VF01)" in 11 s; `native-linux` failed in all three.
- Why it matters: `plans/0079-local-linux-release.md:39-40` says deep lanes "remain CI"; AGENTS.md §Release gates says "never claim an unrun lane passed". Both are satisfied vacuously — the lanes run, fail, and nothing stops. A permanently red CI trains everyone to ignore it, including the `Gate` step that *is* the local release gate (0.51.2).
- Recommendation: decide per lane in one commit: either it blocks (then fix it and make the release script check `gh run view` green for the tag's commit) or it is deleted. There is no third state. Delete `native-linux` (see C3/H7) and the TLAPS lane now (H4).

**C2. Duplicated, drifting keymap tables.** [V]
- Evidence: `crates/strop-core/src/commands/bindings.rs:1-1242` (`kind: CommandKind`) vs `crates/strop-engine/src/keymap/bindings_adapter.rs:1-1234` (`handler: Handler::Leaf(|e,k| …)`); normalised diff = 328 lines, including a semantic divergence at `bindings_adapter.rs:27` `"i I a A"` vs `commands/bindings.rs:28` `"i a"`. `crates/strop-core/src/commands.rs:1-7` claims "The engine consumes this table as its single keymap source" — the engine includes its own copy (`keymap/mod.rs:71 include!("bindings_adapter.rs")`) and reads only `SECTIONS`/`AbsorbKind` from core. The only consumer of the core table was `strop-gui/src/commands.rs:20`, deleted in `c309ef8`; today it has **zero** consumers. No test asserts the two tables agree.
- Why: the `?` help coverage contract (0003 §5.7) is only tested against the engine copy (`keymap/tests.rs`), and both files sit over the 800-line ceiling with no 0028 roadmap entry (AGENTS.md requires one).
- Fix: delete `strop-core/src/commands/bindings.rs`; if a UI-protocol client ever needs rows, project them from the engine table at runtime. Add the missing 0028 line or split the engine table by section.

### HIGH

**H1. `Editor` is a god object; the file-size rule produced thin slices of one type.** [V]
- Evidence: `crates/strop-engine/src/editor/mod.rs:148-420` — 106 fields (`docs`, `remote`, `filesystem`, `notify`, `io`, `completion`, `lsp_state`, `terminals`, `containers`, `devcontainers`, `recovery`, … plus `preview_tx/rx`, `git_tx/rx`, `clip_tx/rx`, `shell_tx/rx` at `:267-268, 287-288, 309-310, 324-325`). `grep '^impl Editor'` = 162 blocks in 137 files. `editor/events.rs:1-5` says every async producer "lands on ONE channel", yet four private channel pairs bypass it. Longest functions: `editor/explain.rs:18 open_explain` 404 lines; `editor/lsp/routing.rs:7 handle_lsp_event` 321; `editor/normal/ex.rs:115 run_ex` 263 (a 31-arm string match); `editor/insert.rs:59 feed_insert` 252; `editor/io/open.rs:29 run` 247.
- Why: every subsystem can reach every other subsystem's state through `self`; nothing in the type system says what `completion` may touch in `recovery`. 0056 AR02 "closed the public boundary" to *frontends* (`editor/api.rs`) but not between subsystems. This is the real "architecture debt" and no ring/proof scheme fixes it until the struct is split.
- Fix: carve subsystem structs with their own `impl` and an explicit `Ctx`/message interface (devcontainers, remote, completion, lsp are already near-independent state blobs); route the four private channels through `AppEvent`.

**H2. `strop-core` is a grab bag, does IO, depends on tracing, and ships Verus.** [V]
- Evidence: `crates/strop-core/src/lib.rs:1-2` ("the buffer… No UI, no modes, no grammar") vs modules `theme`, `commands/bindings.rs`, `frontend_input`, `layout`, `process` ("worker-only child ownership", `process.rs:1-2`), `worker/{deploy_policy,frame_policy,recovery_policy,session}.rs`. `Cargo.toml:9-22`: normal deps `strop-trace`, `tempfile`, `sha2`, `parking_lot`, `libc`, `vstd`, `verus_builtin`, `verus_builtin_macros`. `buffer/io.rs:203` writes files with `NamedTempFile`. `cargo tree -p strop-core -e normal` = 64 crates, of which `vstd` alone contributes 19 and is the sole reason the workspace carries two `syn` majors. `crates/strop-core/src/process.rs:87 pub fn spawn_leased` has no production caller (only `cfg(test)` at `:238-266`).
- Why: doctrine 5 and any kernel/ring story. The crate everyone depends on is the one that depends on the most things.
- Fix: split into `strop-text` (rope, range, editmap, layout — pure) and move theme/commands/frontend_input/worker policies/process to where their consumers are. Drop `vstd` from the release graph (see §4 pilot).

**H3. The verification programme: cost without a documented catch.** [V unless noted]
- Verus: 10 `verus!` files, ~1,460 lines (15 % of strop-core). Real content: `editmap.rs` (`map_position`, `sort_by_start`, `check_batch`, composition lemma `:590-729`). The other nine are tautologies — e.g. `viewguard.rs:30-35 revision_is_stale ensures stale == (prepared != live)`; `searchguard.rs:52 generation_is_live ensures a == b`; `recovery_policy.rs:24-34 recovery_admitted ensures a && b && c`. Defects: `searchguard::completion_is_honest` has **no production caller** (only `editor/picker/lifecycle_traces.rs:285-291`, inside `#[cfg(test)]`), violating 0057 §8's own rule (`plans/0057:488-489`); `strop-worker-deploy/src/deploy.rs:511` calls `activation_admitted(true, true, receipted)`, so the theorem `unverified_object_never_activates` (`deploy_policy.rs:97-105`) is unreachable from production; `strop-ui-protocol/src/frame.rs:12,16,83,89` re-implements the frame bounds by hand with `MAX_BODY_BYTES = 32 MiB` vs the "verified" 1 MiB in `frame_policy.rs:18`.
- TLA+: 20 main specs, 13,325 lines, 127 cfgs; only EditorProtocol (via `editor/transaction_conformance*.rs`, ~1,400 lines of Rust) and SearchLifecycle have any hand-written correspondence; the other ~16 bind to Rust by comments only. Six TLAPS modules prove `Init⇒Inv`, `Inv∧Next⇒Inv'` of abstract models with no refinement; toolchain TLAPS 1.5.0 (2022) compiling Isabelle at image build (`Dockerfile:108-124`); lane red in CI (C1). `_Mutant`/`MUTATION` toggles are well-engineered negative controls for the *models* (`specs/check-model.sh:33-68`) — 105 of 111 registered mutants are these.
- Evidence tooling: `verification/check.py` (683 lines) checks row structure, that named test fns exist, and sha256 of source files + evidence bodies (`:244-306`); `--stamp` re-pins (`:597-645`); `--force` leaves no persisted trace (`:625`). It verifies no semantic property (its own docstring `:26-27`). `inventory.json` = 28 rows / 165 claims / 472 evidence pointers pinning 357 files — e.g. row `theme-presentation` pins `crates/strop/src/main.rs` and `strop-syntax/src/languages.rs`, so a highlighting change trips it (run 42268309030, then commit `d901cfb` "Re-pin …", 34 lines of digests). `check_mutants.py --execute` runs the **3** real Rust mutants with a `RUSTFLAGS` change that rebuilds the whole graph twice. `freeze.py` hashes hashes of 2026-09 baseline JSON whose sha256s are hard-coded in the Python (`freeze.py:31-38,107`).
- No documented catch: grep of `plans/` and `CHANGELOG.md` for "TLC found/counterexample/caught/mutant" yields nothing crediting a model or proof with a Rust defect; the two "found a real fault" passages in 0058 (`:597`, `:1669`) credit native runs and a CI timeout. Loom (9 tests, real primitives swapped under `cfg(strop_loom)`) is the one lane with a documented regression (0.43.1), and only after the campaign was extended (`b48ff00`).
- Fix: keep `editmap` proofs, Loom, `transaction_conformance`; retire TLAPS and the nine tautological kernels now; demote `inventory.json`/`freeze.py` to a plain "which tests cover which plan item" document or delete.

**H4. Test hermeticity and the doctrine claims.** [V]
- Env-gated vacuous passes: `STROP_CONTAINER_TESTS` gates 23 tests (`strop-containers/tests/{docker.rs,incarnation.rs,devcontainer_live.rs}`, `strop-worker-deploy/tests/container.rs`, `strop-git/tests/container.rs`, `strop/tests/container_editor.rs`) with a printed skip; `STROP_REQUIRE_SSH_TESTS` gates 17 (`strop/tests/worker_ssh.rs:54-57`, `remote_ssh.rs:162-164`, `strop-remote/src/pool_tests.rs:10-23`) that **return silently**. Host-toolchain deps in default `cargo test`: `git` CLI fixtures without `GIT_CONFIG_GLOBAL`/`HOME` isolation (`strop-engine/src/editor/test_support.rs:254-300`, `editor/git/tests.rs:45-70`) while `git_memory/tests/native_paths.rs:17-20` does isolate; real `sh` in a PTY (18 engine unit tests); `python3` as a fake LSP server (`editor/lsp/tests/navigation.rs:606`); `strip` ungated (`strop/tests/worker_stdio.rs:399`). ~40 `Instant::now() < deadline` poll loops (e.g. `notify/tests.rs:17-57` 30 s; `terminal_editor/harness.rs:234-300` 60–90 s); `STROP_JOBS_BUDGET_MS` raised to 300 s in `Dockerfile:36`. Test binary re-executes itself in 5 places to isolate process-global theme/config state (`strop-engine/src/config.rs:505`, `editor/explain.rs:726`, `strop/src/render/theme_tests.rs:47-55`) — a product smell (global mutable theme) leaking into test architecture.
- Differential harness: `crates/strop/tests/differential.rs` — one `#[test]`, 67 `Case`s, 1 known divergence; silent `return` when nvim is missing (`:486-501`); covers ~14/26 `Motion` variants (no `h/k/F/T/?/W/B/E/ge/gE/{/_`), no `<<`, no `ds/cs/ys`, no visual-block. Neovim is `apk add neovim` unpinned (`Dockerfile:22`) though 0006 says pinned.
- Preview doctrine: exactly one test asserts preview range == executed range (`editor/tests/search_tests.rs:68`), plus three for `:s`. Nothing loops the differential cases through `Editor::preview()`.
- Help contract: `keymap/tests.rs:116-233` is a 118-entry hand list whose own doc (`:111-114`) says the trie cutover "DELETES this list… dead weight"; the trie exists (`keymap/lookup.rs`) and the list was never deleted. `EX_COMMANDS` (56 entries, `editor/normal.rs:26`) is not iterated by `help.rs:182`.
- Snapshots: AGENTS.md promises "golden cell-grid snapshots"; there is no insta/`.snap`/golden file anywhere; 41 `TestBackend` sites assert cell properties.
- Dead tests: `editor/tests/keybinds_tests.rs:257-296` `dbg_unicode`, `dbg_l_steps`, `dbg_l_from_3` only `eprintln!`.
- Test binary ≠ shipped binary: `crates/strop/Cargo.toml:46` (dev-deps) enables `strop-engine/test-support`, and `editor/namespace.rs:109-115` swaps `Worker::local()` for an in-process pipe worker under that feature. Release features are not leaked (verified with `cargo tree -e features`) — the gap is coverage, not shipping.
- 15 tree-sitter `test/corpus` files under `crates/strop-syntax/grammars/*/test/corpus` have no runner.
- Fix: skip-notice on every gated test; isolate git fixtures; fail (not skip) the differential test when `STROP_REQUIRE_NVIM=1` and set it in the Dockerfile; grow `CASES` to the full `Motion`/`Op`/`Object` lattice and add a loop asserting `preview()` ranges == post-execute diff for every case; delete `DISPATCHED` and derive the help contract from the trie; delete the `dbg_*` tests.

**H5. Build/CI cost and shape.** [V]
- `Dockerfile:29-31` `COPY Cargo.toml Cargo.lock ./ && COPY crates ./crates` then `RUN cargo … test` — no `--mount=type=cache`, no deps-only layer. Any source edit invalidates the entire compile including 750 lockfile packages. The `test` stage also runs the SSH suites at image build, so a flake = full rebuild.
- `ci.yml` runs seven compose stages serially in one job (70–125 min) and a second job that `git worktree add`s commit `a858e889…` (0.36.0), builds it, runs five Python benches and uploads artifacts — this job has failed on every run observed.
- `install-zig.sh` and the Dockerfile `AR15` comment (`Dockerfile:8-16`) still describe a multi-arch amd64+arm64 build that no longer exists.
- Fix: BuildKit cache mounts for `~/.cargo/registry`, `~/.cargo/git`, `target/`; split the `test` stage; delete `native-linux` or reduce it to "run the four native test groups" without the baseline build and benches; move `model`/`core-assurance` to a weekly schedule.

**H6. Error typing doctrine is not followed.** [V]
- 148 `Result<_, String>` signatures in non-test code (`crates/strop/src/update.rs` 10, `strop-containers/src/tar.rs` 10, `strop-picker/src/source/remote.rs` 7, `strop-engine/src/editor/io/save/container.rs` 6, `editor/filesystem/draft/editing.rs` 6, `strop-git/src/memory.rs` 5, …) vs AGENTS.md "libraries return typed errors (thiserror)". Also `editor/resolution/mod.rs:279,289` convert typed grammar errors to `String` at the one resolver seam.
- Fix: thiserror enums at crate boundaries; strings only in the binary's final `eprintln`.

**H7. Platform and distribution text still claims things the project does not ship.** [V] (partly fixed in `c309ef8`)
- Still present after `c309ef8`: `SECURITY.md` is entirely a Windows/SignPath code-signing policy and contains no vulnerability-reporting policy; `README.md:115-119` "Code signing policy… Public Windows executables"; `crates/strop/src/update.rs:33-60` `Channel::{Brew,Cargo,Mise}` (install.sh only ever writes `"tarball"`); `crates/strop-worker-deploy/src/container.rs:151` maps `("linux","arm64") → aarch64-unknown-linux-musl` though the catalog never contains it — arm64 containers are "supported" then fail; `crates/strop-remote/src/bootstrap.rs:130-144` Darwin/aarch64 arms that can only yield `None`; `crates/strop-terminal/build.rs:7-10` and `.github/scripts/install-zig.sh:15-16` non-x86_64 arms; `verification/bench_*.py` Darwin branches; `crates/strop-lsp/src/registry.rs:132,144` "brew install …" hints; `verification/native/strop-gui-windows-tree.txt` (40 KB, 0 refs); `verification/measurements/0058-retained-native-platforms.json` (292 KB).
- `strop-ui-protocol` (2,130 lines) + `crates/strop/src/ui_stdio/` (869) + `crates/strop/tests/ui_stdio/` (1,324) + `specs/UiSession.tla` (620) + `ui-gate.sh`: a protocol server whose only client (`strop-gui`) is gone; `lib.rs:2-3` still says "the future Windows GUI". Decide: keep as an automation/test-driver contract or delete.

### MEDIUM

**M1. Plans are 21.9k lines and several status lines lie.** [V] `plans/0037:3` "planned for later" — `strop-containers` implements it. `0046:3` "planned; stage A landing" — the engine crate exists. `0061:3-6` "implementation started" — crate deleted. `0062` references `.github/scripts/publish-order.py` (missing). 36 backticked paths across 19 plans no longer exist. Two files share number 0083; 0052 does not exist. None of the five over-ceiling files is mentioned in 0028. Fix: a `plans/STATUS.md` with one row per plan, bodies frozen as history.

**M2. Dependency hygiene.** [V] Declared and never referenced: `strop-fs` `serde_json`, `thiserror`; `strop-lsp` `libc`; `strop-worker-deploy` `serde`; `strop-terminal` dev `tempfile`. `strop-fs` `tempfile` is test-only. Duplicate majors: `syn 2+3` (verus), `rustix 0.38+1` (crossterm 0.28), `hashbrown`, `getrandom 0.2+0.4` (ureq 2), `unicode-width`, `webpki-roots` (ureq 2). TLS is rustls-only — doctrine 5 holds.

**M3. Test-helper duplication and oversize test files.** [V] Docker fixture copied five times; two sshd fixtures; `drain_until` copied twice; four harness stacks with no shared test-support crate. Over ceiling: `editor/picker/tests.rs` 1,022, `editor/filesystem/tests.rs` 823. `strop-worker-deploy/tests/common/mod.rs:7` `#![allow(dead_code)]`.

**M4. Layering oddities between crates.** [V] `strop-core → strop-trace`, `strop-picker → strop-syntax, strop-remote, strop-worker-client`, `strop-git → strop-remote`, `strop-terminal → strop-worker-client`, `strop-fs → strop-worker-protocol`. No cycles, but protocol crates sit below almost everything. `strop-engine` has 0 `.await` (doctrine 3 holds); four separate single-thread tokio runtimes are created on demand.

**M5. Hot-path doctrine is honoured; one nit.** [V] The resolver is pure and preview and execution share one revision-stamped seam (`editor/resolution/mod.rs:260-290`; `editor/normal/preview.rs:16`). Nit: `resolve/mod.rs:173-365` allocates a `String` label per resolution on the per-keystroke path; make it `&'static str`.

### LOW

**L1.** Debris: `verification/baseline/0057-linux-logs/` (0 refs), two unreferenced `0058-*terminal-load*.json`, `demos/scripts/feel.txt`, empty `examples/`, `dist/` holding 40+ historical tarballs on disk (ignored, but `release-local.sh` reuses whatever is there).
**L2.** `#[allow(clippy::too_many_arguments)]` ×5, `#[allow(dead_code)] handle` at `strop-worker-deploy/src/container.rs:99`. 140 non-test `unwrap/expect` sites to audit once.
**L3.** `read_system_clipboard` tries `pbpaste` and `launch_browser` tries `open` (`editor/registers.rs:519`, `editor/io/native.rs:239`) — macOS leftovers.
**L4.** `docs/vim-compat.md` says "(soon)" rows exist but the table has none — stale generated doc.

---

## 3. Trim register (reviewer's)

| Item | Path(s) | Why |
|---|---|---|
| Second keymap table | `strop-core/src/commands/bindings.rs` | zero consumers; drifted |
| TLAPS lane | `specs/*Proofs.tla`, `specs/tlaps-gate.sh`, Dockerfile `tlaps`, compose `tlaps`, ci step | no Rust link, 2022 toolchain, red |
| Tautological Verus kernels | `strop-core/src/{mutguard,viewguard,cohortguard,searchguard,id}.rs`, `worker/{session,recovery_policy,deploy_policy,frame_policy}.rs` `verus!` blocks | keep the functions as plain Rust; delete `completion_is_honest` |
| Evidence apparatus | `verification/check.py`, `inventory.json`, `freeze.py`, `baseline/`, `measurements/`, `completion_evidence.py`, ci VF01, Dockerfile `core-assurance` python steps | digest ritual; 16 "Re-pin" commits |
| Mutant registry | `verification/check_mutants.py`, `mutants.json`, `mutants_fixtures/` | 105/111 are TLA toggles |
| TLC models | all but ≤3 (EditorProtocol, SearchLifecycle, WorkerSession) | no recorded real bug |
| UI protocol server with no client | `strop-ui-protocol/`, `strop/src/ui_stdio/`, `strop/tests/ui_stdio/`, `--ui-stdio` | keep-as-automation or delete |
| CI job | `ci.yml` `native-linux` | permanently red, ~60 min |
| Release workflow | `.github/workflows/release.yml` | an `echo` |
| Historical channels | `update.rs` `Channel::{Brew,Cargo,Mise}` | unreachable from `install.sh` |
| arm64/macOS arms | `strop-worker-deploy/src/container.rs:151`, bootstrap, terminal build.rs, install-zig.sh, Dockerfile AR15 comment, bench Darwin branches, lsp brew hints | out of scope per 0079 |
| Windows signing docs | `SECURITY.md`, `README.md:115-119` | replace with a vulnerability-reporting policy |
| Unused deps | listed in M2 | |
| Dead tests/lists | `dbg_*`; `DISPATCHED` | self-described dead weight |
| Dead fn | `strop-core/src/process.rs:87 spawn_leased` [V]; `strop-remote` `push_config/push_known_hosts/is_connected`, `devcontainers active_environment` [S] | |
| Files | `verification/native/strop-gui-windows-tree.txt`, `baseline/0057-linux-logs/`, unreferenced measurements, `demos/scripts/feel.txt`, empty `examples/` | 0 refs |
| Plans | mark retired/implemented; renumber one 0083; freeze 0018–0058 | |
| Test duplication | five docker fixtures, two sshd fixtures | one test-support crate |

---

## 4. Verdict on the "verified foundation / rings" proposal (gripsack `plan/0056`)

**Adopt now:** rings as a dependency-direction and trust discipline; a sans-IO kernel crate with zero deps; the survival rule; the release rule (green CI on the tag, no receipts/inventories/freeze); generated claim lists; the trim register as a plan section.

**Does not transfer:** gripsack's centre of gravity is P1–P4 — crash-safe transactions over a storage model. Strop's product is vim grammar fidelity plus an honest preview; the oracle for that is `nvim --clean` and a property test, not Lean. Strop's crash-relevant state (session persistence, draft recovery, worker Store saves) is a side-dish; a MemFs crash harness is worth more there than a Lean storage model. Remote/container/LSP surfaces will never enter ring 0/1.

**Strop-specific risks:** (1) the 106-field `Editor` stays in ring 4 untouched unless decomposed; (2) `strop-core` cannot be ring 0 as-is — extraction must *move* code; (3) ropey: kernel functions take byte offsets / `&[u8]`, tree-sitter objects stay outside; (4) the `proofs` image is ~20 GB / 11 min cold — only viable if the lane runs on kernel/proofs changes and is cached; (5) opportunity cost: the differential harness gap is cheaper and more product-relevant than any theorem.

**Survival pass:** survives — `editmap` proofs (port to Lean), Loom, `transaction_conformance`, unit/e2e suites. Deleted — TLAPS, nine tautological Verus kernels (functions stay), inventory/check/freeze/mutants/baselines/measurements, ≥17 of 20 TLC models, `native-linux` + benches, `release.yml`. Probation (one release) — EditorProtocol, SearchLifecycle, WorkerSession TLC.

**Proposed ring map:**

| Ring | Strop contents | Checked by |
|---|---|---|
| 0 Kernel (`strop-kernel`, `no_std`+`alloc`, no deps) | edit-map algebra; `Range`/`MotionShape` normalisation; operator-pending parser + count arithmetic; pure resolver scanners over `&[u8]`/line indices; protocol admission decisions (`frame_policy`, `classify_session`, deploy verdicts, `next_generation`) | Lean over the Aeneas translation; one mutant per theorem family |
| 1 Protocol (sans-IO machines) | worker session/stream machine, save/conflict plan, draft-recovery cohort machine, LSP queue admission, bounded event channel policy | property tests with shrinking, Loom, crash harness; ≤3 TLC bug-finders on probation |
| 2 Host | `strop-fs`, `strop-worker`, `strop-remote`, `strop-containers`, `strop-git`, `strop-terminal`, `strop-trace` | small enough to read; fault injection; gated e2e over real sshd/docker |
| 3 Edge | LSP wire, tree-sitter grammars, SFTP/SSH output parsing, `docker inspect` JSON, `devcontainer.json`, release catalog + tarball, terminal input decoding | typed admission, bounds, weekly fuzzing |
| 4 Product | `strop-engine::Editor` (decomposed), keymap + trie, `crates/strop` render/CLI | differential harness vs Neovim, preview==execute property, cell-grid tests, headless e2e |

**First-step pilot (0.53):** create `strop-kernel` by moving `editmap.rs` (minus `verus!`), `next_generation`/`index_for_len` and `frame_policy`; re-point callers; delete `vstd`/`verus_builtin*` and the `verify` stage; add `proofs/` + `proofs` compose service (gripsack pins) and prove the existing Verus postconditions verbatim; one mutant per function must fail. Same release: the trim. Go/no-go: gripsack's three falsifiers plus "the per-PR `proofs` lane runs only when `crates/strop-kernel/**` or `proofs/**` changed". Fallback: kernel crate and trim still land.

**Second step (0.54):** move the operator-pending parser and byte-slice motion scanners into the kernel with the first grammar theorems, and extend `differential.rs` to the full lattice with a `preview()==execute` loop in the same release — theorem and oracle together.

---

## 5. Prioritised remediation (reviewer's)

**Now (0.53):** CI honesty (delete `native-linux`, `release.yml`, TLAPS, VF01; green gate; release checks CI); delete the core keymap table; kernel pilot; evidence apparatus out; platform/doc leftovers and the `strop-ui-protocol` decision; test hygiene quick wins; Dockerfile cache mounts.

**Next (0.54–0.55):** decompose `Editor`; dissolve `strop-core` into `strop-kernel` + `strop-text` + moves with a dependency-direction lint; differential harness to the full lattice + preview==execute; one `strop-test-support` crate; thiserror at library boundaries and `ureq 3`; `plans/STATUS.md`.

**Later:** expand ring 0 one decision per PR with a theorem each; weekly TLC/fuzz/Loom/mutants; run or delete the tree-sitter corpora; revisit the devcontainers surface (~17.5k lines, thinnest default-test coverage).
