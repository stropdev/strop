"""Answer initialize with the capabilities given as JSON and log every method received (#244)."""
import json
import sys

log, capabilities = sys.argv[1], json.loads(sys.argv[2])
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
    method = message.get("method")
    if method is not None:
        with open(log, "a") as out:
            print(method, file=out)
    if "id" not in message:
        if method == "exit":
            break
        continue
    if method == "initialize":
        result = {"capabilities": capabilities}
    elif method == "workspace/symbol":
        result = []
    else:
        result = None
    body = json.dumps({"jsonrpc": "2.0", "id": message["id"], "result": result}).encode()
    sys.stdout.buffer.write(b"Content-Length: %d\r\n\r\n" % len(body) + body)
    sys.stdout.buffer.flush()
