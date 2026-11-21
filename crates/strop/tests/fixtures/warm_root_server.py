"""Record the real child cwd and the rootUri received on initialize (#243)."""
import json
import sys
from pathlib import Path

log, label = sys.argv[1:]
stream = sys.stdin.buffer
while True:
    length = None
    while True:
        line = stream.readline()
        if not line:
            sys.exit(0)
        if line == b"\r\n":
            break
        if line.lower().startswith(b"content-length:"):
            length = int(line.split(b":", 1)[1])
    message = json.loads(stream.read(length))
    if "id" not in message:
        if message.get("method") == "exit":
            break
        continue
    method = message.get("method")
    if method == "initialize":
        cwd = Path.cwd()
        with open(log, "a") as out:
            print(json.dumps([label, str(cwd), cwd.as_uri(), message["params"]["rootUri"]]), file=out)
        result = {"capabilities": {"workspaceSymbolProvider": True, "documentSymbolProvider": True}}
    elif method in ("workspace/symbol", "textDocument/documentSymbol"):
        result = []
    else:
        result = None
    body = json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
