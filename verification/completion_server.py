#!/usr/bin/env python3
"""Owned LSP pressure fixture for 0059, never a product completion provider.

A private Unix control socket releases held requests in an exact chosen order.
Cancellation is intentionally advisory: held requests retain their physical
ownership until release or shutdown. No sleep, network, real HOME or toolchain.
"""
from __future__ import annotations

import argparse
import json
import os
import selectors
import socket
import sys
import time
from pathlib import Path

MAX_INPUT = 70 * 1024 * 1024


def frame(value: dict) -> bytes:
    body = json.dumps(value, separators=(",", ":")).encode()
    return b"Content-Length: %d\r\n\r\n" % len(body) + body


def position(text: str, units: int) -> int:
    consumed = 0
    for index, char in enumerate(text):
        if consumed == units:
            return index
        consumed += len(char.encode("utf-16-le")) // 2
    if consumed == units:
        return len(text)
    raise ValueError("fixture received an invalid UTF-16 position")


class Server:
    def __init__(self, mode: str, control: Path):
        self.mode = mode
        self.selector = selectors.DefaultSelector()
        self.selector.register(sys.stdin.fileno(), selectors.EVENT_READ, "lsp")
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(str(control))
        self.listener.listen(1)
        self.selector.register(self.listener, selectors.EVENT_READ, "accept")
        self.clients: dict[socket.socket, bytearray] = {}
        self.waiters: dict[socket.socket, dict] = {}
        self.input = bytearray()
        self.documents: dict[str, str] = {}
        self.versions: dict[str, int] = {}
        self.held: dict[int, tuple[str, object]] = {}
        self.events: list[dict] = []
        self.counts = {"queries": 0, "resolves": 0, "cancels": 0, "formats": 0,
                       "physical_high_water": 0, "sync_bytes": 0, "sync_count": 0}
        self.running = True

    def reply(self, ident: int, result: object) -> None:
        sys.stdout.buffer.write(frame({"jsonrpc": "2.0", "id": ident, "result": result}))
        sys.stdout.buffer.flush()

    def note(self, kind: str, **fields) -> None:
        if len(self.events) < 8192:
            self.events.append({"event": kind, "at_ns": time.perf_counter_ns(), **fields})

    def result(self, ident: int, kind: str, value: object, hold: bool) -> None:
        self.counts["physical_high_water"] = max(self.counts["physical_high_water"], len(self.held) + 1)
        if hold:
            self.held[ident] = (kind, value)
        else:
            self.reply(ident, value)
        self.note(kind, id=ident, held=hold)

    def completion(self, params: dict) -> list[dict]:
        uri = params["textDocument"]["uri"]
        lines = self.documents[uri].split("\n")
        at = params["position"]
        line = lines[at["line"]].removesuffix("\r")
        caret = position(line, at["character"])
        start = caret
        while start and (line[start - 1].isalnum() or line[start - 1] == "_"):
            start -= 1
        prefix = line[start:caret]
        beginning = len(line[:start].encode("utf-16-le")) // 2
        labels = ["response", "result"]
        if prefix and not any(label.startswith(prefix) for label in labels):
            labels = [prefix + "Response", prefix + "Result"]
        items = [{
            "label": label, "kind": 6, "sortText": str(index), "detail": "fixture value",
            "textEdit": {"range": {"start": {"line": at["line"], "character": beginning},
                                     "end": at}, "newText": label},
            "documentation": {"kind": "markdown", "value": "**Selected value**\n\n```c\nint response;\n```"},
            "data": {"fixture": index, "secret": "completion-opaque-private-7c3a80"},
        } for index, label in enumerate(labels)]
        if self.mode == "oversized":
            items.insert(0, {"label": prefix + "Oversized", "documentation": "x" * (70 * 1024)})
            items.extend({"label": prefix + f"Candidate{index:04}", "detail": "d" * 2048}
                         for index in range(512))
        return items

    def message(self, message: dict) -> None:
        method = message.get("method")
        params = message.get("params") or {}
        ident = message.get("id")
        if method == "initialize":
            self.reply(ident, {"capabilities": {
                "positionEncoding": "utf-16", "textDocumentSync": 1,
                "documentFormattingProvider": True,
                "definitionProvider": True,
                "completionProvider": {"resolveProvider": True, "triggerCharacters": ["."]},
            }, "serverInfo": {"name": "strop-owned-completion-pressure-fixture"}})
        elif method in ("textDocument/didOpen", "textDocument/didChange"):
            document = params["textDocument"]
            uri = document["uri"]
            text = document.get("text") if method.endswith("didOpen") else params["contentChanges"][-1]["text"]
            if uri in self.versions and document["version"] <= self.versions[uri]:
                raise ValueError("document synchronization versions did not increase")
            self.documents[uri] = text
            self.versions[uri] = document["version"]
            self.counts["sync_bytes"] += len(text.encode())
            self.counts["sync_count"] += 1
            self.note("sync", version=document["version"], bytes=len(text.encode()))
        elif method == "textDocument/didClose":
            uri = params["textDocument"]["uri"]
            self.documents.pop(uri, None)
        elif method == "textDocument/completion":
            self.counts["queries"] += 1
            if self.mode == "oversized-frame":
                self.result(ident, "completion", [{"label": "x" * (33 * 1024 * 1024)}], False)
            else:
                self.result(ident, "completion", self.completion(params), self.mode in ("hold", "ignore"))
        elif method == "completionItem/resolve":
            self.counts["resolves"] += 1
            if params.get("data", {}).get("secret") != "completion-opaque-private-7c3a80":
                raise ValueError("completion resolve lost opaque protocol data")
            result = dict(params)
            result["documentation"] = {"kind": "markdown", "value": "**Resolved value**\n\n```c\nint response;\n```"}
            if self.mode in ("import", "resolve-hold"):
                result["additionalTextEdits"] = [{"range": {
                    "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}},
                    "newText": "// represented completion import\n"}]
            self.result(ident, "resolve", result, self.mode == "resolve-hold")
        elif method == "textDocument/definition":
            self.reply(ident, {"uri": params["textDocument"]["uri"], "range": {
                "start": {"line": 0, "character": 0}, "end": {"line": 0, "character": 0}}})
            self.note("definition", id=ident)
        elif method == "textDocument/formatting":
            self.counts["formats"] += 1
            self.note("format", id=ident)
            self.reply(ident, [{"range": {"start": {"line": 99, "character": 0},
                                          "end": {"line": 99, "character": 0}},
                                "newText": "WRONG-LINE"}])
        elif method == "$/cancelRequest":
            self.counts["cancels"] += 1
            self.note("cancel", id=params["id"])
        elif method == "shutdown":
            self.reply(ident, None)
        elif method == "exit":
            self.running = False
        elif ident is not None:
            self.reply(ident, None)

    def control(self, command: dict) -> dict | None:
        if command["action"] == "await":
            if self.counts[command["counter"]] < command["at_least"]:
                return None
        if command["action"] == "release":
            chosen = command.get("id")
            for ident in list(self.held):
                kind, value = self.held[ident]
                if chosen is not None and ident != chosen:
                    continue
                if command.get("kind") not in (None, kind):
                    continue
                del self.held[ident]
                self.reply(ident, value)
                self.note("released", id=ident)
        elif command["action"] not in ("snapshot", "await"):
            raise ValueError("unknown fixture control action")
        return {"counts": self.counts, "held": [{"id": ident, "kind": value[0]}
                for ident, value in self.held.items()], "events": self.events}

    def read_messages(self) -> None:
        chunk = os.read(sys.stdin.fileno(), 65536)
        if not chunk:
            self.running = False
            return
        self.input.extend(chunk)
        if len(self.input) > MAX_INPUT:
            raise ValueError("fixture input exceeded its explicit bound")
        while b"\r\n\r\n" in self.input:
            end = self.input.index(b"\r\n\r\n")
            headers = self.input[:end].decode("ascii").split("\r\n")
            lengths = [int(value) for line in headers for name, sep, value in [line.partition(":")]
                       if sep and name.lower() == "content-length"]
            if len(lengths) != 1 or not 0 <= lengths[0] <= MAX_INPUT:
                raise ValueError("invalid fixture input framing")
            total = end + 4 + lengths[0]
            if len(self.input) < total:
                return
            message = json.loads(self.input[end + 4:total])
            del self.input[:total]
            self.message(message)
            for client, command in list(self.waiters.items()):
                response = self.control(command)
                if response is not None:
                    del self.waiters[client]
                    client.sendall(json.dumps(response).encode() + b"\n")

    def run(self) -> None:
        try:
            while self.running:
                for event, _ in self.selector.select():
                    if event.data == "lsp":
                        self.read_messages()
                    elif event.data == "accept":
                        client, _ = self.listener.accept()
                        self.clients[client] = bytearray()
                        self.selector.register(client, selectors.EVENT_READ, "control")
                    else:
                        client = event.fileobj
                        chunk = client.recv(65536)
                        if not chunk:
                            self.selector.unregister(client)
                            del self.clients[client]
                            self.waiters.pop(client, None)
                            client.close()
                            continue
                        pending = self.clients[client]
                        pending.extend(chunk)
                        if len(pending) > 8192:
                            raise ValueError("fixture control command exceeded its bound")
                        while b"\n" in pending:
                            end = pending.index(b"\n")
                            command = json.loads(pending[:end])
                            del pending[:end + 1]
                            response = self.control(command)
                            if response is None:
                                self.waiters[client] = command
                            else:
                                client.sendall(json.dumps(response).encode() + b"\n")
        finally:
            for client in self.clients:
                client.close()
            self.listener.close()
            self.selector.close()


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--control", type=Path, required=True)
    parser.add_argument("--mode", choices=("ready", "hold", "ignore", "resolve-hold", "import",
                                          "oversized", "oversized-frame"), default="ready")
    args = parser.parse_args()
    Server(args.mode, args.control).run()


if __name__ == "__main__":
    main()
