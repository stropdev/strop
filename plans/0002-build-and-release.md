# 0002 — Build & Release Story

> Carries the rootle/gripsack build matrix over 1:1: docker + docker compose, musl-static
> Linux binaries, native macOS builds, four tarballs, crates.io + homebrew + site dispatch.
> The new wrinkle is native C/C++ code: tree-sitter grammars and libgit2.

Status: accepted-in-principle. No code yet; this is the contract the first Dockerfile and
release workflow are written against.

---

## 1. Matrix (same as rootle/gripsack)

| Target | Runner | Method |
|---|---|---|
| `x86_64-unknown-linux-musl` | `ubuntu-latest` | `docker compose run release` (rust:alpine) |
| `aarch64-unknown-linux-musl` | `ubuntu-24.04-arm` | same compose build, native — rust:alpine is multi-arch, no cross config |
| `x86_64-apple-darwin` | `macos-14` | native cargo, stock Xcode cross (no macos-13 queue) |
| `aarch64-apple-darwin` | `macos-14` | native cargo |

Windows is out by design (WSL is the story there) — same call as rootle/gripsack.
A static musl binary is the whole Linux story: one artifact, every distro, every ssh box.

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
   and the musl build stays clean. If a future feature needs libgit2 https, the escape
   hatch is `openssl-sys/vendored` (compiles OpenSSL from source under musl; real
   build-time cost, real binary growth) — prefer routing through shell `git` instead.

## 3. glibc fallback (accepted, not built)

If a future dependency refuses to static-link under musl, the fallback is
**cargo-zigbuild**: `zig cc`/`zig c++` as compiler+linker with glibc-version pinning —
`cargo zigbuild --target x86_64-unknown-linux-gnu.2.17` produces a manylinux-class
binary without docker. It handles the C++ side via libc++ (statically linked). It would
replace only the Linux docker jobs; the macOS jobs and the release pipeline are
unchanged. Not building this now: musl-static is strictly better for the
ssh-into-a-server test, and a second Linux build system is dead weight until needed.

## 4. Dockerfile + compose layout

Mirror the gripsack pattern (multi-stage, gates run at image-build time):

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
- **release** job: assemble all four tarballs, fail unless exactly 4; crates.io publish
  with the tag/version guard (`cargo metadata` vs `$GITHUB_REF_NAME`; strop publishes as
  `strop-editor` per plan 0001 — the guard reads that package); homebrew formula (source,
  from the crate) + cask (prebuilt darwin binaries, per-arch sha256) bump via
  `HOMEBREW_TAP_TOKEN`; `gh release create`; best-effort site redeploy dispatch
  (`continue-on-error`, `SITE_REPO_TOKEN`) once strop.dev has a site.

## 6. Deferred

- **Windows targets**: revisit only if WSL coverage proves insufficient.
- **Grammar smoke on macOS**: same headless parse, cheap to add; the musl static link is
  the fragile one, so Linux gets the gate first.
- **cargo-zigbuild / pinned-glibc artifacts**: §3.
- **dist/ crates.io binary publishing (cargo-binstall metadata)**: nice-to-have for
  faster `cargo install`; not part of v1.
