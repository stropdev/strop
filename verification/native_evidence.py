"""Bind the retained native profiles to their actual runtime and benchmark inputs."""

from __future__ import annotations

import json
import math
import tomllib
from pathlib import Path

from bench_native_product import METHODS as PRODUCT_METHODS
from transfer_evidence import METRICS, SOURCE_SHA256, digest, percentiles

REPORT = "verification/measurements/0058-retained-native-platforms.json"
TARGETS = (
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "aarch64-apple-darwin",
)
METHODS = (*PRODUCT_METHODS, "verification/bench_lsp_ui.py", "verification/bench_transfer_ui.py")


def checked_samples(raw: list, summary: dict, label: str) -> None:
    if (not isinstance(raw, list) or len(raw) != 64
            or not all(type(value) in (int, float) and math.isfinite(value) and value >= 0
                       for value in raw)
            or summary != percentiles(raw)):
        raise SystemExit(f"{label}: native summary lacks its exact 64 valid observations")


def validate(root: Path, source_paths: list[str]) -> dict:
    report = json.loads((root / REPORT).read_text(encoding="utf-8"))
    methods = {path: digest(root, path) for path in METHODS}
    source = report.get("source", {})
    profiles = report.get("profiles", {})
    if (report.get("schema") != 1 or set(profiles) != set(TARGETS)
            or report.get("methods") != methods
            or source.get("cargo_lock_sha256") != digest(root, "Cargo.lock")
            or source.get("cargo_manifest_sha256") != digest(root, "Cargo.toml")
            or source.get("zig_bootstrap_sha256") != digest(root, ".github/scripts/install-zig.sh")
            or source.get("worker_sources") != {path: digest(root, path) for path in source_paths}
            or source.get("native_workflow_sha256") != digest(root, ".github/workflows/ci.yml")
            or source.get("repository") != "stropdev/strop"
            or source.get("baseline_commit") != "a05d84f823d268259ca4c4c3358313c902f6189d"):
        raise SystemExit("retained native evidence has stale source/methods or missing/retired targets")
    artifacts = {}
    current_version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    for target, records in profiles.items():
        comparison = records["comparison"]
        profile = "release-macos-native" if target.endswith("darwin") else "release-gnu-native"
        if (comparison.get("target") != target or comparison.get("profile") != profile
                or comparison.get("samples_per_artifact") != 64
                or comparison.get("methods") != {path: methods[path] for path in PRODUCT_METHODS}):
            raise SystemExit(f"{target}: native product method or profile differs")
        baseline_host = records["baseline-handshake"]["platform"]
        if baseline_host != records["candidate-handshake"]["platform"]:
            raise SystemExit(f"{target}: baseline and candidate ran on different hosts")
        for role, version in (("baseline", "0.35.0"), ("candidate", current_version)):
            handshake = records[f"{role}-handshake"]
            artifact = comparison[f"{role}_sha256"]
            if handshake.get("target") != target or handshake.get("version") != version:
                raise SystemExit(f"{target}/{role}: worker admitted a different version/target")
            for kind in ("handshake", "semantic", "tui", "terminal-load", "lsp", "transfer"):
                sample = records[f"{role}-{kind}"]
                if (sample.get("binary_sha256") != artifact
                        or sample.get("binary_bytes", sample.get("bytes")) != handshake["bytes"]
                        or (kind in ("semantic", "lsp", "transfer")
                            and sample.get("binary_version") != version)
                        or (kind != "handshake" and sample.get("measured_requests") != 64)
                        or sample.get("warmup_requests", sample.get("warmup_runs")) != 8):
                    raise SystemExit(f"{target}/{role}/{kind}: artifact or warmup binding differs")
            checked_samples([row["ready_ms"] for row in handshake["warm_runs"]],
                            handshake["ready_ms"], f"{target}/{role}/handshake")
            for kind, metric, compared in (
                ("semantic", "input_to_view_ms", "semantic"),
                ("tui", "input_to_grid_ms", "tui"),
                ("terminal-load", "output_to_grid_ms", "terminal_load"),
            ):
                sample = records[f"{role}-{kind}"]
                checked_samples(sample["raw_ms"], sample[metric], f"{target}/{role}/{kind}")
                if comparison["observed"][f"{role}_{compared}_ms"] != sample[metric]:
                    raise SystemExit(f"{target}/{role}/{kind}: comparison differs from raw frames")
            semantic = records[f"{role}-semantic"]
            lsp = records[f"{role}-lsp"]
            if (semantic["fixture"]["worker_count_after_open"] != 1
                    or semantic["platform"] != baseline_host or lsp["platform"] != baseline_host
                    or lsp["fixture"]["worker_count"] != 1 or lsp["measured_requests"] != 64):
                raise SystemExit(f"{target}/{role}: editor/LSP did not use one real worker/host")
            for raw, summary in (("request_raw_ms", "request_ms"),
                                 ("sync_request_raw_ms", "sync_request_ms")):
                checked_samples(lsp[raw], lsp[summary], f"{target}/{role}/lsp/{raw}")
            transfer = records[f"{role}-transfer"]
            fixture = transfer["fixture"]
            if (transfer["measured_requests"] != 64 or fixture["bytes"] != 17_280_000
                    or fixture["source_sha256"] != SOURCE_SHA256
                    or fixture["geometry"] != [120, 40] or fixture["lines"] != 10_000
                    or fixture["rss_peak_kind"] != (
                        "observed current RSS, not a kernel peak" if target.endswith("darwin")
                        else "kernel VmHWM")
                    or fixture["worker_count_after_each_open"] != 1
                    or transfer["platform"] != baseline_host
                    or any(row["workers_after_open"] != 1 for row in transfer["raw"])
                    or any(row["workers_after_exit"] != int(row["worker_post_exit_state"] != "-")
                           for row in transfer["raw"])
                    or (role == "candidate" and any(row["workers_after_exit"] != 0
                                                   for row in transfer["raw"]))):
                raise SystemExit(f"{target}/{role}: transfer fixture or worker retirement differs")
            for metric in METRICS:
                checked_samples([row[metric] for row in transfer["raw"]],
                                transfer["summary"][metric], f"{target}/{role}/{metric}")
        control = records["candidate-control"]
        if (control["binary_sha256"] != comparison["candidate_sha256"]
                or control["platform"] != baseline_host
                or control.get("warmup_requests") != 8 or control.get("measured_requests") != 64
                or records["baseline-lsp"]["analyzer"] != records["candidate-lsp"]["analyzer"]):
            raise SystemExit(f"{target}: control artifact or language-server version differs")
        checked_samples(control["raw_ms"], control["roundtrip_ms"], f"{target}/control")
        artifacts[target] = comparison["candidate_sha256"]
    return {"sha256": digest(root, REPORT), "methods": methods, "artifacts": artifacts,
            "validator_sha256": digest(root, "verification/native_evidence.py")}
