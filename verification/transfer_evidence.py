"""Validate real UI transfer, memory and post-exit worker retirement samples.

The historical pre-worker artifact is a comparison, not a passing retirement
control. Only the new candidate may claim its worker is gone at editor exit.
"""

from __future__ import annotations

import hashlib
import json
import math
from pathlib import Path

REPORTS = {
    "baseline": "verification/measurements/0058-linux-x86-preworker-transfer-ui.json",
    "candidate": "verification/measurements/0058-linux-x86-worker-transfer-ui.json",
}
METHODS = (
    "verification/bench_transfer_ui.py",
    "verification/bench_ui_input_frame.py",
    "verification/bench_worker.py",
)
METRICS = (
    "open_to_view_ms", "editor_rss_kib", "worker_rss_kib",
    "editor_peak_kib", "worker_peak_kib", "editor_threads",
    "worker_threads", "workers_after_exit",
)
SOURCE_SHA256 = "e98467d3f3f38286b9928b5c6466e94839f846ea932ba6d7f8dbb513be0a2254"


def digest(root: Path, relative: str) -> str:
    with (root / relative).open("rb") as source:
        return hashlib.file_digest(source, "sha256").hexdigest()


def percentiles(raw: list[int | float]) -> dict:
    ordered = sorted(raw)
    return {f"p{n}": ordered[math.ceil(n * len(raw) / 100) - 1]
            for n in (50, 95, 99)} | {"max": ordered[-1]}


def validate(root: Path, preworker: str, worker: str) -> dict:
    artifacts = {
        "baseline": json.loads((root / preworker).read_text(encoding="utf-8")),
        "candidate": json.loads((root / worker).read_text(encoding="utf-8")),
    }
    samples = {role: json.loads((root / path).read_text(encoding="utf-8"))
               for role, path in REPORTS.items()}
    if (samples["baseline"].get("platform") != samples["candidate"].get("platform")
            or samples["candidate"].get("platform", {}).get("machine") != "x86_64"
            or samples["candidate"].get("platform", {}).get("profile")
            != "release-musl-stripped"
            or samples["candidate"].get("fixture") != samples["baseline"].get("fixture")):
        raise SystemExit("native transfer reports lack the matched Linux fixture and profile")
    for role, report in samples.items():
        artifact = artifacts[role]
        fixture = report.get("fixture", {})
        if (report.get("binary_sha256") != artifact["binary_sha256"]
                or report.get("binary_bytes") != artifact["bytes"]
                or report.get("binary_version") != artifact["version"]
                or report.get("warmup_requests") != 8
                or report.get("measured_requests") != 64
                or fixture.get("bytes") != 17_280_000
                or fixture.get("lines") != 10_000
                or fixture.get("source_sha256") != SOURCE_SHA256
                or fixture.get("geometry") != [120, 40]
                or fixture.get("worker_count_after_each_open") != 1
                or fixture.get("rss_peak_kind") != "kernel VmHWM"
                or fixture.get("retirement_scope") !=
                "immediate state after editor exit; fixture kills only its owned worker after observation"
                or report.get("method") !=
                "file open to complete 120x40 UI frame, verify last line and observe post-exit worker"):
            raise SystemExit(f"{role} native transfer does not bind its artifact and fixture")
        raw = report.get("raw")
        if not isinstance(raw, list) or len(raw) != 64:
            raise SystemExit(f"{role} transfer requires 64 real UI journeys")
        for row in raw:
            if (not isinstance(row, dict) or row.get("workers_after_open") != 1
                    or row.get("worker_post_exit_state") not in ("-", "S", "R", "D", "Z")
                    or row.get("workers_after_exit") != int(row["worker_post_exit_state"] != "-")
                    or any(type(row.get(metric)) not in (int, float)
                           or not math.isfinite(row[metric]) or row[metric] < 0
                           for metric in METRICS)
                    or row["editor_peak_kib"] < row["editor_rss_kib"]
                    or row["worker_peak_kib"] < row["worker_rss_kib"]):
                raise SystemExit(f"{role} transfer has an incomplete process/retirement observation")
        if (role == "candidate" and any(row["workers_after_exit"] != 0 for row in raw)):
            raise SystemExit("candidate UI shutdown left a real worker behind")
        if report.get("summary") != {metric: percentiles([row[metric] for row in raw])
                                      for metric in METRICS}:
            raise SystemExit(f"{role} transfer summaries differ from the raw journeys")
    return {
        "samples": {role: digest(root, path) for role, path in REPORTS.items()},
        "methods": {path: digest(root, path) for path in METHODS},
        "validator_sha256": digest(root, "verification/transfer_evidence.py"),
        "binary_sha256": {role: report["binary_sha256"] for role, report in samples.items()},
    }
