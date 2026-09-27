# syntax=docker/dockerfile:1
# Multi-stage musl static build (plan 0002). rust:alpine's host triple IS
# musl, so plain `cargo build` is static for free. build-base (gcc/g++)
# compiles tree-sitter's C (0002 §2.1, proven M0); ripgrep for the grep
# worker's tests; neovim for the differential harness (0006 tier 1) —
# rustls only; an openssl-dragging dependency is a bug.
# a dependency that drags in openssl is a bug (AGENTS.md).
# AR15 pin policy: every external base image is pinned to its multi-arch
# manifest-list digest (FROM image:tag@sha256:...), so amd64 and arm64 both
# resolve and the tag can never drift under the digest. The tag stays only
# as a human-readable label. Internal stages and `scratch` need no pin.
# To bump: for each pinned image run
#   docker buildx imagetools inspect <image>:<new-tag> \
#     --format '{{.Manifest.Digest}}'
# then checklist: (1) digest is the index/list digest, not an arch-specific
#   manifest; (2) the list covers linux/amd64 AND linux/arm64;
# (3) update tag+digest together here; (4) `docker compose build test model
#   verify` resolves; (5) run the full gates before landing.

FROM rust:alpine@sha256:1716b3aa042d735f4566d14dc54e8037de9d69556e2d5dd58131d93a613d173d AS builder
RUN apk add --no-cache build-base git ripgrep neovim less curl ca-certificates xz \
    && rustup component add clippy rustfmt
COPY .github/scripts/install-zig.sh /tmp/install-zig.sh
RUN sh /tmp/install-zig.sh /opt/strop-zig && rm /tmp/install-zig.sh
ENV ZIG=/opt/strop-zig/zig
WORKDIR /app
COPY Cargo.toml Cargo.lock ./
COPY crates ./crates
COPY docs ./docs

FROM builder AS test
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server python3
RUN cargo fmt --check \
    && cargo clippy --locked --workspace --all-targets -- -D warnings \
    && STROP_REQUIRE_SSH_TESTS=1 STROP_JOBS_BUDGET_MS=300000 cargo test --locked

# Real SSH deployment on a host with no Python interpreter: this stage
# adds OpenSSH to the Rust builder but never installs python3. The shared
# worker_ssh parity fixture deploys and launches the actual native worker.
FROM builder AS ssh-pythonfree
RUN apk add --no-cache openssh-client openssh-server openssh-sftp-server
RUN ! command -v python && ! command -v python2 && ! command -v python3 \
    && STROP_REQUIRE_SSH_TESTS=1 cargo test --locked -p strop-editor \
        --test worker_ssh deploy_handshake_read_write_notify_parity_over_real_sshd

FROM builder AS bin
RUN cargo build --locked -p strop-editor

# Only the integration runner needs a Docker client; the shipping binary does not.
FROM bin AS container-test
RUN apk add --no-cache docker-cli openssh-client

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
FROM eclipse-temurin:21-jre@sha256:6cbdfc89c9657478bc5abea638030310f6c0267404e98a5808097bb1925932f1 AS model
# v1.8.0 is a moving prerelease asset; use the stable, checksum-verified release.
# Its published SHA-1 and downloaded SHA-256 were independently checked.
ARG TLA_TOOLS_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
ADD --checksum=sha256:${TLA_TOOLS_SHA256} https://github.com/tlaplus/tlaplus/releases/download/v1.7.4/tla2tools.jar /tla/tla2tools.jar
WORKDIR /work
COPY specs ./specs
RUN sh specs/gate.sh

# Same-source Verus lane (0057 VF18, 0058 WK18): edit geometry,
# session/framing/recovery/effect decisions and deploy admission/cache
# keep policy. The verifier pins its own compiler and solver; the
# shipping musl toolchain stays untouched.
FROM rust:1.98.0-slim@sha256:17d1ba895198f9934c6314ec5346a0d5115372f3243390c3d731e242f35c2f27 AS verify
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
# Both edit geometry and worker admission execute from strop-core.
RUN cargo verus verify -p strop-core

# The TLAPS proof lane (0057 VF17 and 0058 WK17): inductive safety of
# SearchLifecycle, WorkerSession and WorkerDeploy with matched mutants.
# Deliberately outside the shipping build graph; like the verify stage,
# it pins its own toolchain. TLAPS 1.5.0 (tag 202210041448)
# is the last versioned release; the 1.6.0 pre-release is a moving asset
# (cf. the tla2tools note above), so the dated, checksum-verified
# installer is pinned. x86_64 only, like the Verus zip.
FROM debian:bookworm-slim@sha256:88200866dfff7ea7f5cbcb6ec7c8a701889efe6fe859fe64d6990e4b07ea4171 AS tlaps
ARG TLAPS_SHA256=ebb7a3f271bdb564f74cb0a2767ef7b9ff7045621a9be7c50d363a03c2e6f08a
ADD --checksum=sha256:${TLAPS_SHA256} https://github.com/tlaplus/tlapm/releases/download/202210041448/tlaps-1.5.0-x86_64-linux-gnu-inst.bin /tmp/tlaps-inst.bin
# libstdc++6 for the bundled z3/ls4 backends; make+gcc because the
# installer compiles Isabelle/Pure; procps because tlapm/Isabelle call
# ps (its absence fails the installer self-test); the installer is an
# ELF self-extractor needing only libc.
RUN apt-get update && apt-get install -y --no-install-recommends \
        libstdc++6 make gcc procps \
    && rm -rf /var/lib/apt/lists/* \
    && chmod +x /tmp/tlaps-inst.bin \
    && /tmp/tlaps-inst.bin -d /opt/tlaps \
    && rm /tmp/tlaps-inst.bin
ENV PATH=/opt/tlaps/bin:$PATH
WORKDIR /work
COPY specs ./specs
RUN sh specs/tlaps-gate.sh

# The core-assurance lane (0057 VF19/VF20; 0058 WK16–WK20):
# exact candidate inventory/source/evidence bindings, native
# Store/recovery/stream journeys, instrumented Loom campaigns over
# shipped synchronization seams, mutant calibration and picker storm.
# FROM test so the plain cargo fingerprints are warm; the cfg-
# instrumented runs (strop_loom, strop_mutant) rebuild what their cfg
# touches and ship nothing — the cfgs exist only in this image.
FROM test AS core-assurance
# Inventory changes re-run only assurance, never rebuild the locked
# workspace tests; no Rust target embeds verification/ as a runtime path.
COPY verification ./verification
COPY specs ./specs
COPY .github ./.github
COPY tests ./tests
COPY install.sh ./install.sh
COPY Dockerfile ./Dockerfile
RUN python3 verification/check.py \
    && python3 verification/check_mutants.py \
    && python3 verification/check_mutants.py --self-test \
    && cargo test --locked -p strop-fs tests::store:: \
    && cargo test --locked -p strop-worker-client --test loopback store_save_round_trip_with_conflict_and_verify \
    && cargo test --locked -p strop-worker-client --test loopback pty_stalled_consumer_resumes_without_losing_vt_bytes \
    && cargo test --locked -p strop-worker-deploy concurrent_private_cache_creation_reuses_only_the_other_installers_owned_directory \
    && RUSTFLAGS="--cfg strop_loom" cargo test --locked -p strop-engine -p strop-lsp -p strop-worker loom \
    && python3 verification/check_mutants.py --execute \
    && cargo test --locked -p strop-editor --test ui_stdio storm
