"""Test server: report one declaration in the negotiated column units."""
import json
from pathlib import Path
import sys

encoding, target, name = sys.argv[1:]
target = Path(target)
uri = target.as_uri()
prefix, _ = target.read_text(encoding="utf-8").split(name, 1)
column = len(prefix.encode("utf-16-le")) // 2 if encoding == "utf-16" else len(prefix.encode())
span = {"start": {"line": 0, "character": column},
        "end": {"line": 0, "character": column + len(name)}}


def read():
    length = None
    while True:
        line = sys.stdin.buffer.readline()
        if not line:
            return None
        if line == b"\r\n":
            break
        key, _, value = line.partition(b":")
        if key.lower() == b"content-length":
            length = int(value)
    return json.loads(sys.stdin.buffer.read(length))


while (msg := read()) is not None:
    method = msg.get("method")
    if method == "exit":
        break
    if "id" not in msg:
        continue
    if method == "initialize":
        result = {"capabilities": {"positionEncoding": encoding, "documentSymbolProvider": True,
                                   "workspaceSymbolProvider": True, "textDocumentSync": 1}}
    elif method == "workspace/symbol":
        result = [{"name": name, "kind": 14, "location": {"uri": uri, "range": span}}]
    elif method == "textDocument/documentSymbol" and msg["params"]["textDocument"]["uri"] == uri:
        result = [{"name": name, "kind": 14, "range": span, "selectionRange": span}]
    else:
        result = None
    body = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
