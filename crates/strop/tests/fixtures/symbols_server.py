#!/usr/bin/env python3
"""A language server that answers `documentSymbol` with a reply too large to
decode the first time, and with one symbol after (plans/0101 §5.2). Every
`initialize` and `documentSymbol` it receives is logged, one line each, to the
path in argv[1]. No network, no sleep, no real HOME.
"""
import json
import sys

LOG = open(sys.argv[1], "a", buffering=1)
TOO_MANY = 40_000  # 43 structure tokens each: over the 512 MiB decode budget


def send(value):
    body = json.dumps(value, separators=(",", ":")).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def symbol(index, name):
    line = index * 3
    return {
        "name": name,
        "detail": "fn(&self, value: usize) -> usize",
        "kind": 12,
        "tags": [],
        "deprecated": False,
        "range": {"start": {"line": line, "character": 0},
                  "end": {"line": line + 2, "character": 1}},
        "selectionRange": {"start": {"line": line, "character": 3},
                           "end": {"line": line, "character": 4}},
        "children": [],
    }


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line == b"\r\n":
            break
        name, _, value = line.decode().partition(":")
        if name.lower() == "content-length":
            length = int(value.strip())
    return json.loads(sys.stdin.buffer.read(length))


asked = 0
while True:
    message = read()
    if message is None:
        break
    method = message.get("method")
    if method == "initialize":
        LOG.write("initialize\n")
        send({"jsonrpc": "2.0", "id": message["id"], "result": {
            "capabilities": {"documentSymbolProvider": True, "textDocumentSync": 1},
        }})
    elif method == "textDocument/documentSymbol":
        asked += 1
        LOG.write("documentSymbol\n")
        if asked == 1:
            result = [symbol(i, "f%d" % i) for i in range(TOO_MANY)]
        else:
            result = [symbol(0, "only_symbol")]
        send({"jsonrpc": "2.0", "id": message["id"], "result": result})
    elif method == "shutdown":
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
    elif method == "exit":
        break
    elif "id" in message:
        send({"jsonrpc": "2.0", "id": message["id"], "result": None})
