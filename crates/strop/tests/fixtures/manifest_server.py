"""#268: root/cwd witness and gated workspace replies. No installed LSPs needed."""
import json
import os
from pathlib import Path
import socket
import sys

log, label, filename = sys.argv[1:]
cwd = Path.cwd()
target = cwd / "src" / filename
stream = sys.stdin.buffer


def record(method, **fields):
    with open(log, "a", encoding="utf-8") as out:
        print(json.dumps(dict(method=method, label=label, cwd=str(cwd), **fields)), file=out)


def receive():
    length = None
    while True:
        line = stream.readline()
        if not line:
            return None
        if line == b"\r\n":
            return json.loads(stream.read(length))
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])


def send(value):
    body = json.dumps(dict(jsonrpc="2.0", **value)).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()


def symbol():
    # Only comments in these fixture files: this row cannot come from syntax.
    if not target.is_file():
        return None
    text = target.read_text(encoding="utf-8").splitlines()[0]
    name = text.split()[-1]
    col = len(text[:text.rindex(name)].encode("utf-8"))
    span = {"start": {"line": 0, "character": col},
            "end": {"line": 0, "character": col + len(name.encode("utf-8"))}}
    return {"name": name, "kind": 14, "location": {"uri": target.as_uri(), "range": span}}


while (msg := receive()) is not None:
    method = msg.get("method")
    if method == "exit":
        break
    if method == "textDocument/didOpen":
        doc = msg["params"]["textDocument"]
        record(method, language=doc["languageId"], uri=doc["uri"])
    if "id" not in msg:
        continue
    gate = None
    if method == "initialize":
        record(method, uri=msg["params"]["rootUri"], cwd_uri=cwd.as_uri())
        result = {"capabilities": {"textDocumentSync": 1, "positionEncoding": "utf-8",
                                   "documentSymbolProvider": True,
                                   "workspaceSymbolProvider": label != "no-capability"}}
    elif method == "workspace/symbol":
        row = symbol()
        # No earlier answer may populate the stale-delivery journey's picker.
        if row and row["name"].startswith("Epoch") and msg["params"]["query"] != "Epoch":
            row = None
        result = [row] if row else []
        if row and row["name"] in ("EpochOld", "EpochLive") and msg["params"]["query"] == "Epoch":
            # One rendezvous per process/role. Additional live queries reply normally.
            gatepath = Path(log).parent / row["name"]
            if gatepath.exists() and not getattr(symbol, "gated", False):
                symbol.gated = True
                gate = socket.socket(socket.AF_UNIX)
                gate.settimeout(60)
                gate.connect(str(gatepath))
                if gate.recv(1) != b"!":
                    sys.exit(0)
    elif method == "textDocument/documentSymbol":
        row = symbol()
        if row and msg["params"]["textDocument"]["uri"] == row["location"]["uri"]:
            span = row["location"]["range"]
            result = [{"name": row["name"], "kind": row["kind"], "range": span, "selectionRange": span}]
        else:
            result = []
    else:
        result = None
    try:
        send({"id": msg["id"], "result": result})
    except BrokenPipeError:
        # Revocation can close the actual process transport before release.
        os._exit(0)
    if gate is not None:
        gate.sendall(b"!")
        gate.close()
