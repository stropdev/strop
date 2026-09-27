# 0002 — Build & Release Story

> Docker + docker compose, musl-static Linux binaries, native Apple Silicon builds,
> three tarballs, crates.io + homebrew + site dispatch.
> The new wrinkle is native C/C++ code: tree-sitter grammars and libgit2.

Status: historical build/release design. Current workflow sources own implemented
details; [0062](0062-distribution-and-wsl-onboarding.md) extends the distribution
contract for the native Windows GUI and selected-WSL backend without weakening
the TUI/static gates. Historical illustrative snippets are not replacement gates.

[0056](0056-architecture-prerequisites.md) AR11–AR15 close the current shared TUI
installation/ownership, release-catalog, build/evidence foundations first. 0062
extends them for Windows/WSL packaging instead of repairing them inside GUI delivery.
The separate [0057](0057-core-verification-and-assurance.md) release immediately
qualifies the stable core and adds required generalized proof/core-assurance gates.
[0058](0058-unified-native-worker.md) then adds shared native worker mode and verified
local/SSH/container artifact deployment, reusing the release matrix/catalog and
requalifying its changes. Completion/debugger/GUI follow. Publication binds the
actual uploaded/executed worker to the exact source/target/assurance candidate.

---

## 1. Supported release matrix

| Target | Runner | Method |
|---|---|---|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `docker compose run release` (rust:alpine) |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | same compose build, native — rust:alpine is multi-arch, no cross config |
| `aarch64-apple-darwin` | `macos-14` | native cargo |

Starting with 0.36.0, the user explicitly removed Intel macOS support.
No `x86_64-apple-darwin` binary, native CI lane or Homebrew Intel download
is published. Install/update and remote-worker target discovery refuse
Intel macOS rather than selecting an ARM artifact. Historical releases
retain their original artifacts and evidence. See [0028](0028-roadmap-and-review.md)
for the scope decision and re-entry condition.

The TUI's Windows story remains WSL. Native Windows GUI distribution is now
specified by [0061](0061-gui-windows-and-wsl.md) and
[0062](0062-distribution-and-wsl-onboarding.md); workspace execution stays in WSL.
One musl-static artifact per supported Linux architecture covers glibc and musl distributions.

## 2. Why musl-static survives tree-sitter (the decision this plan records)

The risk that motivated this plan: tree-sitter grammars are C (some scanners are C++),
compiled per-grammar by the `cc` crate. Verdict: **musl-static is achievable, no zig
required.**

Evidence: difftastic statically links ~30 tree-sitter grammars (including C++ scanners
such as tree-sitter-cpp) and ships a fully static `x86_64-unknown-linux-musl` binary
(verified 2026-09-01: `static-pie linked`, `ldd` → "statically linked"). On `rust:alpine`
the host triple *is* musl, so `cargo build` is static for free once a C/C++ toolchain
exists in the image.

Requirements this imposes:

1. **Dockerfile installs `build-base` (gcc, g++, musl-dev), not just `musl-dev git`.**
   C++ scanners statically link `libstdc++.a`; that works under musl. Strop does not
   `dlopen` grammars and does not throw C++ exceptions across the Rust boundary, so the
   known musl + static-libstdc++ edge cases do not apply.
2. **Grammars are statically linked into the binary** (build.rs via `cc` — the
   neovim/difftastic model). Helix's runtime model (helix-loader `dlopen`s grammar `.so`
   files at runtime) is rejected for strop: it is incompatible with a single static
   binary and forces a runtime-directory install story. Consequence: the shipped grammar
   set is curated at build time; adding a grammar is a rebuild, not a download. Grammar
   loading is out of scope for the plugin-runtime non-goal anyway.
3. **`git2` builds with `default-features = false`.** libgit2's vendored C build works
   under musl, but git2's default features drag in `openssl-sys` for https/ssh. Plan 0001
   already splits git work: libgit2 for local hot paths (gutter, hunks — no network),
   shell `git` for log/blame. Network operations are therefore out of libgit2 entirely,
   openssl stays out of the tree (consistent with the rustls-only rule from gripsack),
   and the musl build stays clean. Future network features must preserve that
   rule through shell Git or an approved rustls path. Vendored OpenSSL is not an
   approved escape hatch under the current repository contract.

## 3. glibc fallback (accepted, not built)

If a future dependency refuses to static-link under musl, the fallback is
**cargo-zigbuild**: `zig cc`/`zig c++` as compiler+linker with glibc-version pinning —
`cargo zigbuild --target x86_64-unknown-linux-gnu.2.17` produces a manylinux-class
binary without docker. It handles the C++ side via libc++ (statically linked). It would
replace only the Linux docker jobs; the macOS jobs and the release pipeline are
unchanged. Not building this now: musl-static is strictly better for the
ssh-into-a-server test, and a second Linux build system is dead weight until needed.

## 4. Dockerfile + compose layout

Static-verify lesson, learned on first contact (2026-09-01): a musl static-pie binary
still prints a `/lib/ld-musl-…` line in `ldd`, so "grep for 'not a dynamic executable'"
fails on a *good* binary, and `! ldd | grep "=>"` passes vacuously on a *bad* one (a
dynamically-linked musl binary has no `=>` lines). The honest gate is
`! readelf -d <bin> | grep NEEDED` + `file <bin>` says "static-pie linked" (x86_64)
or "statically linked" (aarch64 musl is non-PIE — first v0.1.0 run caught this). The
rootle/gripsack release workflows use the vacuous form — worth re-gating there too.

```dockerfile
FROM rust:alpine AS builder
RUN apk add --no-cache build-base git \
    && rustup component add clippy rustfmt
# ... COPY workspace

FROM builder AS test
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo test --locked

FROM builder AS release
RUN cargo build --release --locked -p strop \
    && strip target/release/strop \
    && ldd target/release/strop 2>&1 \
       | grep -q "Not a valid dynamic program\|not a dynamic executable"
```

Compose services mirror gripsack: `test` (gate at build time — fmt, clippy, cargo test,
**including a headless parse of a file whose grammar has a C++ scanner**, so a broken
static-libstdc++ link fails CI on the PR, not a user's first `cpp` file), `build`,
`release` (tarball + sha256 → `./dist/`, `VERSION` required, tarball layout is the
install.sh contract: `strop-<VERSION>-<TARGET>.tar.gz` containing `<dir>/strop`), and
later `e2e`.

The compose release command takes `TARGET` exactly like gripsack's; aarch64 just runs
the same build on the arm runner.

## 5. Release workflow (rootle/gripsack shape)

Trigger: `push` on tags `v*`. `concurrency.group: release`, `cancel-in-progress: false`
(two tags raced the homebrew push once — the gripsack lesson).

- **build** job: matrix above; docker path on Linux, native cargo on macOS.
- **Verify steps** (per platform, extended vs. rootle):
  - checksum verifies, tarball extracts, `file` + `ldd`/`otool -L` gates
    (Linux fully static; macOS system libs only).
  - `strop --version` runs.
  - **Headless grammar smoke** against the shipped tarball: parse a C++-scanner file
    (`strop --headless parse <file>` spelling decided when the binary exists; the gate
    is contractual, the spelling is not). The same check runs per-PR in the docker
    `test` stage (§4) — release verify is the belt, CI is the suspenders.
- **release** job: assemble all three tarballs, fail unless exactly 3; crates.io publish
  with the tag/version guard (`cargo metadata` vs `$GITHUB_REF_NAME`; strop publishes as
  `strop-editor` per plan 0001 — the guard reads that package); homebrew formula (source,
  from the crate) + cask (Apple Silicon binary and sha256) bump via
  `HOMEBREW_TAP_TOKEN`; `gh release create`; best-effort site redeploy dispatch
  (`continue-on-error`, `SITE_REPO_TOKEN`) once strop.dev has a site.

Publication ordering includes **versioned workspace dev-dependencies**:
Cargo resolves them while generating the packaged lockfile even though the
library verification build does not compile its tests. Path-only dev-dependencies
are stripped from the published manifest. `.github/scripts/publish-order.py`
orders these prerequisites; `sh tests/publish-order.sh` covers the ordering,
private prerequisites and cycles.

The 0.36.0 publication exposed this distinction: `strop-worker-client` was
attempted before its versioned `strop-worker` test dependency existed. The
worker subsequently published, allowing the original publication job to resume
without rebuilding or moving `v0.36.0` from `a858e8891135`. The permanent
ordering correction is a post-tag tooling change for subsequent releases;
already published crate versions and the qualified tag remain immutable.

## 6. Deferred

- **Native Windows workspace/process targets** remain later work. The Windows GUI
  frontend + WSL backend and its installer now have 0061/0062 contracts.
- **Grammar smoke on macOS**: same headless parse, cheap to add; the musl static link is
  the fragile one, so Linux gets the gate first.
- **cargo-zigbuild / pinned-glibc artifacts**: §3.
- **dist/ crates.io binary publishing (cargo-binstall metadata)**: nice-to-have for
  faster `cargo install`; not part of v1.


## 7. Upstream watch (decided 2026-09-02)

All curated highlight queries are Helix-vendored (`crates/strop-syntax/queries/`). They
must not silently rot: a weekly `upstream-watch.yml` (the gripsack pattern — Monday cron,
`issues: write`, idempotent-by-title) diffs our vendored queries against
`helix-editor/helix` master and the grammar crates' latest releases (rust, c, cpp,
python, go, javascript, typescript, json, bash), opening one issue per divergence.
Queries are data (0001 §5.11): updating a vendored file is a content change, not a
release — the watch keeps the debt visible instead of fossilized.