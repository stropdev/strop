#!/usr/bin/env python3
"""Measure a 17 MiB native-worker file open to visible editor UI and memory high-water.

Each fresh real editor opens the same on-disk 10k-line file through its worker,
paints the first line, navigates to and verifies the final line, then shuts
itself and its worker down. No fake wire or frontend: this is cold transfer,
not warm reuse, remote transfer, or a proof of retained memory bounds.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import signal
import subprocess
import tempfile
import time
from pathlib import Path

from bench_ui_input_frame import Peer, key
from bench_worker import cpu_model, native_profile, percentiles, process_state, worker_processes

LINES = 10_000
FILL = "v" * 1700


def high_water(pid: int) -> int:
    if platform.system() == "Linux":
        status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
        return next(int(line.split()[1]) for line in status.splitlines()
                    if line.startswith("VmHWM:"))
    # ps offers only current RSS on macOS; never call this an OS peak.
    return process_state(pid)[0]


def post_exit_worker_state(pid: int) -> str:
    if platform.system() == "Linux":
        try:
            status = Path(f"/proc/{pid}/status").read_text(encoding="utf-8")
        except FileNotFoundError:
            return "-"
        return next(line.split(":", 1)[1].strip().split()[0]
                    for line in status.splitlines() if line.startswith("State:"))
    observed = subprocess.run(["ps", "-p", str(pid), "-o", "stat="],
                              capture_output=True, text=True, check=False)
    return observed.stdout.strip().split()[0][:1] if observed.returncode == 0 else "-"


def visible(peer: Peer, marker: str) -> bool:
    return any(line.startswith(marker) for pane in peer.panes for line in pane["lines"])


def observe(binary: Path, root: Path, version: str) -> dict:
    env = dict(os.environ, HOME=str(root / "home"),
               XDG_CONFIG_HOME=str(root / "config"),
               XDG_STATE_HOME=str(root / "state"), STROP_LOG="")
    with tempfile.TemporaryFile() as errors:
        child = subprocess.Popen([str(binary), "--ui-stdio"], cwd=root,
                                 stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                 stderr=errors, env=env, start_new_session=True)
        worker = None
        worker_fd = None
        worker_state = "-"
        try:
            peer = Peer(child)
            peer.send({"type": "hello", "protocol": 1,
                       "client": {"name": "transfer-benchmark", "version": version},
                       "capabilities": {"clipboard_write": False}})
            welcome, _, _ = peer.receive()
            if welcome["type"] != "welcome" or welcome["protocol"] != 1:
                raise RuntimeError(f"the native editor refused its client: {welcome}")
            peer.incarnation = welcome["backend"]["incarnation"]
            frame, _, _ = peer.receive()
            peer.view(frame)
            peer.sequence += 1
            peer.send({"type": "viewport", "seq": peer.sequence,
                       "columns": 120, "rows": 40})
            acknowledged = False
            for _ in range(256):
                message, _, _ = peer.receive()
                if message["type"] in ("snapshot", "delta"):
                    peer.view(message)
                elif message["type"] == "ack":
                    if (message["seq"] != peer.sequence
                            or message["outcome"]["outcome"] != "applied"):
                        raise RuntimeError(f"the editor refused viewport: {message}")
                    peer.applied = message["outcome"]["applied"]
                    acknowledged = True
                else:
                    raise RuntimeError(f"unexpected viewport response: {message}")
                if acknowledged and peer.geometry == {"columns": 120, "rows": 40}:
                    break
            else:
                raise RuntimeError("viewport was not published")
            start = time.perf_counter_ns()
            peer.act([key(char) for char in ":e large.txt"] + [key("Enter")])
            for _ in range(256):
                if visible(peer, "line 00000 worker transfer"):
                    opened_at = time.perf_counter_ns()
                    break
                message, _, seen_at = peer.receive()
                if message["type"] not in ("snapshot", "delta"):
                    raise RuntimeError(f"unexpected worker file response: {message}")
                peer.view(message)
                if visible(peer, "line 00000 worker transfer"):
                    opened_at = seen_at
                    break
            else:
                raise RuntimeError(f"17 MiB worker file did not reach UI: {peer.state}")
            opened_ms = round((opened_at - start) / 1_000_000, 3)
            peer.act([key(char) for char in ":10000"] + [key("Enter")])
            if not visible(peer, "line 09999 worker transfer"):
                raise RuntimeError("large worker transfer omitted its final source line")
            workers = worker_processes(child.pid)
            if len(workers) != 1:
                raise RuntimeError(f"large file open retained {len(workers)} local workers")
            worker = workers[0]
            if platform.system() == "Linux":
                worker_fd = os.pidfd_open(worker)
            editor_rss, editor_threads = process_state(child.pid)
            worker_rss, worker_threads = process_state(worker)
            editor_peak, worker_peak = high_water(child.pid), high_water(worker)
            peer.sequence += 1
            peer.send({"type": "shutdown", "seq": peer.sequence})
            shutdown_ack = False
            for _ in range(256):
                message, _, _ = peer.receive()
                if message["type"] in ("snapshot", "delta"):
                    peer.view(message)
                elif message["type"] == "ack":
                    shutdown_ack = (message["seq"] == peer.sequence
                                    and message["outcome"]["outcome"] == "applied")
                elif message["type"] == "bye" and shutdown_ack:
                    break
                else:
                    raise RuntimeError(f"worker UI shutdown was not admitted: {message}")
            else:
                raise RuntimeError("native editor did not retire its worker session")
            if child.wait(timeout=15) != 0:
                raise RuntimeError("native editor exited nonzero after worker transfer")
            worker_state = post_exit_worker_state(worker)
            return {"open_to_view_ms": opened_ms,
                    "editor_rss_kib": editor_rss, "worker_rss_kib": worker_rss,
                    "editor_peak_kib": editor_peak, "worker_peak_kib": worker_peak,
                    "editor_threads": editor_threads, "worker_threads": worker_threads,
                    "workers_after_open": len(workers),
                    "workers_after_exit": int(worker_state != "-"),
                    "worker_post_exit_state": worker_state}
        finally:
            if child.poll() is None:
                os.killpg(child.pid, signal.SIGKILL)
                child.wait()
            if worker_fd is not None:
                # The pidfd pins this fixture's worker identity across
                # editor exit. Cleanup follows, never precedes, sampling.
                try:
                    signal.pidfd_send_signal(worker_fd, signal.SIGKILL)
                except ProcessLookupError:
                    pass
                os.close(worker_fd)
            elif worker is not None and platform.system() == "Darwin":
                try:
                    if (worker_state not in ("-", "Z")
                            and os.getpgid(worker) == child.pid):
                        os.killpg(child.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass
            if child.returncode != 0:
                errors.seek(0)
                detail = errors.read(1024)
                if detail:
                    print(f"worker transfer diagnostic: {detail!r}", file=os.sys.stderr)


def measure(binary: Path, warmup: int, iterations: int, profile: str) -> dict:
    if platform.system() not in ("Linux", "Darwin"):
        raise ValueError("process high-water measurement needs Linux or macOS")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("--binary must be the native release executable")
    identity = subprocess.check_output([str(binary), "--version"], text=True).strip().split()
    if len(identity) != 2 or identity[0] != "strop":
        raise ValueError("the benchmark artifact did not report a Strop version")
    with tempfile.TemporaryDirectory(prefix="strop-worker-transfer-") as name:
        root = Path(name)
        for folder in ("home", "config", "state"):
            (root / folder).mkdir()
        path = root / "large.txt"
        with path.open("w", encoding="utf-8") as target:
            for index in range(LINES):
                target.write(f"line {index:05d} worker transfer {FILL}\n")
        with path.open("rb") as contents:
            source_sha256 = hashlib.file_digest(contents, "sha256").hexdigest()
        source_bytes = path.stat().st_size
        for _ in range(warmup):
            observe(binary, root, identity[1])
        samples = [observe(binary, root, identity[1]) for _ in range(iterations)]
        with binary.open("rb") as artifact:
            digest = hashlib.file_digest(artifact, "sha256").hexdigest()
        keys = ("open_to_view_ms", "editor_rss_kib", "worker_rss_kib",
                "editor_peak_kib", "worker_peak_kib", "editor_threads",
                "worker_threads", "workers_after_exit")
        return {
            "binary_sha256": digest, "binary_bytes": binary.stat().st_size,
            "binary_version": identity[1],
            "platform": {"machine": platform.machine(), "kernel": platform.release(),
                         "cpu": cpu_model(), "profile": profile},
            "fixture": {"lines": LINES, "bytes": source_bytes,
                        "source_sha256": source_sha256, "geometry": [120, 40],
                        "worker_count_after_each_open": 1,
                        "rss_peak_kind": ("kernel VmHWM" if platform.system() == "Linux"
                                          else "observed current RSS, not a kernel peak"),
                        "retirement_scope": "immediate state after editor exit; fixture kills only its owned worker after observation"},
            "method": "file open to complete 120x40 UI frame, verify last line and observe post-exit worker",
            "warmup_requests": warmup, "measured_requests": iterations,
            "raw": samples,
            "summary": {key: percentiles([sample[key] for sample in samples]) for key in keys},
        }


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
    parser.add_argument("--out", type=Path)
    parser.add_argument("--warmup", type=int, default=8)
    parser.add_argument("--iterations", type=int, default=64)
    parser.add_argument("--profile", choices=("release-musl-stripped", "release-gnu-native",
                                             "release-macos-native"))
    args = parser.parse_args()
    if args.warmup < 0 or args.iterations < 1:
        parser.error("warmup must be nonnegative and iterations positive")
    try:
        result = measure(args.binary, args.warmup, args.iterations,
                         args.profile or native_profile())
    except (OSError, KeyError, ValueError, RuntimeError, TimeoutError,
            subprocess.TimeoutExpired) as error:
        parser.exit(1, f"worker transfer measurement failed: {error}\n")
    serialized = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out is None:
        print(serialized, end="")
    else:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(serialized, encoding="utf-8")
        print(f"native worker transfer measurements: {args.out} ({args.iterations} samples)")


if __name__ == "__main__":
    main()
