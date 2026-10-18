# 0002 — Build & Release Story

> Docker + docker compose, one musl-static Linux x86_64 tarball,
> crates.io + Linux-only Homebrew formula + site dispatch.
> Native C/C++ dependencies include tree-sitter grammars and libgit2.

Status: historical build/release design with an active Linux/WSL x86_64-only
policy amendment. Current workflow sources own implemented details. Native
Windows distribution in [0061](0061-gui-windows-and-wsl.md) and
[0062](0062-distribution-and-wsl-onboarding.md) is deferred pending an explicit
expansion decision. Historical illustrative snippets are not replacement gates.

[0056](0056-architecture-prerequisites.md) AR11–AR15 close the current shared TUI
installation/ownership, release-catalog, build/evidence foundations first. 0062
retains future Windows/WSL packaging requirements without activating that work.
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

The active support boundary is **Linux x86_64 only**, including a Linux x86_64
distribution inside WSL. GNU/Linux native qualification and the static musl
artifact gates both remain required. macOS, ARM/aarch64 and native Windows
executables/installers are unsupported; install/update and worker discovery
must refuse them rather than aliasing a supported artifact.

Historical releases retain their original artifacts and evidence. Expansion
requires an explicit support decision, owned native qualification capacity,
and fresh worker/SSH/PTY/editor/LSP/retirement and install/artifact evidence
before support is readvertised; see [0028](0028-roadmap-and-review.md).
Linux editor, worker and protocol obligations are not weakened.

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
replace only the Linux docker jobs and leave the release pipeline unchanged.
Not building this now: musl-static is strictly better for the ssh-into-a-server
test, and a second Linux build system is dead weight until needed.

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

The compose release command takes `TARGET` exactly like gripsack's; the only
active release target is `x86_64-unknown-linux-musl`.

## 5. Release workflow (rootle/gripsack shape)

Trigger: `push` on tags `v*`. `concurrency.group: release`, `cancel-in-progress: false`
(two tags raced the homebrew push once — the gripsack lesson).

- **build** job: the Linux x86_64 matrix above, using Docker.
- **Verify steps** (for the supported target):
  - checksum verifies, tarball extracts, and `file` + `readelf` confirm a fully
    static Linux executable.
  - `strop --version` runs.
  - **Headless grammar smoke** against the shipped tarball: parse a C++-scanner file
    (`strop --headless parse <file>` spelling decided when the binary exists; the gate
    is contractual, the spelling is not). The same check runs per-PR in the docker
    `test` stage (§4) — release verify is the belt, CI is the suspenders.
- **release** job: assemble exactly one Linux x86_64 tarball; crates.io publish
  with the tag/version guard (`cargo metadata` vs `$GITHUB_REF_NAME`; strop publishes as
  `strop-editor` per plan 0001 — the guard reads that package); Linux x86_64-only
  Homebrew source formula bump via `HOMEBREW_TAP_TOKEN`, with no macOS cask;
  `gh release create`; site redeploy dispatch using `SITE_REPO_TOKEN`.
  Catalog, artifact and promotion controls remain required for the supported target.

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

- **macOS, ARM/aarch64 and native Windows** require an explicit support expansion
  decision and fresh native qualification. This includes the Windows GUI/WSL
  frontend and installer designs retained in 0061/0062; they are not active
  delivery requirements. Historical evidence is not a qualification waiver.
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