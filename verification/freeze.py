#!/usr/bin/env python3
"""VF01 candidate freeze (0057 §2): record the exact candidate under
verification into verification/candidate.json.

Recorded, all from the live tree — never from memory:

  * git commit, tree id, commit date, and any dirty paths (a frozen
    candidate must be exact; dirt is recorded, and refused without
    --allow-dirty);
  * Cargo.lock digest (locked-dependency identity);
  * inventory digest (verification/inventory.json — the boundary/claim
    set this candidate is qualified against);
  * shipped helper digests: every Python helper plus the Rust files that
    embed Python source (the supervisor SOURCE and the interpreter
    probe), so "the code that ships is the code the campaign exercises";
  * Dockerfile stage pins: every external base image as image:tag@sha256
    per stage (AR15 convention), plus the checksum-pinned tool downloads
    (tla2tools, Verus, Z3, TLAPS) parsed from the same Dockerfile;
  * Zig bootstrap pin (version + per-platform digests) from
    .github/scripts/install-zig.sh;
  * the verus_builtin crate pins from crates/strop-core/Cargo.toml.

Boring tools only: python3 stdlib + git + sha256 of files.

Usage:
  python3 verification/freeze.py [--allow-dirty] [--out PATH]
  python3 verification/freeze.py --check   # candidate.json matches tree?
"""

from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = Path(__file__).resolve().parent / "candidate.json"

# The shipped helper bundle (0057 §5 "The Python supervisor/editor
# boundary is mandatory"): every helper source plus its Rust carriers.
HELPER_SOURCES = [
    "crates/strop-remote/src/protected.py",
    "crates/strop-remote/src/save/helper.py",
    "crates/strop-remote/src/filesystem/main.py",
    "crates/strop-remote/src/filesystem/observe.py",
    "crates/strop-remote/src/filesystem/mutate.py",
    "crates/strop-remote/src/filesystem/cleanup.py",
    # Rust carriers of embedded Python: the supervisor SOURCE and the
    # interpreter capability PROBE ship as string literals in these files.
    "crates/strop-remote/src/exec/supervisor.rs",
    "crates/strop-remote/src/exec/python.rs",
]

SCHEMA = 1


def sha256_file(rel: str) -> str:
    data = (ROOT / rel).read_bytes()
    return hashlib.sha256(data).hexdigest()


def git(*args: str) -> str:
    out = subprocess.run(
        ["git", *args], cwd=ROOT, check=True, capture_output=True, text=True
    )
    return out.stdout.strip()


def dockerfile_pins(text: str) -> dict:
    stages = {}
    for m in re.finditer(r"(?m)^FROM\s+(\S+?)(?:\s+AS\s+(\S+))?\s*$", text):
        image, name = m.group(1), m.group(2)
        if name and "@" in image:  # external pinned base (AR15 convention)
            stages[name] = image
    checksums = {}
    for m in re.finditer(
        r"(?m)^ADD\s+--checksum=sha256:\$\{(\w+)\}\s+(\S+)", text
    ):
        arg, url = m.group(1), m.group(2)
        am = re.search(rf"(?m)^ARG\s+{arg}=(\S+)\s*$", text)
        checksums[arg] = {
            "url": url,
            "sha256": am.group(1) if am else None,
        }
    return {"stage_base_images": stages, "checksum_pinned_downloads": checksums}


def zig_pins(text: str) -> dict:
    version = re.search(r"ziglang\.org/download/([^/]+)/", text)
    digests = dict(re.findall(r'(\w+-\w+);\s*digest=([0-9a-f]{64})', text))
    return {"version": version.group(1) if version else None, "digests": digests}


def cargo_toml_pins(text: str) -> dict:
    pins = {}
    for name in ("verus_builtin", "verus_builtin_macros"):
        m = re.search(rf'(?m)^{name}\s*=\s*"([^"]+)"', text)
        if m:
            pins[name] = m.group(1)
    return pins


def freeze(allow_dirty: bool) -> dict:
    commit = git("rev-parse", "HEAD")
    dirty = git("status", "--porcelain").splitlines()
    dirty = sorted(line[3:] for line in dirty if line.strip())
    if dirty and not allow_dirty:
        raise SystemExit(
            "refusing to freeze a dirty tree (a candidate is exact); "
            "dirty paths:\n  " + "\n  ".join(dirty) + "\n"
            "commit first, or pass --allow-dirty to record the dirt honestly"
        )
    dockerfile = (ROOT / "Dockerfile").read_text(encoding="utf-8")
    pins = dockerfile_pins(dockerfile)
    candidate = {
        "schema": SCHEMA,
        "release": "0057",
        "plan": "plans/0057-core-verification-and-assurance.md",
        "commit": commit,
        "tree": git("rev-parse", "HEAD^{tree}"),
        # Deterministic: the commit's own date, not the wall clock.
        "commit_date": git("show", "-s", "--format=%cI", "HEAD"),
        "dirty": dirty,
        "cargo_lock_sha256": sha256_file("Cargo.lock"),
        "inventory_sha256": sha256_file("verification/inventory.json"),
        "helper_sources": {rel: sha256_file(rel) for rel in HELPER_SOURCES},
        "install_transaction": {
            "install.sh": sha256_file("install.sh"),
            "crates/strop/src/update.rs": sha256_file("crates/strop/src/update.rs"),
            ".github/scripts/release-catalog.py": sha256_file(
                ".github/scripts/release-catalog.py"
            ),
        },
        "dockerfile_sha256": sha256_file("Dockerfile"),
        "dockerfile_stages": pins["stage_base_images"],
        "checksum_pinned_downloads": pins["checksum_pinned_downloads"],
        "zig_bootstrap": zig_pins(
            (ROOT / ".github/scripts/install-zig.sh").read_text(encoding="utf-8")
        ),
        "verus_crate_pins": cargo_toml_pins(
            (ROOT / "crates/strop-core/Cargo.toml").read_text(encoding="utf-8")
        ),
    }
    return candidate


def check(path: Path) -> int:
    try:
        recorded = json.loads(path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: cannot read {path}: {exc}")
        return 1
    problems = []
    if recorded.get("commit") != git("rev-parse", "HEAD"):
        problems.append("commit does not match HEAD")
    if recorded.get("cargo_lock_sha256") != sha256_file("Cargo.lock"):
        problems.append("Cargo.lock drifted from the freeze")
    if recorded.get("inventory_sha256") != sha256_file("verification/inventory.json"):
        problems.append("inventory drifted from the freeze")
    for rel, digest in (recorded.get("helper_sources") or {}).items():
        if sha256_file(rel) != digest:
            problems.append(f"helper source drifted: {rel}")
    for section in ("install_transaction",):
        for rel, digest in (recorded.get(section) or {}).items():
            if sha256_file(rel) != digest:
                problems.append(f"{section} file drifted: {rel}")
    if recorded.get("dockerfile_sha256") != sha256_file("Dockerfile"):
        problems.append("Dockerfile drifted from the freeze")
    if problems:
        for p in problems:
            print(f"FAIL: {p}")
        return 1
    print(f"candidate check ok: {recorded['commit'][:12]} matches the tree")
    return 0


def main(argv) -> int:
    allow_dirty = False
    out = DEFAULT_OUT
    mode = "freeze"
    i = 1
    while i < len(argv):
        arg = argv[i]
        if arg == "--allow-dirty":
            allow_dirty = True
        elif arg == "--check":
            mode = "check"
        elif arg == "--out":
            i += 1
            if i >= len(argv):
                print("usage: --out PATH", file=sys.stderr)
                return 2
            out = Path(argv[i])
        else:
            print(__doc__, file=sys.stderr)
            return 2
        i += 1
    if mode == "check":
        return check(out)
    candidate = freeze(allow_dirty)
    out.write_text(
        json.dumps(candidate, indent=2, sort_keys=True) + "\n", encoding="utf-8"
    )
    print(f"froze candidate {candidate['commit'][:12]} -> {out}")
    if candidate["dirty"]:
        print(f"WARNING: tree is dirty ({len(candidate['dirty'])} paths recorded)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
