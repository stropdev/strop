#!/usr/bin/env python3
"""Measure matched pre-worker/current native binaries through live worker and UI paths.

Run on the same native host, release build profile, and fixture for both
artifacts. All seven sampled journeys must succeed before any result is written. This
is local launch, Health, worker-backed UI semantic view and PTY TUI paint;
remote deploy, LSP and terminal output load have separate WK20 obligations.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import re
import subprocess
import sys
from pathlib import Path

from bench_worker import native_profile

ROOT = Path(__file__).resolve().parents[1]
METHODS = (
    "verification/bench_native_product.py",
    "verification/bench_worker.py",
    "verification/bench_worker_roundtrip.py",
    "verification/bench_ui_input_frame.py",
    "crates/strop/tests/terminal_editor.rs",
)
TUI_MARKER = re.compile(r"STROP_TUI_BENCH=(\{[^\n]+\})")


def execute(command: list[str], *, binary: Path | None = None) -> str:
    env = dict(os.environ)
    if binary is not None:
        env["STROP_BENCH_BINARY"] = str(binary)
    result = subprocess.run(command, cwd=ROOT, env=env, capture_output=True,
                            text=True, timeout=900, check=False)
    if result.returncode != 0:
        raise RuntimeError(f"{command[0]} returned {result.returncode}:\n"
                           f"{result.stdout}\n{result.stderr}")
    return result.stdout


def version(binary: Path) -> str:
    parts = execute([str(binary), "--version"]).strip().split()
    if len(parts) != 2 or parts[0] != "strop":
        raise ValueError(f"not a Strop release binary: {binary}")
    return parts[1]


def sample(binary: Path, target: str, profile: str, protocol: int) -> dict[str, dict]:
    python = sys.executable
    worker = json.loads(execute([
        python, "-B", "verification/bench_worker.py", "--binary", str(binary),
        "--protocol", str(protocol), "--version", version(binary),
        "--target", target, "--warmup", "8", "--iterations", "64",
    ]))
    semantic = json.loads(execute([
        python, "-B", "verification/bench_ui_input_frame.py", "--binary", str(binary),
        "--profile", profile, "--warmup", "8", "--iterations", "64",
    ]))
    tested = execute([
        "cargo", "test", "--release", "--locked", "-p", "strop-editor",
        "--test", "terminal_editor", "native_terminal_input_to_painted_frame_samples",
        "--", "--ignored", "--nocapture",
    ], binary=binary)
    matched = TUI_MARKER.findall(tested)
    if len(matched) != 1 or "test result: ok. 1 passed" not in tested:
        raise RuntimeError("the real PTY test did not publish one passing benchmark")
    grid = json.loads(matched[0])
    digest = worker["binary_sha256"]
    for name, observed, values, summary in (
        ("handshake", worker, [row["ready_ms"] for row in worker["warm_runs"]],
         worker["ready_ms"]),
        ("semantic", semantic, semantic["raw_ms"], semantic["input_to_view_ms"]),
        ("grid", grid, grid["raw_ms"], grid["input_to_grid_ms"]),
    ):
        if (observed["binary_sha256"] != digest or len(values) != 64
                or observed.get("measured_requests", 64) != 64
                or observed.get("warmup_requests", observed.get("warmup_runs")) != 8
                or not all(isinstance(n, (int, float)) and math.isfinite(n) and n >= 0
                           for n in values)):
            raise RuntimeError(f"{name} did not measure the exact 64-request artifact")
        ordered = sorted(values)
        expected = {f"p{n}": ordered[math.ceil(n * 64 / 100) - 1]
                    for n in (50, 95, 99)} | {"max": ordered[-1]}
        if summary != expected:
            raise RuntimeError(f"{name} percentiles disagree with its raw observations")
    if semantic["fixture"]["worker_count_after_open"] != 1:
        raise RuntimeError("editor file fixture did not retain exactly one local worker")
    if worker["platform"] != semantic["platform"]:
        raise RuntimeError("worker and UI samples were not taken on the same native host")
    expected_os = "macos" if target.endswith("-apple-darwin") else "linux"
    if (grid["platform"]["os"] != expected_os
            or grid["platform"]["arch"] != target.split("-", 1)[0]):
        raise RuntimeError("decoded terminal grid ran on a different native target")
    return {"handshake": worker, "semantic_frame": semantic, "tui_cell_grid": grid}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--baseline", required=True, type=Path)
    parser.add_argument("--candidate", required=True, type=Path)
    parser.add_argument("--target", required=True)
    parser.add_argument("--out", required=True, type=Path)
    args = parser.parse_args()
    baseline = args.baseline.resolve()
    candidate = args.candidate.resolve()
    if not all(path.is_file() and os.access(path, os.X_OK)
               for path in (baseline, candidate)):
        parser.error("both native release artifacts must exist and be executable")
    profile = native_profile(args.target)
    try:
        base = sample(baseline, args.target, profile, protocol=1)
        current = sample(candidate, args.target, profile, protocol=2)
        control = json.loads(execute([
            sys.executable, "-B", "verification/bench_worker_roundtrip.py",
            "--binary", str(candidate), "--protocol", "2",
            "--version", version(candidate), "--target", args.target,
            "--profile", profile, "--warmup", "8", "--iterations", "64",
        ]))
        raw = control["raw_ms"]
        if (control["binary_sha256"] != current["handshake"]["binary_sha256"]
                or control["platform"] != current["handshake"]["platform"]
                or control["warmup_requests"] != 8
                or control["measured_requests"] != 64
                or len(raw) != 64
                or not all(isinstance(n, (int, float)) and math.isfinite(n) and n >= 0
                           for n in raw)):
            raise RuntimeError("control route is not the same candidate artifact and host")
        ordered = sorted(raw)
        expected = {f"p{n}": ordered[math.ceil(n * 64 / 100) - 1]
                    for n in (50, 95, 99)} | {"max": ordered[-1]}
        if control["roundtrip_ms"] != expected:
            raise RuntimeError("control frame summary differs from its raw 64 responses")
        if (base["handshake"]["platform"] != current["handshake"]["platform"]
                or base["handshake"]["target"] != current["handshake"]["target"]):
            raise RuntimeError("pre-worker and worker targets/hosts differ")
        records = {
            "baseline-handshake": base["handshake"],
            "candidate-handshake": current["handshake"],
            "baseline-semantic": base["semantic_frame"],
            "candidate-semantic": current["semantic_frame"],
            "baseline-tui": base["tui_cell_grid"],
            "candidate-tui": current["tui_cell_grid"],
            "candidate-control": control,
        }
        hashes = {rel: hashlib.sha256((ROOT / rel).read_bytes()).hexdigest()
                  for rel in METHODS}
        report = {
            "schema": 1, "target": args.target, "profile": profile,
            "host": current["handshake"]["platform"],
            "baseline_sha256": base["handshake"]["binary_sha256"],
            "candidate_sha256": current["handshake"]["binary_sha256"],
            "methods": hashes,
            "samples_per_artifact": 64,
            "scope": "native local worker Hello, Health, worker-backed semantic edit, real PTY TUI cell-grid paint; not cold/warm remote deploy, LSP, terminal output load or retirement high-water",
            "observed": {
                "baseline_handshake_ms": base["handshake"]["ready_ms"],
                "candidate_handshake_ms": current["handshake"]["ready_ms"],
                "candidate_health_ms": control["roundtrip_ms"],
                "baseline_semantic_ms": base["semantic_frame"]["input_to_view_ms"],
                "candidate_semantic_ms": current["semantic_frame"]["input_to_view_ms"],
                "baseline_tui_ms": base["tui_cell_grid"]["input_to_grid_ms"],
                "candidate_tui_ms": current["tui_cell_grid"]["input_to_grid_ms"],
            },
        }
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired,
            json.JSONDecodeError) as error:
        parser.exit(1, f"native product benchmark failed: {error}\n")
    args.out.mkdir(parents=True, exist_ok=True)
    for name, record in records.items():
        (args.out / f"{name}.json").write_text(json.dumps(record, indent=2) + "\n")
    (args.out / "comparison.json").write_text(json.dumps(report, indent=2) + "\n")
    print(f"native product measurements: {args.out} ({args.target}, 64 samples per artifact)")


if __name__ == "__main__":
    main()
