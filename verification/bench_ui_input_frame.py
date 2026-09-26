#!/usr/bin/env python3
"""Measure one-key input to the real framed UI view on Linux or macOS.

The matching release binary serves --ui-stdio; an admitted action opens a
10k-line file through the local worker, then individual committed text inputs
publish semantic view frames at 120x40. This is NOT terminal-cell presentation,
SSH/container deployment, LSP latency or a pre-worker speedup claim.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import selectors
import subprocess
import tempfile
import time
from pathlib import Path

from bench_worker import cpu_model, native_profile, percentiles, process_state, worker_processes

MAX_HEADER = 8192
MAX_BODY = 32 * 1024 * 1024


def encoded(message: dict) -> bytes:
    body = json.dumps(message, separators=(",", ":")).encode("utf-8")
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def key(code: str) -> dict:
    code_value = {"Char": code} if len(code) == 1 else code
    return {"action": "input", "data": {"Key": {
        "code": code_value,
        "modifiers": {"shift": False, "control": False, "alt": False,
                      "super_key": False, "hyper": False, "meta": False},
        "kind": "Press",
        "state": {"keypad": False, "caps_lock": False, "num_lock": False},
    }}}


class Peer:
    def __init__(self, child: subprocess.Popen):
        self.child = child
        self.pending = bytearray()
        self.incarnation = None
        self.generation = None
        self.geometry: dict = {}
        self.applied = 0
        self.sequence = 0
        self.panes: list[dict] = []
        self.state: dict = {}

    def send(self, message: dict) -> int:
        request = encoded(message)
        self.child.stdin.write(request)
        self.child.stdin.flush()
        return len(request)

    def receive(self) -> tuple[dict, int, int]:
        deadline = time.monotonic() + 30
        while True:
            end = self.pending.find(b"\r\n\r\n")
            if end >= 0:
                if end + 4 > MAX_HEADER:
                    raise RuntimeError("UI frame header exceeded its bound")
                headers = self.pending[:end].decode("ascii").split("\r\n")
                lengths = [value.strip() for header in headers
                           for name, sep, value in [header.partition(":")]
                           if sep and name.lower() == "content-length"]
                if len(lengths) != 1 or not lengths[0].isdecimal():
                    raise RuntimeError("UI frame has no unique numeric length")
                length = int(lengths[0])
                if length > MAX_BODY:
                    raise RuntimeError("UI frame exceeds the bounded body")
                complete = end + 4 + length
                if len(self.pending) >= complete:
                    body = bytes(self.pending[end + 4:complete])
                    del self.pending[:complete]
                    return json.loads(body), complete, time.perf_counter_ns()
            if len(self.pending) > MAX_HEADER and end < 0:
                raise RuntimeError("UI frame header did not terminate")
            remaining = deadline - time.monotonic()
            if remaining <= 0:
                raise TimeoutError("UI backend did not publish a frame within 30s")
            selector = selectors.DefaultSelector()
            try:
                selector.register(self.child.stdout, selectors.EVENT_READ)
                if not selector.select(remaining):
                    raise TimeoutError("UI backend stalled before its next frame")
            finally:
                selector.close()
            chunk = os.read(self.child.stdout.fileno(), 8192)
            if not chunk:
                raise RuntimeError("UI backend closed its protocol pipe")
            self.pending.extend(chunk)

    def view(self, message: dict) -> None:
        if message["type"] == "snapshot":
            if message["incarnation"] != self.incarnation:
                raise RuntimeError("UI backend incarnation changed")
            view = message["view"]
            self.geometry = view["geometry"]
            self.generation = view["generation"]
            self.panes = view["panes"]
            self.state = view["state"]
            return
        if message["type"] != "delta" or message["incarnation"] != self.incarnation:
            raise RuntimeError(f"unexpected UI view: {message['type']}")
        delta = message["delta"]
        if delta["base"] != self.generation or len(delta["panes"]) != len(self.panes):
            raise RuntimeError("UI delta does not address the published view")
        self.generation = delta["generation"]
        if delta["geometry"] is not None:
            self.geometry = delta["geometry"]
        for index, pane in enumerate(delta["panes"]):
            if pane != "unchanged":
                self.panes[index] = pane["changed"]
        if delta["state"] is not None:
            self.state = delta["state"]

    def act(self, actions: list[dict]) -> tuple[float, int, int, int]:
        self.sequence += 1
        seq = self.sequence
        base = {"incarnation": self.incarnation, "generation": self.generation}
        started = time.perf_counter_ns()
        request_bytes = self.send({"type": "act", "seq": seq,
                                   "base": base, "actions": actions})
        ack_bytes = 0
        ack_generation = None
        while True:
            message, size, received_at = self.receive()
            kind = message["type"]
            if kind in ("snapshot", "delta"):
                self.view(message)
                if ack_generation is not None and self.generation == ack_generation:
                    return ((received_at - started) / 1_000_000, request_bytes,
                            ack_bytes, size)
            elif kind == "ack":
                if message["seq"] != seq or message["outcome"]["outcome"] != "applied":
                    raise RuntimeError(f"UI action was not applied: {message}")
                if message["outcome"]["applied"] != self.applied + 1:
                    raise RuntimeError("UI action sequence did not advance once")
                self.applied += 1
                ack_generation = message["outcome"]["generation"]
                ack_bytes = size
            else:
                raise RuntimeError(f"unexpected UI response to input: {message}")

    def wait_file(self) -> None:
        for _ in range(256):
            if any(line.startswith("line 00000") for pane in self.panes
                   for line in pane["lines"]):
                return
            message, _, _ = self.receive()
            if message["type"] not in ("snapshot", "delta"):
                raise RuntimeError(f"file open was not a view: {message}")
            self.view(message)
        raise RuntimeError("worker-opened file did not reach the UI view")


def measure(binary: Path, warmup: int, iterations: int, profile: str) -> dict:
    if platform.system() not in ("Linux", "Darwin"):
        raise ValueError("worker process evidence requires Linux /proc or macOS ps")
    if (platform.system() == "Darwin") != (profile == "release-macos-native"):
        raise ValueError("benchmark profile does not match the native operating system")
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("--binary must be the executable release artifact")
    identity = subprocess.check_output([str(binary), "--version"], text=True).strip().split()
    if len(identity) != 2 or identity[0] != "strop":
        raise ValueError("the benchmark artifact did not report a Strop version")
    binary_version = identity[1]
    with tempfile.TemporaryDirectory(prefix="strop-ui-frame-") as name:
        root = Path(name)
        for folder in ("home", "config", "state"):
            (root / folder).mkdir()
        (root / "notes.txt").write_text(
            "\n".join(f"line {line:05d} worker frame fixture" for line in range(10_000)))
        env = dict(os.environ, HOME=str(root / "home"),
                   XDG_CONFIG_HOME=str(root / "config"),
                   XDG_STATE_HOME=str(root / "state"), STROP_LOG="")
        with tempfile.TemporaryFile() as errors:
            child = subprocess.Popen([str(binary), "--ui-stdio"], cwd=root,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=errors, env=env)
            try:
                peer = Peer(child)
                peer.send({"type": "hello", "protocol": 1,
                           "client": {"name": "ui-frame-benchmark", "version": binary_version},
                           "capabilities": {"clipboard_write": False}})
                welcome, _, _ = peer.receive()
                if welcome["type"] != "welcome" or welcome["protocol"] != 1:
                    raise RuntimeError(f"UI backend refused its handshake: {welcome}")
                peer.incarnation = welcome["backend"]["incarnation"]
                first, _, _ = peer.receive()
                peer.view(first)
                peer.sequence += 1
                peer.send({"type": "viewport", "seq": peer.sequence,
                           "columns": 120, "rows": 40})
                viewport_generation = None
                for _ in range(256):
                    viewport, _, _ = peer.receive()
                    if viewport["type"] in ("snapshot", "delta"):
                        peer.view(viewport)
                    elif viewport["type"] == "ack":
                        if viewport["seq"] != peer.sequence or viewport["outcome"]["outcome"] != "applied":
                            raise RuntimeError(f"viewport was not admitted: {viewport}")
                        peer.applied = viewport["outcome"]["applied"]
                        viewport_generation = viewport["outcome"]["generation"]
                    else:
                        raise RuntimeError(f"unexpected viewport response: {viewport}")
                    if (viewport_generation is not None and peer.generation == viewport_generation
                            and peer.geometry == {"columns": 120, "rows": 40}):
                        break
                else:
                    raise RuntimeError("admitted viewport never published its geometry")
                peer.act([key(char) for char in ":e notes.txt"] + [key("Enter")])
                peer.wait_file()
                peer.act([key(char) for char in ":5000"] + [key("Enter")])
                peer.act([key("A")])
                if peer.state.get("mode") != "INSERT":
                    raise RuntimeError("UI did not enter insert mode")
                workers = worker_processes(child.pid)
                if len(workers) != 1:
                    raise RuntimeError(f"expected one live local worker, got {workers}")
                editor_rss, editor_threads = process_state(child.pid)
                worker_rss, worker_threads = process_state(workers[0])
                samples = []
                for index in range(warmup + iterations):
                    elapsed, sent, ack, view = peer.act([
                        {"action": "input", "data": {"Text": "x"}}])
                    expected = "line 04999 worker frame fixture" + "x" * (index + 1)
                    if not any(expected == line for pane in peer.panes
                               for line in pane["lines"]):
                        raise RuntimeError("input did not modify the expected visible worker file line")
                    if index >= warmup:
                        samples.append((round(elapsed, 3), sent, ack, view))
                peer.sequence += 1
                peer.send({"type": "shutdown", "seq": peer.sequence})
                shutdown_acked = False
                for _ in range(256):
                    response, _, _ = peer.receive()
                    if response["type"] in ("snapshot", "delta"):
                        peer.view(response)
                    elif response["type"] == "ack":
                        if (response["seq"] != peer.sequence
                                or response["outcome"]["outcome"] != "applied"
                                or response["outcome"]["applied"] != peer.applied):
                            raise RuntimeError(f"shutdown was not admitted: {response}")
                        shutdown_acked = True
                    elif response["type"] == "bye":
                        if not shutdown_acked or response["reason"] != "requested":
                            raise RuntimeError(f"UI exited without authorized shutdown: {response}")
                        break
                    else:
                        raise RuntimeError(f"unexpected UI shutdown response: {response}")
                else:
                    raise RuntimeError("UI shutdown did not settle after 256 frames")
                if child.wait(timeout=15) != 0:
                    raise RuntimeError("UI backend exited nonzero")
                with binary.open("rb") as artifact:
                    digest = hashlib.file_digest(artifact, "sha256").hexdigest()
                return {
                    "binary_sha256": digest, "binary_bytes": binary.stat().st_size,
                    "binary_version": binary_version,
                    "platform": {"machine": platform.machine(),
                                 "kernel": platform.release(),
                                 "cpu": cpu_model(),
                                 "profile": profile},
                    "fixture": {"lines": 10_000, "geometry": [120, 40],
                                "input": "one committed text character",
                                "worker_count_after_open": len(workers),
                                "editor_rss_kib_after_open": editor_rss,
                                "editor_threads_after_open": editor_threads,
                                "worker_rss_kib_after_open": worker_rss,
                                "worker_threads_after_open": worker_threads},
                    "method": "framed UI admitted input write+flush through complete semantic view frame",
                    "warmup_requests": warmup, "measured_requests": iterations,
                    "input_to_view_ms": percentiles([value[0] for value in samples]),
                    "raw_ms": [value[0] for value in samples],
                    "request_bytes": sum(value[1] for value in samples),
                    "ack_bytes": sum(value[2] for value in samples),
                    "view_bytes": sum(value[3] for value in samples),
                }
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()
                if child.returncode != 0:
                    errors.seek(0)
                    detail = errors.read(1024)
                    if detail:
                        print(f"UI backend diagnostic: {detail!r}", file=os.sys.stderr)


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", required=True, type=Path)
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
        parser.exit(1, f"UI frame measurement failed: {error}\n")
    print(json.dumps(result, indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
