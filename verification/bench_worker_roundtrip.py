#!/usr/bin/env python3
"""Measure warm real-worker control envelope write → complete response frame.

This is a scoped Linux local IPC measurement, not editor input→render,
an LSP request, SSH/container deployment, or terminal throughput. The
worker is one real process; no Python code is on its serving path. Run
with the named binary/profile on the same native host as the other
measurements; the method requires /proc for process samples.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import subprocess
import time
from pathlib import Path

from bench_worker import frame, percentiles, process_state


def encoded(message: dict) -> bytes:
    body = b"\0" + json.dumps(message, separators=(",", ":")).encode("utf-8")
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def roundtrip(child: subprocess.Popen, session: dict, request_id: int) -> tuple[float, int]:
    request = encoded({
        "type": "request", "session": session,
        "id": request_id, "body": {"op": "health"},
    })
    started = time.perf_counter_ns()
    child.stdin.write(request)
    child.stdin.flush()
    result = frame(child.stdout)
    elapsed = time.perf_counter_ns() - started
    if result != {
        "type": "result", "id": request_id,
        "outcome": {"outcome": "healthy"},
    }:
        raise RuntimeError(f"unexpected result for health {request_id}: {result}")
    return round(elapsed / 1_000_000, 3), len(request)


def measure(args: argparse.Namespace) -> dict:
    if platform.system() != "Linux":
        raise ValueError("warm worker process measurements require Linux /proc")
    binary = Path(args.binary)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("--binary must name an executable release artifact")
    with binary.open("rb") as artifact:
        digest = hashlib.file_digest(artifact, "sha256").hexdigest()
    child = subprocess.Popen(
        [str(binary), "--worker-stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    try:
        hello = encoded({
            "type": "hello", "protocol": args.protocol,
            "client": {
                "name": "worker-roundtrip-benchmark", "version": args.version,
                "build": None, "target": args.target,
            },
        })
        child.stdin.write(hello)
        child.stdin.flush()
        welcome = frame(child.stdout)
        if (welcome.get("type") != "welcome"
                or welcome.get("protocol") != args.protocol
                or welcome["worker"]["version"] != args.version
                or welcome["worker"]["target"] != args.target):
            raise RuntimeError(f"worker rejected requested identity: {welcome}")
        session = welcome["session"]
        rss_kib, threads = process_state(child.pid)
        cpu = next(
            (line.split(":", 1)[1].strip()
             for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines()
             if line.startswith("model name")),
            "unreported",
        )
        for request_id in range(1, args.warmup + 1):
            roundtrip(child, session, request_id)
        samples = [
            roundtrip(child, session, request_id)
            for request_id in range(args.warmup + 1, args.warmup + args.iterations + 1)
        ]
        child.stdin.write(encoded({"type": "shutdown", "session": session}))
        child.stdin.flush()
        if frame(child.stdout).get("type") != "bye" or child.wait(timeout=10) != 0:
            raise RuntimeError("worker did not shut down cleanly after measurement")
        return {
            "binary_sha256": digest, "binary_bytes": binary.stat().st_size,
            "protocol": args.protocol, "version": args.version, "target": args.target,
            "platform": {
                "machine": platform.machine(), "kernel": platform.release(),
                "cpu": cpu, "profile": args.profile,
            },
            "method": "warm single-process health write+flush to complete control response",
            "warmup_requests": args.warmup, "measured_requests": args.iterations,
            "request_bytes": {"min": min(length for _, length in samples),
                              "max": max(length for _, length in samples),
                              "total": sum(length for _, length in samples)},
            "worker": {"rss_kib_after_welcome": rss_kib, "threads_after_welcome": threads},
            "roundtrip_ms": percentiles([elapsed for elapsed, _ in samples]),
            "raw_ms": [elapsed for elapsed, _ in samples],
        }
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--protocol", required=True, type=int)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--profile", required=True)
    parser.add_argument("--warmup", type=int, default=8)
    parser.add_argument("--iterations", type=int, default=64)
    args = parser.parse_args()
    if args.warmup < 0 or args.iterations < 1:
        parser.error("warmup must be nonnegative and iterations positive")
    try:
        result = measure(args)
    except (OSError, ValueError, RuntimeError, subprocess.TimeoutExpired) as error:
        parser.exit(1, f"measurement failed: {error}\n")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
