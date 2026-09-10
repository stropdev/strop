# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan 0002). rust:alpine's host triple IS
# musl, so plain `cargo build` is static for free. build-base (gcc/g++)
# compiles tree-sitter's C (0002 §2.1, proven M0); ripgrep for the grep
# worker's tests; neovim for the differential harness (0006 tier 1) —
# rustls only; an openssl-dragging dependency is a bug.
# a dependency that drags in openssl is a bug (AGENTS.md).

FROM rust:alpine AS builder
RUN apk add --no-cache build-base git ripgrep neovim \
    && rustup component add clippy rustfmt
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY docs ./docs

FROM builder AS test
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server python3
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && STROP_REQUIRE_SSH_TESTS=1 cargo test --locked

FROM builder AS bin
RUN cargo build --locked -p strop-editor

# Stripped static release binary (0002 §4). The gate: no NEEDED shared
# libraries. (`ldd | grep "not a dynamic"` is wrong on current
# rust:alpine — a static-pie musl binary still prints the ld-musl line;
# and `! ldd | grep "=>"` passes vacuously. readelf NEEDED is the truth.)
FROM test AS release
RUN cargo build --release --locked -p strop-editor \
    && strip target/release/strop \
    && ! readelf -d target/release/strop | grep -q NEEDED \
    && file target/release/strop | grep -qE "static-pie linked|statically linked" \
    && echo "static: ok"

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/strop /strop
ENTRYPOINT ["/strop"]

# The editor protocol model check (0024/R12): TLC over
# specs/EditorProtocol.tla plus the kept-mutant kill check — the gate
# script checks both freshness invariants in the deliberate mutant.
FROM eclipse-temurin:21-jre AS model
# v1.8.0 is a moving prerelease asset; use the stable, checksum-verified release.
# Its published SHA-1 and downloaded SHA-256 were independently checked.
ARG TLA_TOOLS_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
RUN sh specs/gate.sh

# The Verus pilot (0045): proves the edit-geometry kernel in strop-core.
# Deliberately NOT rust:alpine — the verifier pins its own compiler
# (1.98.0) and solver (z3 4.16.0); the shipping TUI toolchain is
# untouched. Tooling is checksum-pinned like the model stage.
FROM rust:1.98.0-slim AS verify
ARG VERUS_SHA256=13d01e134c0620c3b29770874707d16c33b3d227c843a489c8ceb744d43c0a16
ARG Z3_SHA256=7288c49a5bd6dbafd7b0b0d1f65956b91672da24b08f09242919af159be3418e
RUN apt-get update && apt-get install -y --no-install-recommends curl unzip ca-certificates \
    && rm -rf /var/lib/apt/lists/*
ADD --checksum=sha256:${VERUS_SHA256} https://github.com/verus-lang/verus/releases/download/release/0.2026.09.06.8dea4a2/verus-0.2026.09.06.8dea4a2-x86-linux.zip /opt/verus.zip
ADD --checksum=sha256:${Z3_SHA256} https://github.com/Z3Prover/z3/releases/download/z3-4.16.0/z3-4.16.0-x64-glibc-2.39.zip /opt/z3.zip
RUN cd /opt && unzip -q verus.zip && unzip -q z3.zip -d z3
ENV PATH=/opt/verus-x86-linux:$PATH
ENV VERUS_Z3_PATH=/opt/z3/z3-4.16.0-x64-glibc-2.39/bin/z3
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
# Only strop-core carries verus! blocks today; verify it alone.
RUN cargo verus verify -p strop-core
