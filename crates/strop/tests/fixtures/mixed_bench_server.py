"""#262 controlled provider. File gates live outside the measured workarea."""
import json
from pathlib import Path
import sys
import time

control, target = map(Path, sys.argv[1:])
uri = target.as_uri()
name = "generated_0000"
line = target.read_text().splitlines()[1]
column = line.index(name)
span = {"start": {"line": 1, "character": column},
        "end": {"line": 1, "character": column + len(name)}}


def gate(tag):
    (control / (tag + ".waiting")).touch()
    deadline = time.monotonic() + 30
    while not (control / (tag + ".release")).exists():
        if time.monotonic() > deadline:
            raise RuntimeError(tag + " gate timed out")
        time.sleep(.001)


def read():
    stream = sys.stdin.buffer
    length = None
    while True:
        line = stream.readline()
        if not line:
            return None
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    return json.loads(stream.read(length))


while (msg := read()) is not None:
    method = msg.get("method")
    if method == "exit":
        break
    if "id" not in msg:
        continue
    if method == "initialize":
        result = {"capabilities": {"textDocumentSync": 1, "documentSymbolProvider": True,
                                   "workspaceSymbolProvider": True}}
    elif method == "textDocument/documentSymbol" and msg["params"]["textDocument"]["uri"] == uri:
        gate("document")
        time.sleep(.080)  # deliberate provider service time, not a readiness barrier
        result = [{"name": name, "kind": 12, "range": span, "selectionRange": span}]
    elif method == "workspace/symbol":
        query = msg["params"]["query"]
        if query == "retired":
            gate("retired")
        time.sleep(.080)
        # Distinct qualified server label for precisely the same target token.
        label = {"retired": "proof::generated_0000_retired", "proof": "proof::generated_0000"}.get(
            query, "agent_generated_0000")
        result = [{"name": label, "kind": 12, "location": {"uri": uri, "range": span}}]
    else:
        result = None
    body = json.dumps({"jsonrpc": "2.0", "id": msg["id"], "result": result}).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
