#!/usr/bin/env python3
"""VF20/WK20 candidate freeze: record the exact committed worker release
outside the source tree in dist/worker-candidate.json.

Recorded, all from the live tree — never from memory:

  * git commit, tree id, commit date, and any dirty paths (a frozen
    candidate must be exact; dirt is recorded, and refused without
    --allow-dirty);
  * Cargo.lock digest (locked-dependency identity);
  * inventory digest (verification/inventory.json — the boundary/claim
    set this candidate is qualified against);
  * worker implementation digests: codec, native effects, transports,
    deployment and editor reconciliation from the shipped Rust tree;
    historical Python helper digests stay immutable under
    verification/baseline/0057-helper-digests.json;
  * the immutable original pre-worker archive (dirty 75-claim snapshot)
    and the separate clean 74-claim Linux-only candidate with raw gate
    logs; neither is automatically a macOS/arm64 release qualification;
  * Dockerfile stage pins: every external base image as image:tag@sha256
  * Zig bootstrap pin (version + per-platform digests) from
    .github/scripts/install-zig.sh;
  * the Verus crate pins from strop-core (including worker admission).
  * the exact native worker handshake/size/RSS benchmark harness bytes
    and the separate warm control-frame roundtrip harness bytes;

Boring tools only: python3 stdlib + git + sha256 of files.

Usage:
  python3 verification/freeze.py [--allow-dirty] [--out PATH]
  python3 verification/freeze.py --check   # current committed tree?
"""

from __future__ import annotations

import hashlib
import json
import math
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "dist" / "worker-candidate.json"

# The sources whose correspondence changes under the worker cutover.
# Every entry ships in the candidate or governs deployment/recovery;
# the pre-worker helper list is historical evidence, not a live input.
WORKER_SOURCES = [
    "crates/strop-core/src/worker.rs",
    "crates/strop-core/src/worker/cache_record.rs",
    "crates/strop-core/src/worker/session.rs",
    "crates/strop-core/src/worker/frame_policy.rs",
    "crates/strop-core/src/worker/recovery_policy.rs",
    "crates/strop-core/src/worker/deploy_policy.rs",
    "crates/strop-worker-protocol/src/frame.rs",
    "crates/strop-worker-protocol/src/codec.rs",
    "crates/strop-worker-protocol/src/request.rs",
    "crates/strop-worker-protocol/src/message.rs",
    "crates/strop-worker-protocol/src/guard.rs",
    "crates/strop-worker/Cargo.toml",
    "crates/strop-worker/src/serve.rs",
    "crates/strop-worker/src/serve/cache_lease.rs",
    "crates/strop-worker/src/serve/cache_gc.rs",
    "crates/strop-worker/src/serve/handshake.rs",
    "crates/strop-worker/src/serve/fs.rs",
    "crates/strop-worker/src/serve/process.rs",
    "crates/strop-worker/src/serve/process/pty.rs",
    "crates/strop-worker/src/serve/schedule.rs",
    "crates/strop-worker/src/serve/stream_window.rs",
    "crates/strop-worker/src/exec/supervisor.rs",
    "crates/strop-worker-client/src/lib.rs",
    "crates/strop-worker-client/src/connection.rs",
    "crates/strop-worker-client/src/connection/reader.rs",
    "crates/strop-worker-client/src/exec.rs",
    "crates/strop-worker-client/src/payload.rs",
    "crates/strop-worker-client/src/session.rs",
    "crates/strop-worker-client/src/lifecycle.rs",
    "crates/strop-terminal/src/client.rs",
    "crates/strop-fs/src/context.rs",
    "crates/strop-fs/src/attributes.rs",
    "crates/strop-fs/src/local/store.rs",
    "crates/strop-fs/src/local/verify.rs",
    "crates/strop-lsp/src/client/spawn.rs",
    "crates/strop-lsp/src/client/worker_io.rs",
    "crates/strop-picker/src/source/worker.rs",
    "crates/strop-picker/src/source/remote.rs",
    "crates/strop-git/src/exec.rs",
    "crates/strop-git/src/container.rs",
    "crates/strop-remote/src/worker_transport.rs",
    "crates/strop-remote/src/deploy_provider.rs",
    "crates/strop-worker-deploy/src/lib.rs",
    "crates/strop-worker-deploy/src/manifest.rs",
    "crates/strop-worker-deploy/src/deploy.rs",
    "crates/strop-worker-deploy/src/cache.rs",
    "crates/strop-worker-deploy/src/container.rs",
    "crates/strop-worker-deploy/src/provider.rs",
    "crates/strop-engine/src/editor/namespace.rs",
    "crates/strop-engine/src/editor/io/save.rs",
    "crates/strop-engine/src/editor/remote/save.rs",
    "crates/strop-engine/src/editor/remote/save/store.rs",
    "crates/strop-engine/src/editor/remote/workers.rs",
    "crates/strop-engine/src/editor/containers/container_worker.rs",
    "crates/strop-engine/src/editor/remote/controls.rs",
    "crates/strop-engine/src/editor/lsp/attach.rs",
    "crates/strop-engine/src/editor/picker/query.rs",
    "crates/strop-engine/src/editor/explain.rs",
]

BASELINE = {
    "candidate_sha256": "b7f3045cdf94f4b97220a4e7aa71abc131747623c81c8b5f81e2fdfad20d1307",
    "inventory_sha256": "40c0b816e9b91cdc8f2357d88a7cdfab1c26982884088d2166e589711fc1b798",
    "helper_digests_sha256": "7067e2969445da4876668744300d736bb3c9320ece0d8aed7be98a560ad49414",
    "linux_candidate_sha256": "15ab49230ad7f76a6dc5f0901f07ac87e0f43f25e8d8388dea6a2c8b04aed7b2",
    "linux_inventory_sha256": "5bd0ad567a28e8562edaa4d5f12879a8b68f07af6b25d2c18c91ac83a8adde61",
    "linux_evidence_sha256": "2f0fa785f992b904da6f5796d424d3eee3972bf1485c4565dd54940a526735ba",
}

BASELINE_PATHS = {
    "candidate_sha256": "verification/baseline/0057-candidate.json",
    "inventory_sha256": "verification/baseline/0057-inventory.json",
    "helper_digests_sha256": "verification/baseline/0057-helper-digests.json",
    "linux_candidate_sha256": "verification/baseline/0057-linux-candidate.json",
    "linux_inventory_sha256": "verification/baseline/0057-linux-inventory.json",
    "linux_evidence_sha256": "verification/baseline/0057-linux-evidence.json",
}

LINUX_MEASUREMENTS = (
    "verification/measurements/0058-linux-x86-preworker.json",
    "verification/measurements/0058-linux-x86-worker.json",
    "verification/measurements/0058-linux-x86-comparison.json",
)

UI_FRAME_MEASUREMENTS = (
    "verification/measurements/0058-linux-x86-preworker-ui-frame.json",
    "verification/measurements/0058-linux-x86-worker-ui-frame.json",
    "verification/measurements/0058-linux-x86-ui-frame-comparison.json",
)

TUI_FRAME_MEASUREMENTS = (
    "verification/measurements/0058-linux-x86-preworker-tui-frame.json",
    "verification/measurements/0058-linux-x86-worker-tui-frame.json",
    "verification/measurements/0058-linux-x86-tui-frame-comparison.json",
)

TERMINAL_LOAD_MEASUREMENTS = (
    "verification/measurements/0058-linux-x86-preworker-terminal-load-after-fix.json",
    "verification/measurements/0058-linux-x86-worker-terminal-load-after-fix.json",
    "verification/measurements/0058-linux-x86-terminal-load-after-fix-comparison.json",
)
NATIVE_PRODUCT_REPORT = "verification/measurements/0058-linux-x86-native-product.json"

SCHEMA = 6


def sha256_file(rel: str) -> str:
    data = (ROOT / rel).read_bytes()
    return hashlib.sha256(data).hexdigest()


def git(*args: str) -> str:
    out = subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    )
    return out.stdout.strip()


def dockerfile_pins(text: str) -> dict:
    stages = {}
    for m in re.finditer(r"(?m)^FROM\s+(\S+?)(?:\s+AS\s+(\S+))?\s*$", text):
        image, name = m.group(1), m.group(2)
        if name and "@" in image:  # external pinned base (AR15 convention)
            stages[name] = image
    checksums = {}
    for m in re.finditer(
        r"(?m)^ADD\s+--checksum=sha256:\$\{(\w+)\}\s+(\S+)", text
    ):
        arg, url = m.group(1), m.group(2)
        am = re.search(rf"(?m)^ARG\s+{arg}=(\S+)\s*$", text)
        checksums[arg] = {
            "url": url,
            "sha256": am.group(1) if am else None,
        }
    return {"stage_base_images": stages, "checksum_pinned_downloads": checksums}

def zig_pins(text: str) -> dict:
    version = re.search(r"ziglang\.org/download/([^/]+)/", text)
    digests = dict(re.findall(r'(\w+-\w+);\s*digest=([0-9a-f]{64})', text))
    return {"version": version.group(1) if version else None, "digests": digests}


def cargo_toml_pins(text: str) -> dict:
    pins = {}
    for name in ("verus_builtin", "verus_builtin_macros"):
        m = re.search(rf'(?m)^{name}\s*=\s*"([^"]+)"', text)
        if m:
            pins[name] = m.group(1)
    return pins


def baseline_hashes() -> dict:
    actual = {key: sha256_file(path) for key, path in BASELINE_PATHS.items()}
    if actual != BASELINE:
        raise SystemExit(
            "pre-worker VF20 archive changed; refuse to re-freeze historical evidence"
        )
    scoped = json.loads((ROOT / BASELINE_PATHS["linux_evidence_sha256"]).read_text())
    candidate = json.loads((ROOT / BASELINE_PATHS["linux_candidate_sha256"]).read_text())
    if (
        scoped.get("qualification") != "linux-x86_64-local-evidence-only"
        or candidate.get("dirty")
        or scoped.get("commit") != candidate.get("commit")
        or scoped.get("tree") != candidate.get("tree")
        or scoped.get("helper_digests_sha256") != actual["helper_digests_sha256"]
    ):
        raise SystemExit("Linux-only pre-worker evidence is inconsistent with its clean source")
    for lane in scoped["gates"].values():
        if lane.get("status") != "passed" or sha256_file(lane["log"]) != lane["sha256"]:
            raise SystemExit("Linux-only pre-worker raw gate evidence changed")
    return actual

def linux_measurements() -> dict:
    hashes = {rel: sha256_file(rel) for rel in LINUX_MEASUREMENTS}
    baseline, worker, summary = [
        json.loads((ROOT / rel).read_text()) for rel in LINUX_MEASUREMENTS
    ]
    if (
        baseline.get("platform") != worker.get("platform")
        or len(baseline.get("warm_runs", [])) != 64
        or len(worker.get("warm_runs", [])) != 64
        or summary["baseline"]["sha256"] != hashes[LINUX_MEASUREMENTS[0]]
        or summary["candidate"]["sha256"] != hashes[LINUX_MEASUREMENTS[1]]
        or summary["method"]["sha256"] != sha256_file("verification/bench_worker.py")
        or summary["baseline"]["artifact_sha256"] != baseline.get("binary_sha256")
        or summary["candidate"]["artifact_sha256"] != worker.get("binary_sha256")
    ):
        raise SystemExit("Linux warm-handshake measurements do not bind the same fixture")
    return hashes


def warm_roundtrip_evidence() -> dict:
    rel = "verification/measurements/0058-linux-x86-worker-roundtrip.json"
    measured = json.loads((ROOT / rel).read_text(encoding="utf-8"))
    raw = measured.get("raw_ms")
    if (
        not isinstance(raw, list)
        or len(raw) != 64
        or not all(isinstance(value, (int, float)) and math.isfinite(value) and value >= 0
                   for value in raw)
        or measured.get("measured_requests") != len(raw)
        or measured.get("warmup_requests") != 8
        or measured.get("target") != "x86_64-unknown-linux-musl"
        or measured.get("platform", {}).get("machine") != "x86_64"
        or measured.get("platform", {}).get("profile") != "release-musl-stripped"
        or not re.fullmatch(r"[0-9a-f]{64}", measured.get("binary_sha256", ""))
    ):
        raise SystemExit("Linux control-frame evidence does not bind the scoped native profile")
    ordered = sorted(raw)
    percentiles = {
        f"p{n}": ordered[math.ceil(n * len(raw) / 100) - 1]
        for n in (50, 95, 99)
    } | {"max": ordered[-1]}
    if (
        measured.get("roundtrip_ms") != percentiles
        or measured.get("request_bytes", {}).get("total", 0) <= 0
    ):
        raise SystemExit("Linux control-frame summary disagrees with its raw samples")
    return {
        "sha256": sha256_file(rel),
        "binary_sha256": measured["binary_sha256"],
        "method_sha256": sha256_file("verification/bench_worker_roundtrip.py"),
    }

def ui_frame_evidence() -> dict:
    hashes = {rel: sha256_file(rel) for rel in UI_FRAME_MEASUREMENTS}
    baseline, worker, summary = [
        json.loads((ROOT / rel).read_text(encoding="utf-8"))
        for rel in UI_FRAME_MEASUREMENTS
    ]
    if (
        baseline.get("platform") != worker.get("platform")
        or baseline.get("platform", {}).get("machine") != "x86_64"
        or baseline.get("platform", {}).get("profile") != "release-musl-stripped"
        or summary.get("schema") != 1
        or summary.get("method", {}).get("sha256")
        != sha256_file("verification/bench_ui_input_frame.py")
        or summary["method"]["harness"] != "verification/bench_ui_input_frame.py"
        or summary["method"]["samples_per_artifact"] != 64
        or summary["method"]["warmup"] != 8
        or summary["method"]["fixture_lines"] != 10_000
        or summary["method"]["geometry"] != [120, 40]
    ):
        raise SystemExit("Linux UI action-to-frame evidence does not bind one native fixture")
    warm_artifacts = [
        json.loads((ROOT / rel).read_text(encoding="utf-8"))
        for rel in LINUX_MEASUREMENTS[:2]
    ]
    for role, rel, measured, artifact in zip(
        ("baseline", "candidate"), UI_FRAME_MEASUREMENTS[:2], (baseline, worker),
        warm_artifacts, strict=True
    ):
        binary_sha256 = artifact["binary_sha256"]
        raw = measured.get("raw_ms")
        fixture = measured.get("fixture", {})
        if (
            not isinstance(raw, list)
            or len(raw) != 64
            or not all(isinstance(value, (int, float)) and math.isfinite(value)
                       and value >= 0 for value in raw)
            or measured.get("measured_requests") != 64
            or measured.get("warmup_requests") != 8
            or measured.get("binary_sha256") != binary_sha256
            or measured.get("binary_version") != artifact["version"]
            or measured.get("method") != summary["method"]["timing"]
            or fixture.get("worker_count_after_open") != 1
            or fixture.get("lines") != 10_000
            or fixture.get("geometry") != [120, 40]
            or fixture.get("input") != "one committed text character"
            or any(measured.get(key, 0) <= 0
                   for key in ("request_bytes", "ack_bytes", "view_bytes"))
            or summary[role]["artifact_sha256"] != binary_sha256
            or summary[role]["measurement"] != rel
            or summary[role]["sha256"] != hashes[rel]
        ):
            raise SystemExit(f"Linux {role} UI sample is incomplete or differs from its artifact")
        ordered = sorted(raw)
        percentiles = {f"p{n}": ordered[math.ceil(n * len(raw) / 100) - 1]
                       for n in (50, 95, 99)} | {"max": ordered[-1]}
        if (
            measured.get("input_to_view_ms") != percentiles
            or summary["observed"][f"{role}_ms"] != percentiles
            or summary["observed"][f"{role}_worker_rss_kib_after_open"]
            != fixture.get("worker_rss_kib_after_open")
            or summary["observed"][f"{role}_editor_rss_kib_after_open"]
            != fixture.get("editor_rss_kib_after_open")
            or summary["observed"][f"{role}_request_bytes"] != measured["request_bytes"]
            or summary["observed"][f"{role}_view_bytes"] != measured["view_bytes"]
        ):
            raise SystemExit(f"Linux {role} UI summary disagrees with its raw frames")
    return {
        "samples": hashes,
        "method_sha256": sha256_file("verification/bench_ui_input_frame.py"),
        "binary_sha256": {"baseline": baseline["binary_sha256"],
                          "candidate": worker["binary_sha256"]},
    }


def tui_frame_evidence() -> dict:
    hashes = {rel: sha256_file(rel) for rel in TUI_FRAME_MEASUREMENTS}
    baseline, worker, summary = [
        json.loads((ROOT / rel).read_text(encoding="utf-8"))
        for rel in TUI_FRAME_MEASUREMENTS
    ]
    if (
        baseline.get("platform") != worker.get("platform")
        or baseline.get("platform") != {"os": "linux", "arch": "x86_64"}
        or summary.get("schema") != 1
        or summary.get("method", {}).get("harness") != "crates/strop/tests/terminal_editor.rs"
        or summary["method"]["test"] != "native_terminal_input_to_painted_frame_samples"
        or summary["method"]["sha256"] != sha256_file("crates/strop/tests/terminal_editor.rs")
        or summary["method"]["warmup"] != 8
        or summary["method"]["samples_per_artifact"] != 64
        or summary["method"]["fixture_lines"] != 10_000
        or summary["method"]["geometry"] != [120, 30]
    ):
        raise SystemExit("Linux TUI paint evidence does not bind the native test and fixture")
    for role, rel, measured, warm_rel in zip(
        ("baseline", "candidate"), TUI_FRAME_MEASUREMENTS[:2], (baseline, worker),
        LINUX_MEASUREMENTS[:2], strict=True
    ):
        artifact = json.loads((ROOT / warm_rel).read_text(encoding="utf-8"))
        raw = measured.get("raw_ms")
        fixture = measured.get("fixture", {})
        if (
            not isinstance(raw, list)
            or len(raw) != 64
            or not all(isinstance(value, (int, float)) and math.isfinite(value)
                       and value >= 0 for value in raw)
            or measured.get("warmup_requests") != 8
            or measured.get("measured_requests") != 64
            or measured.get("binary_sha256") != artifact["binary_sha256"]
            or measured.get("binary_bytes") != artifact["bytes"]
            or measured.get("method") != summary["method"]["timing"]
            or fixture.get("capture") is not False
            or fixture.get("lines") != 10_000
            or fixture.get("geometry") != [120, 30]
            or fixture.get("input") != "one committed character at line 5000"
            or summary[role]["artifact_sha256"] != artifact["binary_sha256"]
            or summary[role]["measurement"] != rel
            or summary[role]["sha256"] != hashes[rel]
        ):
            raise SystemExit(f"Linux {role} TUI paint does not match the measured artifact")
        ordered = sorted(raw)
        percentiles = {f"p{n}": ordered[math.ceil(n * len(raw) / 100) - 1]
                       for n in (50, 95, 99)} | {"max": ordered[-1]}
        if (measured.get("input_to_grid_ms") != percentiles
                or summary["observed"][f"{role}_ms"] != percentiles):
            raise SystemExit(f"Linux {role} TUI summary disagrees with its painted frames")
    return {
        "samples": hashes,
        "method_sha256": sha256_file("crates/strop/tests/terminal_editor.rs"),
        "binary_sha256": {"baseline": baseline["binary_sha256"],
                          "candidate": worker["binary_sha256"]},
    }


def terminal_load_evidence() -> dict:
    hashes = {rel: sha256_file(rel) for rel in TERMINAL_LOAD_MEASUREMENTS}
    baseline, worker, summary = [
        json.loads((ROOT / rel).read_text(encoding="utf-8"))
        for rel in TERMINAL_LOAD_MEASUREMENTS
    ]
    method = summary.get("method", {})
    if (
        summary.get("schema") != 1
        or method.get("harness") != "crates/strop/tests/terminal_editor.rs"
        or method.get("test") != "native_terminal_output_under_load_samples"
        or method.get("sha256") != sha256_file("crates/strop/tests/terminal_editor.rs")
        or method.get("warmup") != 8
        or method.get("samples_per_artifact") != 64
        or method.get("output_lines_per_request") != 256
        or method.get("geometry") != [120, 30]
        or baseline.get("platform") != worker.get("platform")
        or baseline.get("platform") != {"os": "linux", "arch": "x86_64"}
    ):
        raise SystemExit("terminal-load source, platform and fixture are not pinned")
    for role, rel, measured, warm_rel in zip(
        ("baseline", "candidate"), TERMINAL_LOAD_MEASUREMENTS[:2],
        (baseline, worker), LINUX_MEASUREMENTS[:2], strict=True
    ):
        artifact = json.loads((ROOT / warm_rel).read_text(encoding="utf-8"))
        raw = measured.get("raw_ms")
        fixture = measured.get("fixture", {})
        if (
            not isinstance(raw, list)
            or len(raw) != 64
            or not all(isinstance(value, (int, float)) and math.isfinite(value)
                       and value >= 0 for value in raw)
            or measured.get("warmup_requests") != 8
            or measured.get("measured_requests") != 64
            or measured.get("binary_sha256") != artifact["binary_sha256"]
            or measured.get("binary_bytes") != artifact["bytes"]
            or measured.get("method") != method["timing"]
            or fixture.get("output_lines_per_request") != 256
            or fixture.get("geometry") != [120, 30]
            or fixture.get("input") != "one two-key line to a worker-leased shell PTY"
            or fixture.get("capture") is not False
            or summary[role]["artifact_sha256"] != artifact["binary_sha256"]
            or summary[role]["measurement"] != rel
            or summary[role]["sha256"] != hashes[rel]
        ):
            raise SystemExit(f"Linux {role} loaded terminal does not match its artifact")
        ordered = sorted(raw)
        percentiles = {f"p{n}": ordered[math.ceil(n * len(raw) / 100) - 1]
                       for n in (50, 95, 99)} | {"max": ordered[-1]}
        if (measured.get("output_to_grid_ms") != percentiles
                or summary["observed"][f"{role}_ms"] != percentiles):
            raise SystemExit(f"Linux {role} terminal-load summary differs from its raw frames")
    return {
        "samples": hashes,
        "method_sha256": sha256_file("crates/strop/tests/terminal_editor.rs"),
        "binary_sha256": {"baseline": baseline["binary_sha256"],
                          "candidate": worker["binary_sha256"]},
    }


def native_product_evidence() -> dict:
    report = json.loads((ROOT / NATIVE_PRODUCT_REPORT).read_text(encoding="utf-8"))
    methods = (
        "verification/bench_native_product.py",
        "verification/bench_worker.py",
        "verification/bench_worker_roundtrip.py",
        "verification/bench_ui_input_frame.py",
        "crates/strop/tests/terminal_editor.rs",
    )
    measurements = {
        "baseline_handshake_ms": (LINUX_MEASUREMENTS[0], "ready_ms"),
        "candidate_handshake_ms": (LINUX_MEASUREMENTS[1], "ready_ms"),
        "candidate_health_ms": ("verification/measurements/0058-linux-x86-worker-roundtrip.json",
                                "roundtrip_ms"),
        "baseline_semantic_ms": (UI_FRAME_MEASUREMENTS[0], "input_to_view_ms"),
        "candidate_semantic_ms": (UI_FRAME_MEASUREMENTS[1], "input_to_view_ms"),
        "baseline_tui_ms": (TUI_FRAME_MEASUREMENTS[0], "input_to_grid_ms"),
        "candidate_tui_ms": (TUI_FRAME_MEASUREMENTS[1], "input_to_grid_ms"),
        "baseline_terminal_load_ms": (TERMINAL_LOAD_MEASUREMENTS[0], "output_to_grid_ms"),
        "candidate_terminal_load_ms": (TERMINAL_LOAD_MEASUREMENTS[1], "output_to_grid_ms"),
    }
    if (
        report.get("schema") != 1
        or report.get("target") != "x86_64-unknown-linux-musl"
        or report.get("profile") != "release-musl-stripped"
        or report.get("samples_per_artifact") != 64
        or report.get("methods") != {rel: sha256_file(rel) for rel in methods}
        or report.get("baseline_sha256")
        != json.loads((ROOT / LINUX_MEASUREMENTS[0]).read_text())["binary_sha256"]
        or report.get("candidate_sha256")
        != json.loads((ROOT / LINUX_MEASUREMENTS[1]).read_text())["binary_sha256"]
        or set(report.get("observed", {})) != set(measurements)
    ):
        raise SystemExit("native product report does not bind its exact source/artifacts")
    for key, (rel, metric) in measurements.items():
        sample = json.loads((ROOT / rel).read_text(encoding="utf-8"))
        if report["observed"][key] != sample[metric]:
            raise SystemExit(f"native product comparison differs from {rel}")
    return {"sha256": sha256_file(NATIVE_PRODUCT_REPORT),
            "methods": report["methods"],
            "artifact_sha256": report["candidate_sha256"]}


def freeze(allow_dirty: bool) -> dict:
    commit = git("rev-parse", "HEAD")
    dirty = git("status", "--porcelain").splitlines()
    dirty = sorted(line[3:] for line in dirty if line.strip())
    if dirty and not allow_dirty:
        raise SystemExit(
            "refusing to freeze a dirty tree (a candidate is exact); "
            "dirty paths:\n  " + "\n  ".join(dirty) + "\n"
            "commit first, or pass --allow-dirty to record the dirt honestly"
        )
    dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    pins = dockerfile_pins(dockerfile)
    candidate = {
        "schema": SCHEMA,
        "release": "0058",
        "plan": "plans/0058-unified-native-worker.md",
        "commit": commit,
        "tree": git("rev-parse", "HEAD^{tree}"),
        # Deterministic: the commit's own date, not the wall clock.
        "commit_date": git("show", "-s", "--format=%cI", "HEAD"),
        "dirty": dirty,
        "cargo_lock_sha256": sha256_file("Cargo.lock"),
        "inventory_sha256": sha256_file("verification/inventory.json"),
        "worker_sources": {rel: sha256_file(rel) for rel in WORKER_SOURCES},
        "benchmark_sha256": sha256_file("verification/bench_worker.py"),
        "warm_roundtrip_benchmark_sha256": sha256_file(
            "verification/bench_worker_roundtrip.py"
        ),
        "linux_warm_handshake_evidence": linux_measurements(),
        "linux_warm_roundtrip_evidence": warm_roundtrip_evidence(),
        "linux_ui_input_frame_evidence": ui_frame_evidence(),
        "linux_tui_input_frame_evidence": tui_frame_evidence(),
        "linux_terminal_output_load_evidence": terminal_load_evidence(),
        "linux_native_product_evidence": native_product_evidence(),
        "baseline": baseline_hashes(),
        "install_transaction": {
            "install.sh": sha256_file("install.sh"),
            "crates/strop/src/update.rs": sha256_file("crates/strop/src/update.rs"),
            ".github/scripts/release-catalog.py": sha256_file(
                ".github/scripts/release-catalog.py"
            ),
        },
        "dockerfile_sha256": sha256_file("Dockerfile"),
        "dockerfile_stages": pins["stage_base_images"],
        "checksum_pinned_downloads": pins["checksum_pinned_downloads"],
        "zig_bootstrap": zig_pins(
            (ROOT / ".github/scripts/install-zig.sh").read_text(encoding="utf-8")
        ),
        "verus_crate_pins": cargo_toml_pins(
            (ROOT / "crates/strop-core/Cargo.toml").read_text(encoding="utf-8")
        ),
    }
    return candidate


def check(path: Path) -> int:
    try:
        recorded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: cannot read {path}: {exc}")
        return 1
    problems = []
    current = freeze(allow_dirty=True)
    if recorded.get("schema") != SCHEMA or recorded.get("release") != "0058":
        print("FAIL: this is not a current worker candidate; the VF20 archive is historical")
        return 1
    if recorded.get("commit") != git("rev-parse", "HEAD"):
        problems.append("commit does not match HEAD")
    if recorded.get("tree") != git("rev-parse", "HEAD^{tree}"):
        problems.append("committed tree differs from the freeze")
    if recorded.get("dirty") or git("status", "--porcelain"):
        problems.append("candidate or current worktree is dirty; not release-qualified")
    if recorded.get("cargo_lock_sha256") != sha256_file("Cargo.lock"):
        problems.append("Cargo.lock drifted from the freeze")
    if recorded.get("inventory_sha256") != sha256_file("verification/inventory.json"):
        problems.append("inventory drifted from the freeze")
    sources = recorded.get("worker_sources") or {}
    if sorted(sources) != sorted(WORKER_SOURCES):
        problems.append("worker source inventory differs from the freeze contract")
    for rel, digest in sources.items():
        path = ROOT / rel
        if not path.is_file() or sha256_file(rel) != digest:
            problems.append(f"worker source drifted or missing: {rel}")
    if recorded.get("baseline") != baseline_hashes():
        problems.append("pre-worker baseline archive differs from the freeze")
    if recorded.get("dockerfile_sha256") != sha256_file("Dockerfile"):
        problems.append("Dockerfile drifted from the freeze")
    for section in (
        "linux_warm_handshake_evidence",
        "linux_warm_roundtrip_evidence",
        "linux_ui_input_frame_evidence",
        "linux_tui_input_frame_evidence",
        "linux_terminal_output_load_evidence",
        "linux_native_product_evidence",
        "benchmark_sha256",
        "warm_roundtrip_benchmark_sha256",
        "verus_crate_pins",
        "zig_bootstrap",
        "dockerfile_stages",
        "checksum_pinned_downloads",
        "install_transaction",
    ):
        if recorded.get(section) != current[section]:
            problems.append(f"{section} differs from the frozen source inputs")
    if problems:
        for p in problems:
            print(f"FAIL: {p}")
        return 1
    print(f"candidate check ok: {recorded['commit'][:12]} matches the tree")
    return 0


def main(argv) -> int:
    allow_dirty = False
    out = DEFAULT_OUT
    mode = "freeze"
    i = 1
    while i < len(argv):
        arg = argv[i]
        if arg == "--allow-dirty":
            allow_dirty = True
        elif arg == "--check":
            mode = "check"
        elif arg == "--out":
            i += 1
            if i >= len(argv):
                print("usage: --out PATH", file=sys.stderr)
                return 2
            out = Path(argv[i])
        else:
            print(__doc__, file=sys.stderr)
            return 2
        i += 1
    if mode == "check":
        return check(out)
    candidate = freeze(allow_dirty)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(
        json.dumps(candidate, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"froze candidate {candidate['commit'][:12]} -> {out}")
    if candidate["dirty"]:
        print(f"WARNING: tree is dirty ({len(candidate['dirty'])} paths recorded)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
