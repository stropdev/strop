#!/usr/bin/env python3
"""Measure real rust-analyzer definition replies and edit-to-reply via strop --ui-stdio.

A tiny on-disk Rust crate, the installed rust-analyzer, and the actual editor
handle each request. Only a definition landing in the original source counts;
an action acknowledgement or unchanged view is not an LSP reply. The combined
edit-to-definition sample bounds sync+request+presentation, not didChange alone.
"""

from __future__ import annotations

import argparse
import hashlib
import json
import os
import platform
import shutil
import subprocess
import tempfile
import time
from pathlib import Path

from bench_ui_input_frame import Peer, key
from bench_worker import cpu_model, native_profile, percentiles, worker_processes

SOURCE = "pub fn answer() -> u32 { 42 }\n\npub fn run() -> u32 { answer() }\n"
CALL_START = SOURCE.index("pub fn run")


def apply(peer: Peer, actions: list[dict], landed: bool = False,
          probe: bool = False) -> float | None:
    peer.sequence += 1
    seq = peer.sequence
    start = time.perf_counter_ns()
    peer.send({"type": "act", "seq": seq,
               "base": {"incarnation": peer.incarnation, "generation": peer.generation},
               "actions": actions})
    acknowledged = False
    generation = None
    landing_at = None
    for _ in range(256):
        message, _, observed = peer.receive()
        if message["type"] in ("snapshot", "delta"):
            peer.view(message)
            if landed and peer.panes[0]["cursor"] < CALL_START:
                landing_at = observed
        elif message["type"] == "ack":
            if (message["seq"] != seq or message["outcome"]["outcome"] != "applied"
                    or message["outcome"]["applied"] != peer.applied + 1):
                raise RuntimeError(f"the editor refused an LSP fixture action: {message}")
            peer.applied += 1
            generation = message["outcome"]["generation"]
            acknowledged = True
        else:
            raise RuntimeError(f"unexpected LSP fixture response: {message}")
        if acknowledged and peer.generation >= generation and (not landed or landing_at):
            if landed and not peer.panes[0]["lines"][0].startswith("pub fn answer"):
                raise RuntimeError("definition landed in a different document")
            return round(((landing_at or observed) - start) / 1_000_000, 3)
        if (landed and acknowledged and peer.generation >= generation
                and peer.state.get("message") == "no definition found"):
            if probe:
                return None
            raise RuntimeError("real rust-analyzer returned no definition for answer()")
    raise RuntimeError(f"the server did not land its definition; state={peer.state}")


def wait_for(peer: Peer, predicate, stage: str) -> None:
    for _ in range(256):
        if predicate():
            return
        message, _, _ = peer.receive()
        if message["type"] not in ("snapshot", "delta"):
            raise RuntimeError(f"unexpected {stage} response: {message}")
        peer.view(message)
        if peer.state.get("message", "").startswith("lsp: rust-analyzer failed"):
            raise RuntimeError(f"{stage} failed: {peer.state['message']}")
    raise RuntimeError(f"{stage} did not reach the user-visible view: {peer.state}")


def call_site(peer: Peer) -> None:
    apply(peer, [key(char) for char in ":3"] + [key("Enter"), key("f"), key("a")])
    if peer.panes[0]["cursor"] < CALL_START or peer.state.get("mode") != "NORMAL":
        raise RuntimeError("the LSP fixture did not return to the call site")
    if peer.state.get("message") == "no definition found":
        # The previous reply's identical message is not a new view on
        # its next occurrence. Reset the status through the editor's real
        # writable-state command before admitting another probe.
        apply(peer, [key(char) for char in ":set noro"] + [key("Enter")])
        if peer.state.get("message") == "no definition found":
            raise RuntimeError("the editor retained its prior LSP refusal")


def benchmark(binary: Path, warmup: int, iterations: int, profile: str) -> dict:
    if not binary.is_file() or not os.access(binary, os.X_OK):
        raise ValueError("--binary must be the native executable release artifact")
    if not shutil.which("rust-analyzer"):
        raise ValueError("install rust-analyzer on this native runner before the LSP journey")
    identity = subprocess.check_output([str(binary), "--version"], text=True).strip().split()
    if len(identity) != 2 or identity[0] != "strop":
        raise ValueError("the benchmark artifact did not report a Strop version")
    analyzer = subprocess.check_output(["rust-analyzer", "--version"], text=True).strip()
    with tempfile.TemporaryDirectory(prefix="strop-lsp-ui-") as name:
        root = Path(name)
        for folder in ("home", "config", "state", "src"):
            (root / folder).mkdir()
        (root / "Cargo.toml").write_text(
            '[package]\nname = "strop-lsp-fixture"\nversion = "0.1.0"\nedition = "2021"\n')
        (root / "src/lib.rs").write_text(SOURCE)
        # The fixture isolates editor state, not the runner's installed
        # Rust toolchain: rustup shims need their existing toolchain home.
        env = dict(os.environ, HOME=str(root / "home"),
                   XDG_CONFIG_HOME=str(root / "config"),
                   XDG_STATE_HOME=str(root / "state"), STROP_LOG="",
                   RUSTUP_HOME=os.environ.get("RUSTUP_HOME", str(Path.home() / ".rustup")),
                   CARGO_HOME=os.environ.get("CARGO_HOME", str(Path.home() / ".cargo")),
                   CARGO_NET_OFFLINE="true")
        with tempfile.TemporaryFile() as errors:
            child = subprocess.Popen([str(binary), "--ui-stdio"], cwd=root,
                                     stdin=subprocess.PIPE, stdout=subprocess.PIPE,
                                     stderr=errors, env=env)
            try:
                peer = Peer(child)
                peer.send({"type": "hello", "protocol": 1,
                           "client": {"name": "lsp-ui-benchmark", "version": identity[1]},
                           "capabilities": {"clipboard_write": False}})
                welcome, _, _ = peer.receive()
                if welcome["type"] != "welcome" or welcome["protocol"] != 1:
                    raise RuntimeError(f"editor did not admit its client: {welcome}")
                peer.incarnation = welcome["backend"]["incarnation"]
                first, _, _ = peer.receive()
                peer.view(first)
                peer.sequence += 1
                peer.send({"type": "viewport", "seq": peer.sequence,
                           "columns": 120, "rows": 40})
                saw_ack = False
                for _ in range(256):
                    message, _, _ = peer.receive()
                    if message["type"] in ("snapshot", "delta"):
                        peer.view(message)
                    elif message["type"] == "ack":
                        if (message["seq"] != peer.sequence
                                or message["outcome"]["outcome"] != "applied"):
                            raise RuntimeError(f"editor refused viewport: {message}")
                        peer.applied = message["outcome"]["applied"]
                        saw_ack = True
                    else:
                        raise RuntimeError(f"unexpected viewport response: {message}")
                    if saw_ack and peer.geometry == {"columns": 120, "rows": 40}:
                        break
                else:
                    raise RuntimeError("the editor did not publish its viewport")
                apply(peer, [key(char) for char in ":e src/lib.rs"] + [key("Enter")])
                wait_for(peer, lambda: any(pane["lines"] and pane["lines"][0].startswith(
                    "pub fn answer") for pane in peer.panes), "local source file open")
                ready = peer.state.get("message", "").startswith("lsp: rust-analyzer ready")
                if not ready:
                    wait_for(peer, lambda: peer.state.get("message", "").startswith(
                        "lsp: rust-analyzer ready"), "rust-analyzer readiness")
                workers = worker_processes(child.pid)
                if len(workers) != 1:
                    raise RuntimeError(f"expected one real local worker, got {workers}")
                call_site(peer)
                # Initialize/Ready precedes rust-analyzer's workspace
                ready_replies = 0
                for _ in range(256):
                    if apply(peer, [key("g"), key("d")], landed=True, probe=True) is None:
                        ready_replies = 0
                    else:
                        ready_replies += 1
                    call_site(peer)
                    if ready_replies == 8:
                        break
                else:
                    raise RuntimeError("rust-analyzer never stabilized the fixture definition")
                request = []
                for index in range(warmup + iterations):
                    if peer.panes[0]["cursor"] < CALL_START:
                        raise RuntimeError("the editor left the request origin")
                    elapsed = apply(peer, [key("g"), key("d")], landed=True)
                    if index >= warmup:
                        request.append(elapsed)
                    call_site(peer)
                sync_request = []
                for index in range(warmup + iterations):
                    # The source remains valid; each appended space changes its
                    # revision, forcing a real didChange before the definition.
                    elapsed = apply(peer, [key("A"), key(" "), key("Escape"),
                                           key("0"), key("f"), key("a"),
                                           key("g"), key("d")], landed=True)
                    if index >= warmup:
                        sync_request.append(elapsed)
                    call_site(peer)
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
                        raise RuntimeError(f"unexpected shutdown response: {message}")
                else:
                    raise RuntimeError("the editor did not terminate the LSP session")
                if child.wait(timeout=15) != 0:
                    raise RuntimeError("the editor exited nonzero")
                with binary.open("rb") as artifact:
                    digest = hashlib.file_digest(artifact, "sha256").hexdigest()
                return {
                    "binary_sha256": digest, "binary_bytes": binary.stat().st_size,
                    "binary_version": identity[1], "analyzer": analyzer,
                    "platform": {"machine": platform.machine(), "kernel": platform.release(),
                                 "cpu": cpu_model(), "profile": profile},
                    "method": "admitted UI gesture through real rust-analyzer reply and painted definition",
                    "fixture": {"workspace": "ephemeral on-disk Rust crate",
                                "geometry": [120, 40], "worker_count": len(workers),
                                "sync": "insert trailing space on call line, then navigate by gd",
                                "sync_scope": "edit+didChange+request+view, not didChange acknowledgment"},
                    "warmup_requests": warmup, "measured_requests": iterations,
                    "request_ms": percentiles(request), "request_raw_ms": request,
                    "sync_request_ms": percentiles(sync_request),
                    "sync_request_raw_ms": sync_request,
                }
            finally:
                if child.poll() is None:
                    child.kill()
                    child.wait()
                if child.returncode != 0:
                    errors.seek(0)
                    detail = errors.read(1024)
                    if detail:
                        print(f"LSP editor diagnostic: {detail!r}", file=os.sys.stderr)


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
        parser.error("iterations must be positive and warmup nonnegative")
    try:
        result = benchmark(args.binary, args.warmup, args.iterations,
                           args.profile or native_profile())
    except (OSError, KeyError, ValueError, RuntimeError, TimeoutError,
            subprocess.TimeoutExpired) as error:
        parser.exit(1, f"LSP UI measurement failed: {error}\n")
    serialized = json.dumps(result, indent=2, sort_keys=True) + "\n"
    if args.out is None:
        print(serialized, end="")
    else:
        args.out.parent.mkdir(parents=True, exist_ok=True)
        args.out.write_text(serialized, encoding="utf-8")
        print(f"real LSP measurements: {args.out} ({args.iterations} samples per path)")


if __name__ == "__main__":
    main()
