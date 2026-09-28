#!/usr/bin/env python3
"""0059 completion-item/resolve privacy and native-free full replay.

The actual headless frontend supports forensic capture. --ui-stdio deliberately
forbids it; this is a separate capture journey, not a performance measurement.
"""
from __future__ import annotations

import argparse
import hashlib
import json
import os
import subprocess
import sys
import tempfile
from pathlib import Path


SCRIPT = """settle 10000
keys G$a<c-space>
settle 10000
keys <c-n>
settle 10000
frame
keys <c-y>
settle 10000
frame
keys <esc>u
frame
state
"""


def check(binary: Path) -> dict:
    source = "// fixture\nresponse result\nre\n"
    fixture = Path(__file__).with_name("completion_server.py").resolve()
    captures = {}
    with tempfile.TemporaryDirectory(prefix="strop-completion-capture-") as directory:
        base = Path(directory)
        for content in (False, True):
            root = base / ("full" if content else "metadata")
            for folder in ("bin", "home", "config", "state", "cache"):
                (root / folder).mkdir(parents=True)
            file = root / "input.c"
            file.write_text(source)
            script = root / "journey.strop"
            script.write_text(SCRIPT)
            trace = root / "capture.jsonl"
            executable = root / "bin/clangd"
            executable.write_text(f"#!{sys.executable}\nimport runpy, sys\n"
                f"sys.argv = [{str(fixture)!r}, '--mode', 'import', '--control', {str(root / 'control.sock')!r}]\n"
                f"runpy.run_path({str(fixture)!r}, run_name='__main__')\n")
            executable.chmod(0o700)
            environment = dict(os.environ, HOME=str(root / "home"),
                XDG_CONFIG_HOME=str(root / "config"), XDG_STATE_HOME=str(root / "state"),
                XDG_CACHE_HOME=str(root / "cache"), STROP_LOG="",
                PATH=str(root / "bin") + os.pathsep + os.environ.get("PATH", "/usr/bin:/bin"))
            command = [str(binary), "--headless", str(script), str(file), "--log-file", str(trace)]
            if content:
                command.append("--log-content")
            result = subprocess.run(command, cwd=root, env=environment, capture_output=True, text=True, timeout=30)
            if result.returncode:
                raise RuntimeError(f"completion capture failed: {result.stderr}")
            if "// represented completion import" not in result.stdout:
                raise RuntimeError("the capture never applied the resolved completion import")
            if "response" not in result.stdout or "NORMAL" not in result.stdout:
                raise RuntimeError("the capture did not finish its acceptance/undo journey")
            captured = trace.read_text()
            payloads = {
                "opaque_data": "completion-opaque-private-7c3a80",
                "detail": "fixture value",
                "documentation": "Selected value",
                "import": "// represented completion import",
            }
            present = {name: value in captured for name, value in payloads.items()}
            if any(value != content for value in present.values()):
                raise RuntimeError(f"completion payload violated the selected capture policy: {present}")
            records = [json.loads(line) for line in captured.splitlines()]
            if records[-1]["event"] != "trace_end" or not records[-1]["fields"]["complete"]:
                raise RuntimeError("completion capture did not close completely")
            if content:
                # The capture is the authority: neither its original source
                # nor the native fixture executable remains available.
                file.unlink()
                executable.unlink()
                environment["PATH"] = str(root / "no-native-executables")
                environment["HOME"] = str(root / "replay-home")
                environment["XDG_CONFIG_HOME"] = str(root / "replay-config")
                environment["XDG_STATE_HOME"] = str(root / "replay-state")
                replay = subprocess.run([str(binary), "--replay", str(trace)], cwd=root,
                    env=environment, capture_output=True, text=True, timeout=30)
                if replay.returncode:
                    raise RuntimeError(f"native-free LSP completion replay failed: {replay.stderr}")
                state = json.loads(replay.stdout)
                if state["mode"] != "NORMAL" or state["completion"]["worker"] != "idle":
                    raise RuntimeError("replay did not preserve completion exit/retirement")
                captures["native_free_full_replay"] = True
            captures["full" if content else "metadata"] = {
                "bytes": trace.stat().st_size, "events": len(records),
                "payloads_present": present,
                "trace_sha256": hashlib.sha256(captured.encode()).hexdigest(),
            }
        with binary.open("rb") as artifact:
            digest = hashlib.file_digest(artifact, "sha256").hexdigest()
        return {"binary_sha256": digest, "method": "actual headless LSP resolve/import/undo with controlled server; native-free full replay",
                "captures": captures}


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--binary", type=Path, required=True)
    args = parser.parse_args()
    print(json.dumps(check(args.binary.resolve()), indent=2, sort_keys=True))


if __name__ == "__main__":
    main()
