#!/bin/sh
# Publication prerequisites include versioned dev-dependencies: Cargo resolves
# them while generating the packaged lockfile, before compiling the library.
set -eu
ROOT=$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)
python3 - "$ROOT/.github/scripts/publish-order.py" <<'PY'
import json
import subprocess
import sys

script = sys.argv[1]


def dependency(name, kind=None, requirement="^1.0.0"):
    return {"name": name, "kind": kind, "path": f"/fixture/{name}",
            "req": requirement}


def package(name, dependencies=(), publish=None):
    return {"id": name, "name": name, "dependencies": list(dependencies),
            "publish": publish}


def run(packages):
    metadata = {"workspace_members": [p["id"] for p in packages],
                "packages": packages}
    return subprocess.run([sys.executable, script], input=json.dumps(metadata),
                          text=True, capture_output=True, check=False)


# The client has a shallower runtime graph, but packaging still requires its
# worker test dependency to have reached the registry first (0.36.0 failure).
result = run([
    package("core"),
    package("fs", [dependency("core")]),
    package("worker", [dependency("fs")]),
    package("client", [dependency("core"), dependency("worker", "dev")]),
])
assert result.returncode == 0, result.stderr
order = result.stdout.splitlines()
assert set(order) == {"core", "fs", "worker", "client"}, order
assert order.index("core") < order.index("fs") < order.index("worker"), order
assert order.index("worker") < order.index("client"), order
print("ok: versioned dev-dependencies publish before their consumers")

# Cargo strips a path-only dev-dependency from the published manifest. A
# private local test helper therefore does not prevent library publication.
result = run([
    package("helper", publish=[]),
    package("library", [dependency("helper", "dev", "*")]),
])
assert result.returncode == 0, result.stderr
assert result.stdout.splitlines() == ["library"], result.stdout
print("ok: path-only private test helpers stay outside publication")

# A versioned dev-dependency cannot be silently ignored when its prerequisite
# is private: packaging would ask the registry for an unpublished crate.
result = run([
    package("helper", publish=[]),
    package("library", [dependency("helper", "dev")]),
])
assert result.returncode != 0, result.stdout
assert not result.stdout, result.stdout
print("ok: versioned private prerequisites refuse publication")

# Versioned development edges can also make sequential publication impossible.
# Refuse the graph before printing any partial publication plan.
result = run([
    package("one", [dependency("two", "dev")]),
    package("two", [dependency("one")]),
])
assert result.returncode != 0, result.stdout
assert not result.stdout, result.stdout
print("ok: versioned dependency cycles refuse publication")
PY
