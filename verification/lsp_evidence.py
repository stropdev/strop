"""Validate matched native Rust-analyzer journeys without growing freeze.py.

The edit route bounds didChange plus the real request and UI publication;
LSP offers no didChange acknowledgment, so it cannot prove sync alone.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path

REPORT = "verification/measurements/0058-linux-x86-lsp-ui.json"
METHOD_FILES = {
    "path": "verification/bench_lsp_ui.py",
    "ui_peer": "verification/bench_ui_input_frame.py",
    "worker_metrics": "verification/bench_worker.py",
}


def digest(root: Path, relative: str) -> str:
    with (root / relative).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def validate(root: Path, preworker: str, worker: str) -> dict:
    report = json.loads((root / REPORT).read_text(encoding="utf-8"))
    method = report.get("method")
    expected_method = {}
    for key, path in METHOD_FILES.items():
        expected_method[key] = path
        expected_method["sha256" if key == "path" else f"{key}_sha256"] = digest(root, path)
    if report.get("schema") != 1 or method != expected_method:
        raise SystemExit("native LSP report lacks its exact method and helper bindings")
    expected = {
        "baseline": json.loads((root / preworker).read_text(encoding="utf-8")),
        "candidate": json.loads((root / worker).read_text(encoding="utf-8")),
    }
    baseline = report.get("baseline")
    candidate = report.get("candidate")
    if not isinstance(baseline, dict) or not isinstance(candidate, dict):
        raise SystemExit("native LSP report lacks matched baseline and candidate")
    if (baseline.get("analyzer") != candidate.get("analyzer")
            or not str(baseline.get("analyzer", "")).startswith("rust-analyzer ")
            or baseline.get("platform") != candidate.get("platform")
            or baseline.get("platform", {}).get("machine") != "x86_64"
            or baseline.get("platform", {}).get("profile") != "release-musl-stripped"):
        raise SystemExit("native LSP samples did not use one real server/host profile")
    for name, sample in (("baseline", baseline), ("candidate", candidate)):
        artifact = expected[name]
        if (sample.get("binary_sha256") != artifact["binary_sha256"]
                or sample.get("binary_bytes") != artifact["bytes"]
                or sample.get("binary_version") != artifact["version"]
                or sample.get("warmup_requests") != 8
                or sample.get("measured_requests") != 64
                or sample.get("fixture", {}).get("worker_count") != 1
                or sample.get("fixture", {}).get("sync_scope")
                != "edit+didChange+request+view, not didChange acknowledgment"):
            raise SystemExit(f"native LSP {name} did not use its exact worker/fixture")
        for raw_name, summary_name in (("request_raw_ms", "request_ms"),
                                       ("sync_request_raw_ms", "sync_request_ms")):
            raw = sample.get(raw_name)
            if (not isinstance(raw, list) or len(raw) != 64
                    or not all(isinstance(value, (int, float)) and math.isfinite(value)
                               and value >= 0 for value in raw)):
                raise SystemExit(f"native LSP {name} {raw_name} has no 64 valid samples")
            ordered = sorted(raw)
            stats = {f"p{n}": ordered[math.ceil(n * 64 / 100) - 1]
                     for n in (50, 95, 99)} | {"max": ordered[-1]}
            if sample.get(summary_name) != stats:
                raise SystemExit(f"native LSP {name} {summary_name} differs from raw")
    return {
        "sha256": digest(root, REPORT),
        "method_sha256": method["sha256"],
        "validator_sha256": digest(root, "verification/lsp_evidence.py"),
        "baseline_artifact_sha256": baseline["binary_sha256"],
        "candidate_artifact_sha256": candidate["binary_sha256"],
    }
