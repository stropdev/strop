# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan 0002). rust:alpine's host triple IS
# musl, so plain `cargo build` is static for free. build-base (gcc/g++)
# compiles tree-sitter's C (0002 §2.1, proven M0); ripgrep for the grep
# worker's tests — rustls only; an openssl-dragging dependency is a bug.
# a dependency that drags in openssl is a bug (AGENTS.md).

FROM rust:alpine AS builder
RUN apk add --no-cache build-base git ripgrep \
    && rustup component add clippy rustfmt
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates

FROM builder AS test
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && cargo test --locked

FROM builder AS bin
RUN cargo build --locked -p strop-editor

# Stripped static release binary (0002 §4). The gate: no NEEDED shared
# libraries. (`ldd | grep "not a dynamic"` is wrong on current
# rust:alpine — a static-pie musl binary still prints the ld-musl line;
# and `! ldd | grep "=>"` passes vacuously. readelf NEEDED is the truth.)
FROM builder AS release
RUN cargo build --release --locked -p strop-editor \
    && strip target/release/strop \
    && ! readelf -d target/release/strop | grep -q NEEDED \
    && file target/release/strop | grep -q "static-pie linked" \
    && echo "static: ok"

# Shipping image: just the binary.
FROM scratch AS ship
COPY --from=release /app/target/release/strop /strop
ENTRYPOINT ["/strop"]
