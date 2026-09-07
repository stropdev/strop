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
RUN apk add --no-cache openssh-client openssh-server
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
# script proves the protocol AND requires the mutant to die by exactly
# the NoMisapply invariant. The image existing IS the gate.
FROM eclipse-temurin:21-jre AS model
ARG TLA_TOOLS_SHA256=b658b4e504fdf0b721caf7066320f6b6fe5805f4dd2f717d0e47baba4097205e
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v1.8.0/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
RUN sh specs/gate.sh
