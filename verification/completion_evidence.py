"""0059 exact-source measurement binding; historical worker timings stay separate."""
from __future__ import annotations

import hashlib
import json
import math
import tomllib
from pathlib import Path

from bench_worker import percentiles

ROOT = Path(__file__).resolve().parent.parent
REPORT = "verification/measurements/0059-linux-x86-completion.json"
BEFORE = "verification/measurements/0059-linux-x86-typing-before.json"
BEFORE_SHA256 = "7f16ce60bcbf26e507718d694457b448b483ef26baf73757b1a76594ac561829"
METHODS = (
    "verification/qualify_completion.py",
    "verification/bench_completion.py",
    "verification/completion_driver.py",
    "verification/completion_server.py",
    "verification/completion_capture.py",
    "verification/bench_ui_input_frame.py",
    "verification/bench_worker.py",
)
CASES = {
    "disabled", "manual", "automatic", "large", "line_1mib", "unique_words",
    "hold", "ignore", "resolve", "oversized", "oversized-frame", "reopen", "acceptance",
}


def digest(path: Path) -> str:
    with path.open("rb") as stream:
        return hashlib.file_digest(stream, "sha256").hexdigest()


def source_files(root: Path = ROOT) -> dict:
    # Includes embedded resources, tests and build scripts as well as Rust;
    # no selective allow-list can omit a newly added runtime input.
    paths = {"Cargo.toml", "Cargo.lock", "Dockerfile", ".github/scripts/install-zig.sh"}
    paths.update(str(path.relative_to(root)) for path in (root / "crates").rglob("*") if path.is_file())
    config = root / ".cargo"
    if config.is_dir():
        paths.update(str(path.relative_to(root)) for path in config.rglob("*") if path.is_file())
    return {path: digest(root / path) for path in sorted(paths)}


def source_identity(root: Path = ROOT) -> dict:
    files = source_files(root)
    encoded = json.dumps(files, sort_keys=True, separators=(",", ":")).encode()
    return {"sha256": hashlib.sha256(encoded).hexdigest(), "file_count": len(files),
            "cargo_lock_sha256": files["Cargo.lock"]}


def method_hashes(root: Path = ROOT) -> dict:
    return {path: digest(root / path) for path in METHODS}


def checked_samples(raw: list, summary: dict, count: int, label: str) -> None:
    if (not isinstance(raw, list) or len(raw) != count
            or not all(type(value) in (int, float) and math.isfinite(value) and value >= 0 for value in raw)
            or summary != percentiles(raw)):
        raise SystemExit(f"{label}: summary does not bind {count} finite raw observations")


def validate_report(report: dict, root: Path = ROOT) -> None:
    version = tomllib.loads((root / "Cargo.toml").read_text())["workspace"]["package"]["version"]
    if (report.get("schema") != 1 or report.get("release") != "0059"
            or report.get("source") != source_identity(root)
            or report.get("methods") != method_hashes(root)
            or report.get("binary_version") != version
            or len(report.get("binary_sha256", "")) != 64
            or report.get("binary_bytes", 0) <= 0):
        raise SystemExit("completion observations do not bind the current source, methods and artifact")
    profile = report["profile"]
    if profile not in ("release-musl-stripped", "release-gnu-native", "release-macos-native"):
        raise SystemExit("development observations cannot qualify release performance")
    typing = report["typing"]
    if (typing["binary_sha256"] != report["binary_sha256"]
            or typing["binary_version"] != version or typing["binary_bytes"] != report["binary_bytes"]
            or typing["platform"] != report["platform"] | {"profile": profile}
            or typing["warmup_requests"] != 8 or typing["measured_requests"] != 64
            or typing["fixture"]["geometry"] != [120, 40]
            or typing["fixture"]["lines"] != 10000
            or typing["fixture"]["worker_count_after_open"] != 1):
        raise SystemExit("completion typing observation used a different artifact or fixture")
    checked_samples(typing["raw_ms"], typing["input_to_view_ms"], 64, "typing")
    pressure = report["pressure"]
    if (pressure["profile"] != profile or pressure["platform"] != report["platform"]
            or pressure["iterations"] != 32 or set(pressure["cases"]) != CASES):
        raise SystemExit("completion pressure scenarios or native profile are incomplete")
    for name, case in pressure["cases"].items():
        expected = (64 if name in ("disabled", "manual", "automatic", "large", "line_1mib", "unique_words")
                    else 65 if name in ("hold", "ignore") else 32 if name == "reopen"
                    else 1 if name == "resolve" else 0)
        if expected:
            checked_samples(case["raw_ms"], case["input_to_semantic_view_ms"], expected, name)
        elif case["raw_ms"] or case["input_to_semantic_view_ms"] is not None:
            raise SystemExit(f"{name}: a functional journey invented timing samples")
        threads = case["completion_threads_high_water"]
        if (case["publication_charged_high_water_bytes"] > 24 * 1024 * 1024
                or (threads is not None and threads > 1)
                or case["owned_worker_count"] != 1
                or (profile != "release-macos-native" and (threads is None or case["owned_workers_exited"] != 1))):
            raise SystemExit(f"{name}: completion/native ownership bounds failed")
        if "wire" in case and case["wire"]["counts"]["physical_high_water"] > 2:
            raise SystemExit(f"{name}: physical completion ownership exceeded two sent requests")
        work = case["word_work_last_observed"]
        if work and (work["indexed_words"] > 65536 or work["indexed_word_bytes"] > 4 * 1024 * 1024
                     or work["retirement_high_water_bytes"] > 72 * 1024 * 1024):
            raise SystemExit(f"{name}: word index or source retirement exceeded its ownership budget")
        if name in ("manual", "automatic", "large", "line_1mib", "unique_words"):
            if not work or work["builds"] != 1 or work["scanned_bytes"] > case["source_bytes"] + 64 * 1024:
                raise SystemExit(f"{name}: same-location typing restarted or rescanned the source index")
    cases = pressure["cases"]
    if (cases["disabled"]["completion_threads_high_water"] not in (None, 0)
            or cases["large"]["source_bytes"] < 16 * 1024 * 1024
            or cases["line_1mib"]["source_bytes"] < 1024 * 1024
            or cases["unique_words"]["word_work_last_observed"]["indexed_words"] != 65536
            or cases["ignore"]["wire"]["counts"]["queries"] != 2
            or len(cases["ignore"]["wire"]["held"]) != 2
            or cases["reopen"]["source_switches"] != 32):
        raise SystemExit("completion pressure did not exercise the declared boundary conditions")
    acceptance = cases["acceptance"]
    if (not all(acceptance[field] for field in ("opaque_data_roundtripped", "primary_and_import_undo_together", "invalid_formatter_did_not_mutate"))
            or acceptance["wire"]["counts"]["formats"] != 1
            or acceptance["wire"]["counts"]["resolves"] != 1):
        raise SystemExit("completion acceptance did not exercise resolve, import, invalid coordinates and undo")
    stages = report["stages"]
    if (stages["version"] != version or stages["samples_per_case"] != 128
            or stages["geometry"] != [120, 40]
            or {case["scenario"] for case in stages["cases"]} != {"disabled", "manual", "automatic", "cold_16mib", "line_1mib"}):
        raise SystemExit("stage-separated completion observations are incomplete")
    for case in stages["cases"]:
        for metric in ("enqueue_to_consume_ms", "consume_to_frame_ms", "dispatch_ms", "frame_ms"):
            summary = case[metric]
            checked_samples(summary["raw_ms"], {key: summary[key] for key in ("p50", "p95", "p99", "max")}, 128, f"{case['scenario']}/{metric}")
        retired = case["completion_after_retirement"]
        if retired["worker"] != "idle" or retired["publication"]["charged_bytes"] != 0:
            raise SystemExit("stage-separated completion observation retained native ownership at exit")
    capture = report["capture"]
    captured = capture["captures"]
    payloads = ("opaque_data", "detail", "documentation", "import")
    if (capture["binary_sha256"] != report["binary_sha256"]
            or not captured["native_free_full_replay"]
            or captured["metadata"]["startup_layout"] != "symlinked-source"
            or captured["full"]["startup_layout"] != "unrelated-git-cwd"
            or captured["project"]["startup_layout"] != "nested-project-consent"
            or captured["metadata"]["payloads_present"] != dict.fromkeys(payloads, False)
            or captured["full"]["payloads_present"] != dict.fromkeys(payloads, True)
            or captured["project"]["payloads_present"] != dict.fromkeys(payloads, True)):
        raise SystemExit("completion capture privacy or native-free replay was not established")


def validate(root: Path = ROOT) -> dict:
    report = json.loads((root / REPORT).read_text())
    validate_report(report, root)
    before = json.loads((root / BEFORE).read_text())
    after = report["typing"]
    if (digest(root / BEFORE) != BEFORE_SHA256
            or before["platform"] != after["platform"]
            or before["binary_version"] != "0.36.0"
            or before["qualified_source_commit"] != "a858e889113507ef50302286117193d9d4e99573"
            or before["binary_sha256"] != "5a9f826f889f2ce7a76b1a7f90cb3951ab7d8bede57cdc5b5033aa0d74247494"
            or before["method"] != after["method"]
            or before["warmup_requests"] != 8 or before["measured_requests"] != 64):
        raise SystemExit("typing comparison does not bind the immutable same-machine/profile baseline")
    checked_samples(before["raw_ms"], before["input_to_view_ms"], 64, "before typing")
    return {"report_sha256": digest(root / REPORT), "baseline_sha256": BEFORE_SHA256,
            "artifact_sha256": report["binary_sha256"], "source": report["source"],
            "methods": report["methods"], "validator_sha256": digest(Path(__file__)),
            "typing_before_ms": before["input_to_view_ms"], "typing_after_ms": after["input_to_view_ms"]}
