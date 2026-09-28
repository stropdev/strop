"""Actual framed editor + private controlled LSP fixture used by 0059 evidence."""
from __future__ import annotations

import json
import os
import selectors
import socket
import subprocess
import sys
import tempfile
import time
from pathlib import Path

from bench_ui_input_frame import Peer, key
from bench_worker import process_state, worker_processes


def control_key(character: str) -> dict:
    event = key(character)
    event["data"]["Key"]["modifiers"]["control"] = True
    return event


class CompletionEditor:
    def __init__(self, binary: Path, text: str, *, enabled: bool = True,
                 automatic: bool = True, server: str | None = None,
                 geometry: tuple[int, int] = (120, 40)):
        # The native control socket must fit macOS's 104-byte sockaddr_un.
        self.directory = tempfile.TemporaryDirectory(prefix="strop-completion-", dir="/tmp")
        self.root = Path(self.directory.name)
        self.socket = None
        self.control_input = bytearray()
        self.snapshots = []
        self.rebased_actions = 0
        self.editor_peak_kib = 0
        self.worker_peak_kib = 0
        self.worker_threads_peak = 0
        self.completion_threads_peak = 0 if sys.platform == "linux" else None
        for folder in ("home", "config/strop", "state", "cache", "bin"):
            (self.root / folder).mkdir(parents=True)
        (self.root / "config/strop/config.toml").write_text(
            f"[completion]\nenabled = {str(enabled).lower()}\nauto_popup = {str(automatic).lower()}\n")
        self.file = self.root / ("input.c" if server else "input.txt")
        self.file.write_text(text)
        env = dict(os.environ, HOME=str(self.root / "home"),
                   XDG_CONFIG_HOME=str(self.root / "config"),
                   XDG_STATE_HOME=str(self.root / "state"),
                   XDG_CACHE_HOME=str(self.root / "cache"), STROP_LOG="")
        if server:
            fixture = Path(__file__).with_name("completion_server.py").resolve()
            executable = self.root / "bin/clangd"
            executable.write_text(f"#!{sys.executable}\nimport runpy, sys\n"
                f"sys.argv = [{str(fixture)!r}, '--mode', {server!r}, '--control', {str(self.root / 'control.sock')!r}]\n"
                f"runpy.run_path({str(fixture)!r}, run_name='__main__')\n")
            executable.chmod(0o700)
            env["PATH"] = str(self.root / "bin") + os.pathsep + env.get("PATH", "/usr/bin:/bin")
        self.errors = tempfile.TemporaryFile()
        self.child = subprocess.Popen([str(binary), "--ui-stdio"], cwd=self.root, env=env,
                                      stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=self.errors)
        self.peer = Peer(self.child)
        try:
            self.peer.send({"type": "hello", "protocol": 1,
                            "client": {"name": "completion-qualification", "version": "0059"},
                            "capabilities": {"clipboard_write": False}})
            welcome, _, _ = self.peer.receive()
            if welcome["type"] != "welcome":
                raise RuntimeError(f"editor refused qualification handshake: {welcome}")
            self.peer.incarnation = welcome["backend"]["incarnation"]
            self.view(self.peer.receive()[0])
            self.viewport(*geometry)
            self.act([key(char) for char in ":e " + self.file.name] + [key("Enter")])
            self.wait(lambda: any(pane["lines"] and pane["lines"][0] == text.split("\n")[0]
                                 for pane in self.peer.panes), "source open")
            if server:
                self.wait(lambda: "clangd ready" in self.peer.state.get("message", ""), "fixture ready")
                self.socket = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
                self.socket.settimeout(15)
                self.socket.connect(str(self.root / "control.sock"))
            self.resources()
        except BaseException:
            self.close()
            raise

    def view(self, message: dict) -> None:
        self.peer.view(message)
        completion = self.peer.state.get("completion")
        if completion:
            self.snapshots.append(completion)
            publication = completion["publication"]
            if publication["high_water_bytes"] > 24 * 1024 * 1024:
                raise RuntimeError("completion publication exceeded its native retention bound")

    def viewport(self, columns: int, rows: int) -> None:
        self.peer.sequence += 1
        self.peer.send({"type": "viewport", "seq": self.peer.sequence,
                        "columns": columns, "rows": rows})
        ack = None
        while ack is None or self.peer.geometry != {"columns": columns, "rows": rows}:
            message, _, _ = self.peer.receive()
            if message["type"] in ("snapshot", "delta"):
                self.view(message)
            elif message["type"] == "ack" and message["outcome"]["outcome"] == "applied":
                ack = message
                self.peer.applied = message["outcome"]["applied"]
            else:
                raise RuntimeError(f"viewport failed: {message}")

    def act(self, actions: list[dict]) -> float:
        start = time.perf_counter_ns()
        # A native publication can cross an in-flight input. The real protocol
        # refuses the old base; rebase the same intent, counting that latency.
        for _ in range(16):
            self.peer.sequence += 1
            seq = self.peer.sequence
            self.peer.send({"type": "act", "seq": seq,
                            "base": {"incarnation": self.peer.incarnation, "generation": self.peer.generation},
                            "actions": actions})
            generation = None
            view_at = None
            while True:
                message, _, at = self.peer.receive()
                if message["type"] in ("snapshot", "delta"):
                    self.view(message)
                    view_at = at
                elif message["type"] == "ack":
                    if message["seq"] != seq:
                        raise RuntimeError("out-of-order qualification acknowledgement")
                    outcome = message["outcome"]
                    if outcome["outcome"] != "applied":
                        if "stale_generation" in json.dumps(outcome):
                            self.rebased_actions += 1
                            break
                        raise RuntimeError(f"completion action refused: {message}")
                    if outcome["applied"] != self.peer.applied + 1:
                        raise RuntimeError("action did not advance exactly once")
                    self.peer.applied = outcome["applied"]
                    generation = outcome["generation"]
                else:
                    raise RuntimeError(f"unexpected completion input response: {message}")
                if generation is not None and self.peer.generation >= generation and view_at is not None:
                    return (view_at - start) / 1_000_000
        raise RuntimeError("completion input could not catch up with the published view")

    def keys(self, keys: str) -> float:
        return self.act([key(char) for char in keys])

    def wait(self, predicate, stage: str) -> float:
        start = time.perf_counter_ns()
        deadline = time.monotonic() + 30
        while not predicate():
            if time.monotonic() >= deadline:
                raise TimeoutError(f"{stage} stalled: {self.peer.state}")
            message, _, _ = self.peer.receive()
            if message["type"] not in ("snapshot", "delta"):
                raise RuntimeError(f"unexpected {stage} response: {message}")
            self.view(message)
        return (time.perf_counter_ns() - start) / 1_000_000

    def completion(self) -> dict:
        return self.peer.state.get("completion") or {}

    def menu(self) -> dict:
        return self.completion().get("menu") or {}

    def provider(self, source: str) -> str | None:
        return next((provider["state"] for provider in self.completion().get("providers") or []
                     if provider["source"] == source), None)

    def control(self, action: str = "snapshot", **fields) -> dict:
        if self.socket is None:
            raise RuntimeError("this case has no controlled fixture")
        self.socket.sendall(json.dumps({"action": action, **fields}).encode() + b"\n")
        while b"\n" not in self.control_input:
            chunk = self.socket.recv(65536)
            if not chunk:
                raise RuntimeError("completion fixture control socket closed")
            self.control_input.extend(chunk)
        end = self.control_input.index(b"\n")
        response = json.loads(self.control_input[:end])
        del self.control_input[:end + 1]
        if response["counts"]["physical_high_water"] > 2:
            raise RuntimeError("ignoring server received more than two physical completion requests")
        return response

    def resources(self) -> dict:
        rss, threads = process_state(self.child.pid)
        workers = worker_processes(self.child.pid)
        if len(workers) != 1:
            raise RuntimeError(f"expected one real local worker, got {workers}; use a production build without test-support")
        worker_states = [process_state(pid) for pid in workers]
        completion_threads = None
        if sys.platform == "linux":
            completion_threads = 0
            for path in Path(f"/proc/{self.child.pid}/task").glob("*/comm"):
                try:
                    completion_threads += path.read_text().strip().startswith("strop-complet")
                except FileNotFoundError:
                    pass  # An unrelated forwarder can retire during the census.
        if completion_threads is not None and completion_threads > 1:
            raise RuntimeError("completion admitted overlapping persistent CPU workers")
        self.editor_peak_kib = max(self.editor_peak_kib, rss)
        self.worker_peak_kib = max(self.worker_peak_kib, sum(item[0] for item in worker_states))
        self.worker_threads_peak = max(self.worker_threads_peak, sum(item[1] for item in worker_states))
        if completion_threads is not None:
            self.completion_threads_peak = max(self.completion_threads_peak, completion_threads)
        return {"editor_rss_kib": rss, "editor_threads": threads, "worker_pids": workers,
                "worker_rss_kib": sum(item[0] for item in worker_states),
                "completion_threads": completion_threads}

    def shutdown(self) -> dict:
        workers = worker_processes(self.child.pid)
        descriptors = [os.pidfd_open(pid) for pid in workers] if sys.platform == "linux" else []
        self.peer.sequence += 1
        self.peer.send({"type": "shutdown", "seq": self.peer.sequence})
        acked = False
        try:
            while True:
                message, _, _ = self.peer.receive()
                if message["type"] in ("snapshot", "delta"):
                    self.view(message)
                elif message["type"] == "ack":
                    acked = message["outcome"]["outcome"] == "applied"
                elif message["type"] == "bye" and acked:
                    break
                else:
                    raise RuntimeError(f"completion shutdown failed: {message}")
            if self.child.wait(timeout=15):
                raise RuntimeError("completion editor exited nonzero")
            for descriptor in descriptors:
                with selectors.DefaultSelector() as selector:
                    selector.register(descriptor, selectors.EVENT_READ)
                    if not selector.select(15):
                        raise RuntimeError("an owned native worker survived editor shutdown")
            return {"owned_worker_count": len(workers), "owned_workers_exited": len(descriptors)
                    if sys.platform == "linux" else None}
        finally:
            for descriptor in descriptors:
                os.close(descriptor)

    def close(self) -> None:
        if self.socket is not None:
            self.socket.close()
        if self.child.poll() is None:
            self.child.kill()
            self.child.wait()
        if self.child.returncode:
            self.errors.seek(0)
            print(self.errors.read().decode(errors="replace")[:4096], file=sys.stderr)
        self.errors.close()
        self.directory.cleanup()

    def __enter__(self):
        return self

    def __exit__(self, *_):
        self.close()
