# syntax=docker/dockerfile:1
# Static musl build and the gates (plans/0002, plans/0086 §8). rust:alpine's
# host triple IS musl, so plain `cargo build` is static. build-base compiles
# tree-sitter's C; ripgrep serves the grep worker's tests; neovim is the
# differential harness's oracle (plans/0006). rustls only: a dependency that
# drags in openssl is a bug (AGENTS.md).
#
# Base images are pinned by digest (tag kept as a label). To bump, take the
# new digest from `docker buildx imagetools inspect <image>:<tag>`, update tag
# and digest together, and run every compose gate.
#
# BuildKit cache mounts keep the cargo registry and the target directory
# across builds, so a source edit recompiles the workspace, not the 700+
# locked dependencies. Stages that need build products in the image (bin,
# release) copy them out of the cached target directory explicitly.

FROM rust:alpine@sha256:1716b3aa042d735f4566d14dc54e8037de9d69556e2d5dd58131d93a613d173d AS builder
RUN apk add --no-cache build-base git ripgrep neovim less curl ca-certificates xz \
    && rustup component add clippy rustfmt
COPY .github/scripts/install-zig.sh /tmp/install-zig.sh
RUN sh /tmp/install-zig.sh /opt/strop-zig && rm /tmp/install-zig.sh
ENV ZIG=/opt/strop-zig/zig CARGO_TARGET_DIR=/cache/target
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY docs ./docs

# The gate: fmt, clippy -D warnings, the workspace tests (including the real
# sshd suites) and the Loom campaign over the shipped synchronization seams.
FROM builder AS test
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server python3
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/cache/target,id=strop-test,sharing=locked \
    cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && STROP_REQUIRE_SSH_TESTS=1 STROP_REQUIRE_NVIM=1 STROP_JOBS_BUDGET_MS=300000 cargo test --locked \
    && CARGO_TARGET_DIR=/cache/target/loom RUSTFLAGS="--cfg strop_loom" \
        cargo test --locked -p strop-engine -p strop-lsp loom

# Remote editing over plain SSH on a host with no Python interpreter:
# OpenSSH, rg and git on the Rust builder, never python3. The exec journeys
# run against a real listening sshd; the editor journeys drive the real
# binary — open, edit, save, the changed-on-disk refusal, save-as, search,
# Git and a terminal with Ctrl-C — over OpenSSH.
FROM builder AS ssh-pythonfree
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/cache/target,id=strop-ssh,sharing=locked \
    ! command -v python && ! command -v python2 && ! command -v python3 \
    && STROP_REQUIRE_SSH_TESTS=1 cargo test --locked --no-fail-fast -p strop-editor \
        --test ssh_exec --test remote_ssh -- --test-threads=1

# Remote latency measurement (plan 0090 §7), not a gate: open, save,
# search, Git and terminal echo over plain ssh exec + SFTP on one
# multiplexed master, over a loopback sshd, in release. DELAY_MS adds
# netem delay each way on loopback (needs NET_ADMIN).
FROM builder AS ssh-bench
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server iproute2-tc
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/cache/target,id=strop-ssh-bench,sharing=locked \
    cargo test --release --locked -p strop-editor --test ssh_exec --no-run --message-format=json \
        | sed -n 's/.*"executable":"\([^"]*ssh_exec[^"]*\)".*/\1/p' > /tmp/bench-exe \
    && install -D -m755 "$(cat /tmp/bench-exe)" /app/bench/ssh_exec
ENV STROP_REQUIRE_SSH_TESTS=1 STROP_SSH_EXEC_BENCH=1
CMD ["sh", "-c", "if [ -n \"$DELAY_MS\" ]; then tc qdisc add dev lo root netem delay \"${DELAY_MS}ms\"; fi; exec /app/bench/ssh_exec --exact bench::remote_latency_over_real_sshd --nocapture --test-threads=1"]

# Stripped static release binary. The gate: no NEEDED shared libraries and a
# static ELF (readelf NEEDED is the truth; `ldd` lies on static-pie musl).
FROM builder AS release
RUN --mount=type=cache,target=/usr/local/cargo/registry,sharing=locked \
    --mount=type=cache,target=/cache/target,id=strop-release,sharing=locked \
    cargo build --release --locked -p strop-editor \
    && install -D -m755 /cache/target/release/strop /app/target/release/strop \
    && strip /app/target/release/strop \
    && ! readelf -d /app/target/release/strop | grep -q NEEDED \
    && file /app/target/release/strop | grep -qE "static-pie linked|statically linked" \
    && echo "static: ok"

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/strop /strop
ENTRYPOINT ["/strop"]

# TLC over the probation models (plans/0086 §7): EditorProtocol,
# SearchLifecycle, each with its kept mutants.
FROM eclipse-temurin:21-jre@sha256:6cbdfc89c9657478bc5abea638030310f6c0267404e98a5808097bb1925932f1 AS model
ARG TLA_TOOLS_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
RUN sh specs/gate.sh
