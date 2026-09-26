#!/usr/bin/env python3
"""Measure native release binaries through the real framed worker handshake.

Run this script on each release target with its own native executable:
  python3 -B verification/bench_worker.py --binary target/release/strop \
      --protocol 2 --version 0.35.0 --target x86_64-unknown-linux-musl

Reports raw samples and nearest-rank p50/p95/p99/max, not a speedup claim.
The first launch is reported separately; subsequent runs are warmed local
process launch + welcome, not SSH deployment, input-to-frame or durability.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import math
import os
import platform
import subprocess
import time
from pathlib import Path


def frame(reader) -> dict:
    header = bytearray()
    while not header.endswith(b"\r\n\r\n"):
        octet = reader.read(1)
        if not octet:
            raise RuntimeError("worker closed before the welcome frame")
        header.extend(octet)
        if len(header) > 8192:
            raise RuntimeError("worker response header exceeded 8192 bytes")
    fields = header[:-4].decode("ascii").split("\r\n")
    lengths = [value.strip() for field, sep, value in (line.partition(":") for line in fields)
               if sep and field.strip().lower() == "content-length"]
    if len(lengths) != 1 or not lengths[0].isdecimal():
        raise RuntimeError("worker response has no unique numeric Content-Length")
    length = int(lengths[0])
    if length > 1024 * 1024:
        raise RuntimeError("worker response exceeds the control-frame bound")
    body = reader.read(length)
    if len(body) != length or not body.startswith(b"\0"):
        raise RuntimeError("worker response was truncated or not control data")
    return json.loads(body[1:])


def process_state(pid: int) -> tuple[int, int]:
    if platform.system() == "Linux":
        status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
        fields = dict(line.split(":", 1) for line in status.splitlines() if ":" in line)
        return int(fields["VmRSS"].split()[0]), int(fields["Threads"].strip())
    if platform.system() == "Darwin":
        observed = subprocess.check_output(
            ["ps", "-p", str(pid), "-o", "rss=", "-o", "thcount="], text=True
        ).split()
        if len(observed) == 2:
            return int(observed[0]), int(observed[1])
    raise RuntimeError("worker RSS/thread sampling needs Linux /proc or macOS ps")


def worker_processes(pid: int) -> list[int]:
    if platform.system() == "Linux":
        children = Path(f"/proc/{pid}/task/{pid}/children").read_text().split()
        return [int(child) for child in children if b"--worker-stdio" in
                Path(f"/proc/{child}/cmdline").read_bytes()]
    if platform.system() == "Darwin":
        processes = subprocess.check_output(
            ["ps", "-axo", "pid=", "-o", "ppid=", "-o", "command="], text=True
        )
        return [int(parts[0]) for line in processes.splitlines()
                if len(parts := line.split(maxsplit=2)) == 3
                and parts[1] == str(pid) and "--worker-stdio" in parts[2]]
    raise RuntimeError("worker process sampling needs Linux /proc or macOS ps")


def cpu_model() -> str:
    if platform.system() == "Linux":
        return next(
            (line.split(":", 1)[1].strip()
             for line in Path("/proc/cpuinfo").read_text(encoding="utf-8").splitlines()
             if line.startswith("model name")), "unreported"
        )
    if platform.system() == "Darwin":
        return subprocess.check_output(
            ["sysctl", "-n", "machdep.cpu.brand_string"], text=True
        ).strip()
    raise RuntimeError("benchmark supports only Linux and macOS native targets")


def native_profile(target: str | None = None) -> str:
    if platform.system() == "Linux":
        return "release-gnu-native" if target and target.endswith("-gnu") else "release-musl-stripped"
    if platform.system() == "Darwin":
        return "release-macos-native"
    raise RuntimeError("benchmark supports only Linux and macOS native targets")


def sample(args: argparse.Namespace) -> dict:
    hello = json.dumps({
        "type": "hello", "protocol": args.protocol,
        "client": {"name": "worker-benchmark", "version": args.version,
                   "build": None, "target": args.target},
    }, separators=(",", ":")).encode("utf-8")
    body = b"\0" + hello
    encoded = b"Content-Length: %d\r\n\r\n" % len(body) + body
    started = time.perf_counter_ns()
    child = subprocess.Popen(
        [args.binary, "--worker-stdio"],
        stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
    )
    try:
        child.stdin.write(encoded)
        child.stdin.flush()
        welcome = frame(child.stdout)
        ready = time.perf_counter_ns()
        if welcome.get("type") != "welcome" or welcome.get("protocol") != args.protocol:
            raise RuntimeError(f"worker did not admit the requested protocol: {welcome}")
        identity = welcome["worker"]
        if identity["target"] != args.target or identity["version"] != args.version:
            raise RuntimeError(f"worker reported the wrong release/target: {identity}")
        rss_kib, threads = process_state(child.pid)
        child.stdin.close()
        if child.wait(timeout=10) != 0:
            raise RuntimeError(f"worker exited nonzero: {child.stderr.read()[-512:]!r}")
        return {
            "ready_ms": round((ready - started) / 1_000_000, 3),
            "rss_kib": rss_kib, "threads": threads,
        }
    finally:
        if child.poll() is None:
            child.kill()
            child.wait()


def percentiles(values: list[float]) -> dict:
    ordered = sorted(values)
    return {
        f"p{n}": ordered[math.ceil(n * len(ordered) / 100) - 1]
        for n in (50, 95, 99)
    } | {"max": ordered[-1]}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True)
    parser.add_argument("--protocol", required=True, type=int)
    parser.add_argument("--version", required=True)
    parser.add_argument("--target", required=True)
    parser.add_argument("--warmup", type=int, default=4)
    parser.add_argument("--iterations", type=int, default=64)
    args = parser.parse_args()
    if args.iterations < 1 or args.warmup < 0:
        parser.error("iterations must be positive and warmup nonnegative")
    binary = Path(args.binary)
    if not binary.is_file() or not os.access(binary, os.X_OK):
        parser.error("--binary must be an executable release artifact")
    first = sample(args)
    for _ in range(args.warmup):
        sample(args)
    samples = [sample(args) for _ in range(args.iterations)]
    with binary.open("rb") as artifact:
        digest = hashlib.file_digest(artifact, "sha256").hexdigest()
    cpu = cpu_model()
    print(json.dumps({
        "binary_sha256": digest,
        "bytes": binary.stat().st_size,
        "protocol": args.protocol, "version": args.version, "target": args.target,
        "first_observed": first, "warmup_runs": args.warmup,
        "platform": {
            "machine": platform.machine(), "kernel": platform.release(),
            "cpu": cpu, "profile": native_profile(args.target),
        },
        "warm_runs": samples,
        "ready_ms": percentiles([item["ready_ms"] for item in samples]),
        "rss_kib": percentiles([item["rss_kib"] for item in samples]),
        "threads": percentiles([item["threads"] for item in samples]),
    }, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
