#!/usr/bin/env python3
"""0059 exact committed completion candidate, stored outside its own source tree.

Bind the complete Rust/build source set, dependency lock, assurance inventory,
raw current completion measurements, immutable 0057/0058 history, installation
transaction and pinned toolchains. Historical measurements never qualify a
changed live source. Native/Compose lane execution remains a release gate.

Usage:
  python3 verification/freeze.py [--allow-dirty] [--out PATH]
  python3 verification/freeze.py --check
"""
from __future__ import annotations

import hashlib
import json
import re
import subprocess
import sys
import tomllib
from pathlib import Path

# Evidence readers must leave the candidate checkout unchanged.
sys.dont_write_bytecode = True

from completion_evidence import source_files, validate as completion_evidence

ROOT = Path(__file__).resolve().parent.parent
DEFAULT_OUT = ROOT / "dist" / "completion-candidate.json"
SCHEMA = 11
BASELINE = {
    "candidate_sha256": "b7f3045cdf94f4b97220a4e7aa71abc131747623c81c8b5f81e2fdfad20d1307",
    "inventory_sha256": "40c0b816e9b91cdc8f2357d88a7cdfab1c26982884088d2166e589711fc1b798",
    "helper_digests_sha256": "7067e2969445da4876668744300d736bb3c9320ece0d8aed7be98a560ad49414",
    "linux_candidate_sha256": "15ab49230ad7f76a6dc5f0901f07ac87e0f43f25e8d8388dea6a2c8b04aed7b2",
    "linux_inventory_sha256": "5bd0ad567a28e8562edaa4d5f12879a8b68f07af6b25d2c18c91ac83a8adde61",
    "linux_evidence_sha256": "2f0fa785f992b904da6f5796d424d3eee3972bf1485c4565dd54940a526735ba",
}
BASELINE_PATHS = {
    "candidate_sha256": "verification/baseline/0057-candidate.json",
    "inventory_sha256": "verification/baseline/0057-inventory.json",
    "helper_digests_sha256": "verification/baseline/0057-helper-digests.json",
    "linux_candidate_sha256": "verification/baseline/0057-linux-candidate.json",
    "linux_inventory_sha256": "verification/baseline/0057-linux-inventory.json",
    "linux_evidence_sha256": "verification/baseline/0057-linux-evidence.json",
}


def sha256_file(rel: str) -> str:
    return hashlib.sha256((ROOT / rel).read_bytes()).hexdigest()


def git(*args: str) -> str:
    return subprocess.run(["git", *args], cwd=ROOT, check=True,
                          capture_output=True, text=True).stdout.rstrip("\n")


def dockerfile_pins(text: str) -> dict:
    stages = {}
    for match in re.finditer(r"(?m)^FROM\s+(\S+?)(?:\s+AS\s+(\S+))?\s*$", text):
        image, name = match.group(1), match.group(2)
        if name and "@" in image:
            stages[name] = image
    checksums = {}
    for match in re.finditer(r"(?m)^ADD\s+--checksum=sha256:\$\{(\w+)\}\s+(\S+)", text):
        arg, url = match.group(1), match.group(2)
        value = re.search(rf"(?m)^ARG\s+{arg}=(\S+)\s*$", text)
        checksums[arg] = {"url": url, "sha256": value.group(1) if value else None}
    return {"stage_base_images": stages, "checksum_pinned_downloads": checksums}


def zig_pins(text: str) -> dict:
    version = re.search(r"ziglang\.org/download/([^/]+)/", text)
    digests = dict(re.findall(r'(\w+-\w+);\s*digest=([0-9a-f]{64})', text))
    return {"version": version.group(1) if version else None, "digests": digests}


def cargo_toml_pins(text: str) -> dict:
    pins = {}
    for name in ("verus_builtin", "verus_builtin_macros"):
        match = re.search(rf'(?m)^{name}\s*=\s*"([^"]+)"', text)
        if match:
            pins[name] = match.group(1)
    return pins


def baseline_hashes() -> dict:
    actual = {key: sha256_file(path) for key, path in BASELINE_PATHS.items()}
    if actual != BASELINE:
        raise SystemExit("pre-worker VF20 archive changed; refuse to re-freeze historical evidence")
    scoped = json.loads((ROOT / BASELINE_PATHS["linux_evidence_sha256"]).read_text())
    candidate = json.loads((ROOT / BASELINE_PATHS["linux_candidate_sha256"]).read_text())
    if (scoped.get("qualification") != "linux-x86_64-local-evidence-only"
            or candidate.get("dirty") or scoped.get("commit") != candidate.get("commit")
            or scoped.get("tree") != candidate.get("tree")
            or scoped.get("helper_digests_sha256") != actual["helper_digests_sha256"]):
        raise SystemExit("Linux-only pre-worker evidence is inconsistent with its clean source")
    for lane in scoped["gates"].values():
        if lane.get("status") != "passed" or sha256_file(lane["log"]) != lane["sha256"]:
            raise SystemExit("Linux-only pre-worker raw gate evidence changed")
    return actual


def worker_baseline_hashes() -> dict:
    """The released worker evidence is immutable history, not current proof."""
    path = "verification/baseline/0058-measurements.json"
    expected = "affb2d65f97e6ed0d8faeb4759869f6c9989fb8263973e8b1b3453f1f0c97ff6"
    if sha256_file(path) != expected:
        raise SystemExit("0058 worker evidence archive changed")
    archive = json.loads((ROOT / path).read_text())
    inventory = "verification/baseline/0058-inventory.json"
    if sha256_file(inventory) != archive["inventory_sha256"]:
        raise SystemExit("0058 archived worker inventory changed")
    actual = {file: sha256_file(file) for file in archive["measurements"]}
    if actual != archive["measurements"]:
        raise SystemExit("0058 historical worker measurements changed")
    return {"manifest_sha256": expected, "inventory_sha256": archive["inventory_sha256"],
            "release_commit": archive["release_commit"], "measurements": actual}


def freeze(allow_dirty: bool) -> dict:
    commit = git("rev-parse", "HEAD")
    dirty = sorted(line[3:] for line in git("status", "--porcelain").splitlines() if line.strip())
    if dirty and not allow_dirty:
        raise SystemExit("refusing to freeze a dirty tree (a candidate is exact); dirty paths:\n  "
                         + "\n  ".join(dirty) + "\ncommit first, or record dirt with --allow-dirty")
    pins = dockerfile_pins((ROOT / "Dockerfile").read_text())
    return {
        "schema": SCHEMA, "release": "0059", "plan": "plans/0059-nonblocking-code-completion.md",
        "version": tomllib.loads((ROOT / "Cargo.toml").read_text())["workspace"]["package"]["version"],
        "commit": commit, "tree": git("rev-parse", "HEAD^{tree}"),
        "commit_date": git("show", "-s", "--format=%cI", "HEAD"), "dirty": dirty,
        "cargo_lock_sha256": sha256_file("Cargo.lock"),
        "inventory_sha256": sha256_file("verification/inventory.json"),
        "sources": source_files(ROOT), "completion_evidence": completion_evidence(ROOT),
        "baseline": {"pre_worker": baseline_hashes(), "worker": worker_baseline_hashes()},
        "install_transaction": {path: sha256_file(path) for path in (
            "install.sh", "crates/strop/src/update.rs", ".github/scripts/release-catalog.py")},
        "qualification_workflows": {path: sha256_file(path) for path in (
            ".github/workflows/ci.yml", ".github/workflows/release.yml")},
        "dockerfile_sha256": sha256_file("Dockerfile"),
        "dockerfile_stages": pins["stage_base_images"],
        "checksum_pinned_downloads": pins["checksum_pinned_downloads"],
        "zig_bootstrap": zig_pins((ROOT / ".github/scripts/install-zig.sh").read_text()),
        "verus_crate_pins": cargo_toml_pins((ROOT / "crates/strop-core/Cargo.toml").read_text()),
    }


def check(path: Path) -> int:
    try:
        recorded = json.loads(path.read_text())
    except (OSError, json.JSONDecodeError) as error:
        print(f"FAIL: cannot read {path}: {error}")
        return 1
    if recorded.get("schema") != SCHEMA or recorded.get("release") != "0059":
        print("FAIL: not a completion candidate; 0057/0058 candidates are historical archives")
        return 1
    current = freeze(allow_dirty=True)
    problems = []
    if recorded.get("dirty") or current["dirty"]:
        problems.append("candidate or current worktree is dirty; not release-qualified")
    if recorded.keys() != current.keys():
        problems.append("candidate fields differ from the current freeze contract")
    for field, value in current.items():
        if recorded.get(field) != value:
            problems.append(f"{field} differs from the frozen source inputs")
    for problem in problems:
        print(f"FAIL: {problem}")
    if problems:
        return 1
    print(f"candidate check ok: {recorded['commit'][:12]} matches the tree")
    return 0


def main(argv) -> int:
    allow_dirty = False
    out = DEFAULT_OUT
    mode = "freeze"
    index = 1
    while index < len(argv):
        arg = argv[index]
        if arg == "--allow-dirty":
            allow_dirty = True
        elif arg == "--check":
            mode = "check"
        elif arg == "--out":
            index += 1
            if index >= len(argv):
                print("usage: --out PATH", file=sys.stderr)
                return 2
            out = Path(argv[index])
        else:
            print(__doc__, file=sys.stderr)
            return 2
        index += 1
    if mode == "check":
        return check(out)
    candidate = freeze(allow_dirty)
    out.parent.mkdir(parents=True, exist_ok=True)
    out.write_text(json.dumps(candidate, indent=2, sort_keys=True) + "\n")
    print(f"froze candidate {candidate['commit'][:12]} -> {out}")
    if candidate["dirty"]:
        print(f"WARNING: tree is dirty ({len(candidate['dirty'])} paths recorded)")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
