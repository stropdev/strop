---
rfc: "2026-10-24-correctness-sweep-and-quiet-ui"
title: "Quiet diagnostics and split-view correctness"
status: "accepted"
implementation: "complete"
authors: ["tknawara"]
created: "2026-10-24"
updated: "2026-11-20"
issue: "https://github.com/stropdev/strop/issues/166"
milestone: "v0.45.0"
supersedes: []
superseded_by: []
blocked_on: []
legacy_id: "0071"
---

# RFC: Quiet diagnostics and split-view correctness

v0.45.0 removed diagnostic range underlines while retaining severity cues, reclamped inactive splits after shared text shrank and vendored the terminal build’s Zig inputs. The release also distinguished partial from refused UI action batches; 0086 later deleted that protocol with its frontend, not the TUI fixes.

[Tracking issue #166](https://github.com/stropdev/strop/issues/166) · [Published v0.45.0](https://github.com/stropdev/strop/releases/tag/v0.45.0) · [Original record](https://github.com/stropdev/strop/blob/eec2a0e7fe62bb75461d6d779c9dea514a80a4f9/plans/0071-correctness-sweep-and-quiet-ui.md).

---

**Historical design.** The original text follows; its status lines and future work are not the current disposition.

Status: **implemented** (2026-10-24).

## 1. Scope

A user-driven full editor/site sweep found correctness defects alongside the visual
complaints. This plan records only the decisions that change a published contract;
individual vim, picker, configuration, and delivery fixes keep their existing numbered
contracts and regression evidence.

## 2. Diagnostics stay useful without painting source ranges

Diagnostics use the severity gutter, the cursor-line end-of-line note, modeline counts,
and the diagnostics picker. They do not underline source ranges. The former straight
amber underlines were noisy, were not the undercurl the early UI text imagined, and made
unchanged source look damaged. Insert-mode stashing and revision/owner freshness checks
remain unchanged: removing a decoration must not make diagnostic delivery stale.

## 3. Every split converges after a shared-document shrink

Frame preparation clamps every non-terminal pane's `view_top` against the current
content length and pane height. Only the active pane's caret drives scrolling, but an
inactive pane must not remain beyond EOF until focused. UI snapshots ship the same
clamped `window_top` used to construct their `lines`.

## 4. UI action batches have three honest outcomes

`Applied` means the entire admitted batch completed. `Refused` keeps its existing
meaning: no action in the batch applied. An engine error after a nonempty applied prefix
is **Partial**: the prefix is published before the acknowledgement, the applied counter
advances, the remaining actions are not run, and the acknowledgement carries the typed
engine failure.

Protocol version 2 adds:

```text
Partial { applied, generation, completed, refusal }
```

`completed` is the number of actions in this batch that completed before the failure.
A client must consume the published generation and must not retry the original whole
batch as though none of it ran. The in-repo driver surfaces Partial as an error carrying
the generation/completed count rather than flattening it into success or refusal.

This is deliberately not `Applied`: that would claim the skipped suffix ran. It is also
not `Refused`: that would invite a client to duplicate the published prefix.

The audit found no consumer-reachable live `ui_stdio` action sequence for the
defensive mid-batch engine-error arm. Coverage therefore exercises v2 wire
serialization and the real driver’s publication/error handling against a
synthetic wire peer; it does not add a fake production failure mode.

## 5. Local source builds pin every required Zig package

The terminal's three required transitive Zig packages are vendored as source archives
with revisions and SHA-256 recorded in `crates/strop-terminal/vendor/UPSTREAM`.
`build.rs` runs `zig fetch` on those local files before compiling Ghostty VT, retaining
Zig's package-hash verification while avoiding GitHub/Codeberg HTTP parser failures.
This is a build-input change, not a terminal runtime change.

Headless scripts may run `settle` after quit to drain owned cleanup. Any other
post-quit directive is an error; cleanup no longer provides a way to skip assertions.

Assurance evidence was rebound to the new regressions. `worker-deployment` alone used
`check.py --stamp --force worker-deployment`: its `explain.rs` source moved because of
session-theme reporting, a reviewed no-op for deployment behavior and evidence.

## 6. Acceptance

- Repeated headless frames show inactive splits re-clamped after deletion; no source
  diagnostic underlines remain while gutter/EOL signals survive.
- UI protocol serialization and driver handling cover Partial; ordinary Applied and
  zero-application Refused behavior is unchanged. If the defensive mid-batch I/O arm
  cannot be reached through a consumer action sequence, record that limit rather than
  adding a fake production failure mode.
- Focused package suites, the real UI/headless surfaces, and the container quality gate
  pass. Website changes are verified separately in Chromium and never deployed by this
  editor change.

Verification: clean `docker compose run --build --rm test` passed formatting,
strict Clippy and the locked workspace/SSH suite. `docker compose run --build --rm
core-assurance` passed inventory, native effects, Loom and calibrated mutants.
The real headless binary exercised split shrink/repaint, generated help and live
theme selection. Website generator/installer checks and Chromium desktop/mobile
reviews passed in the separate site repository before its source was published.

Release preparation: 0.45.0 workspace/inter-crate versions and `Cargo.lock`
were synchronized. `local-fs-authority`, `worker-serve` and `worker-deployment`
used reviewed forced re-stamps only for their Cargo manifest version changes;
their runtime evidence is unchanged. `verification/check.py --release` passes.
