#!/usr/bin/env python3
"""Collect 0059 native completion evidence from one immutable production artifact.

The build description is supplied by the invoking build lane. Measurements bind
all Rust/build inputs and method files, not the later documentation/evidence
commit. The final freeze separately requires an exact clean commit/tree.
"""
from __future__ import annotations

import argparse
import json
import os
import subprocess
import tempfile
from pathlib import Path

from bench_completion import measure as pressure
from bench_ui_input_frame import measure as typing
from completion_capture import check as capture
from completion_evidence import digest, method_hashes, source_identity, validate_report


def collect(binary: Path, profile: str, build: str) -> dict:
    source = source_identity()
    methods = method_hashes()
    artifact = digest(binary)
    typed = typing(binary, 8, 64, profile)
    stressed = pressure(binary, profile)
    with tempfile.TemporaryDirectory(prefix="strop-completion-stages-") as directory:
        root = Path(directory)
        for folder in ("home", "config", "state", "cache"):
            (root / folder).mkdir()
        environment = dict(os.environ, HOME=str(root / "home"),
            XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "state"),
            XDG_CACHE_HOME=str(root / "cache"), STROP_LOG="")
        result = subprocess.run([str(binary), "--bench", "completion"], cwd=root,
            env=environment, capture_output=True, text=True, timeout=120)
        if result.returncode:
            raise RuntimeError(f"completion stage observation failed: {result.stderr}")
        prefix = "STROP_COMPLETION_STAGES="
        records = [line[len(prefix):] for line in result.stdout.splitlines() if line.startswith(prefix)]
        if len(records) != 1:
            raise RuntimeError("completion stage observation did not emit exactly one result")
        stages = json.loads(records[0])
    captured = capture(binary)
    if source_identity() != source or method_hashes() != methods or digest(binary) != artifact:
        raise RuntimeError("source, methods or binary changed during qualification")
    report = {"schema": 1, "release": "0059", "source": source, "methods": methods,
        "binary_sha256": artifact, "binary_bytes": binary.stat().st_size,
        "binary_version": typed["binary_version"], "profile": profile,
        "build": build, "platform": stressed["platform"], "typing": typed,
        "pressure": stressed, "stages": stages, "capture": captured}
    validate_report(report)
    return report


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    parser.add_argument("--profile", choices=("release-musl-stripped", "release-gnu-native", "release-macos-native"), required=True)
    parser.add_argument("--build", required=True, help="actual build command/toolchain lane, not an inferred profile")
    parser.add_argument("--out", type=Path, required=True)
    args = parser.parse_args()
    report = collect(args.binary.resolve(), args.profile, args.build)
    args.out.parent.mkdir(parents=True, exist_ok=True)
    args.out.write_text(json.dumps(report, indent=2, sort_keys=True) + "\n")
    print(json.dumps({"output": str(args.out), "binary_sha256": report["binary_sha256"],
        "source": report["source"], "typing_ms": report["typing"]["input_to_view_ms"],
        "pressure_cases": len(report["pressure"]["cases"]),
        "native_free_full_replay": report["capture"]["captures"]["native_free_full_replay"]}, indent=2))


if __name__ == "__main__":
    main()
