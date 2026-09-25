#!/usr/bin/env python3
"""VF19 mutant calibration registry checker (plans/0057 §9).

The registry (verification/mutants.json) maps every calibrated seam to
its named mutants, the exact invariant/test that must kill each one and
the lane that demonstrates the kill. This checker enforces both halves
of the VF19 contract:

- coverage: a seam that declares no mutants fails; every fault the model
  gates actually run must be registered (no phantom calibration);
- attribution: a registered kill must match the live gate/test — a
  mutant whose named killer is the wrong invariant, a missing file, a
  stale cfg or an unregistered gate fault all fail here. For the native
  (rust-native) seams `--execute` additionally RUNS the kill campaign:
  the named test must pass unarmed (a red baseline is a broken gate, not
  a kill) and fail armed (a green armed run means the mutant kills
  nothing — or something other than its named check). Compile errors,
  crashes and timeouts are broken gates, never kills.

Lanes: `model`/`tlaps` kills are demonstrated by the TLC/TLAPS gates
themselves (this checker binds the registry to their live text);
`core-assurance` native kills are executed here. Run:
  python3 verification/check_mutants.py [--execute] [--self-test]
"""

from __future__ import annotations

import json
import os
import re
import subprocess
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
REGISTRY = Path(__file__).resolve().parent / "mutants.json"
FIXTURES = Path(__file__).resolve().parent / "mutants_fixtures"
INVENTORY = Path(__file__).resolve().parent / "inventory.json"

SCHEMA_VERSION = 1
LANES = {"model", "tlaps", "core-assurance", "test", "verify", "container-test"}
KINDS = {"tla-fault", "tla-mutant-module", "tlaps-negative-control", "rust-native"}

# Broken-gate markers in an armed cargo run: tooling failures, not kills.
BROKEN_MARKERS = ("error[", "error: could not compile", "error: aborting")


def is_witness(label: str) -> bool:
    return "witness" in label.lower() or "$" in label


def gate_faults(text: str) -> list[dict]:
    """Every kill obligation a model-gate script runs (witnesses excluded):
    name, cfg, killing invariant/property, whether it is temporal."""
    faults = []
    for line in text.splitlines():
        s = line.strip()
        m = re.match(r'check_fault\s+"?([^"\s]+)"?\s+(\S+)\s+(\S+)\s+(\S+)', s)
        if m and not is_witness(m.group(1)):
            faults.append(
                {"name": m.group(1), "config": m.group(2),
                 "killed_by": m.group(4), "temporal": False}
            )
            continue
        m = re.match(
            r'expect_kills\s+"?([^"\s$]+)"?\s+(\S+)\s+(\S+)\s+no\s+"?([^"\s]+)"?', s)
        if m and not is_witness(m.group(1)):
            faults.append(
                {"name": m.group(1), "config": m.group(2),
                 "killed_by": m.group(4), "temporal": False}
            )
            continue
        m = re.match(
            r'expect_kills\s+"?([^"\s$]+)"?\s+(\S+)\s+(\S+)\s+([A-Za-z]\w*)\s*$', s)
        if m and not is_witness(m.group(1)):
            faults.append(
                {"name": m.group(1), "config": m.group(2),
                 "killed_by": m.group(4), "temporal": True}
            )
            continue
        m = re.match(r"plan_fault\s+(\d+)\s+(\S+)\s+(\S+)$", s)
        if m and not is_witness(m.group(3)):
            faults.append(
                {"name": m.group(3), "config": "specs/cfg/change-plan.cfg",
                 "killed_by": m.group(2), "temporal": False}
            )
    return faults


def model_gates() -> dict[str, str]:
    return {
        f"specs/{path.name}": path.read_text(encoding="utf-8")
        for path in (ROOT / "specs").glob("*-gate.sh")
    }


def validate(registry_path: Path, live_tree: bool) -> list[str]:
    errors = []
    try:
        registry = json.loads(registry_path.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        return [f"cannot read {registry_path}: {exc}"]
    if registry.get("version") != SCHEMA_VERSION:
        errors.append(f"registry version must be {SCHEMA_VERSION}")
    seams = registry.get("seams")
    if not isinstance(seams, list):
        return errors + ["registry.seams must be a list"]

    boundary_ids = set()
    if live_tree:
        inventory = json.loads(INVENTORY.read_text(encoding="utf-8"))
        boundary_ids = {row["id"] for row in inventory["boundaries"]}

    registered_faults = {}
    seen_seams = set()
    for seam in seams:
        sid = seam.get("id", "<missing id>")
        if sid in seen_seams:
            errors.append(f"seam {sid}: declared twice")
        seen_seams.add(sid)
        lane = seam.get("lane")
        if lane not in LANES:
            errors.append(f"seam {sid}: unknown lane {lane!r}")
        if live_tree and seam.get("boundary") not in boundary_ids:
            errors.append(
                f"seam {sid}: boundary {seam.get('boundary')!r} is not a "
                "VF01 inventory row"
            )
        mutants = seam.get("mutants")
        if not mutants:
            # Failure mode 1: an uncalibrated seam.
            errors.append(f"seam {sid}: declares no mutants")
            continue
        for mutant in mutants:
            name = mutant.get("name", "<missing name>")
            kind = mutant.get("kind")
            killed_by = mutant.get("killed_by")
            if kind not in KINDS:
                errors.append(f"{sid}/{name}: unknown kind {kind!r}")
                continue
            if not killed_by:
                errors.append(f"{sid}/{name}: no named killing invariant/test")
                continue
            if kind == "tla-fault":
                gate_rel = seam.get("gate", "")
                gate_path = ROOT / gate_rel
                if not gate_path.is_file():
                    errors.append(f"{sid}/{name}: gate {gate_rel} does not exist")
                    continue
                if seam.get("lane") != "model":
                    errors.append(f"{sid}/{name}: tla-fault kills live in the model lane")
                cfg_rel = mutant.get("config", "")
                cfg_path = ROOT / cfg_rel
                if not cfg_path.is_file():
                    errors.append(f"{sid}/{name}: config {cfg_rel} does not exist")
                elif "MUTATION" not in cfg_path.read_text(encoding="utf-8"):
                    errors.append(
                        f"{sid}/{name}: config {cfg_rel} selects no MUTATION — "
                        "the fault is not armed by its cfg"
                    )
                if len(killed_by) != 1:
                    errors.append(
                        f"{sid}/{name}: a tla-fault dies by exactly one invariant"
                    )
                    continue
                registered_faults[name] = {
                    "gate": gate_rel,
                    "config": cfg_rel,
                    "killed_by": killed_by[0],
                    "temporal": bool(mutant.get("temporal")),
                }
            elif kind in {"tla-mutant-module", "tlaps-negative-control"}:
                file_rel = mutant.get("file", "")
                if not (ROOT / file_rel).is_file():
                    errors.append(f"{sid}/{name}: module {file_rel} does not exist")
                gate_rel = seam.get("gate", "")
                gate_path = ROOT / gate_rel
                if not gate_path.is_file():
                    errors.append(f"{sid}/{name}: gate {gate_rel} does not exist")
                else:
                    text = gate_path.read_text(encoding="utf-8")
                    if name not in text:
                        errors.append(
                            f"{sid}/{name}: gate {gate_rel} never runs this mutant"
                        )
                    for invariant in killed_by:
                        if invariant not in text and invariant not in (
                            ROOT / file_rel
                        ).read_text(encoding="utf-8"):
                            errors.append(
                                f"{sid}/{name}: killing invariant {invariant} is "
                                "named in neither the gate nor the mutant"
                            )
            elif kind == "rust-native":
                file_rel = mutant.get("file", "")
                file_path = ROOT / file_rel
                if not file_path.is_file():
                    errors.append(f"{sid}/{name}: seam {file_rel} does not exist")
                elif name not in file_path.read_text(encoding="utf-8"):
                    errors.append(
                        f"{sid}/{name}: the mutant is not marked in its named seam "
                        f"{file_rel} — a stale registry entry is a broken gate"
                    )
                for target in killed_by:
                    crate = target.get("crate")
                    test = target.get("test")
                    crate_dir = ROOT / "crates" / str(crate)
                    if not crate_dir.is_dir():
                        errors.append(f"{sid}/{name}: crate {crate} does not exist")
                        continue
                    found = any(
                        re.search(rf"fn {re.escape(str(test))}\b", source)
                        for source in (
                            p.read_text(encoding="utf-8")
                            for p in crate_dir.rglob("*.rs")
                        )
                    )
                    if not found:
                        errors.append(
                            f"{sid}/{name}: kill test {test} does not exist in "
                            f"crate {crate}"
                        )
    if live_tree:
        # Reverse coverage: every kill obligation the model gates run must
        # be registered (an unregistered mutant is uncalibrated calibration).
        for gate_rel, text in model_gates().items():
            for fault in gate_faults(text):
                registered = registered_faults.get(fault["name"])
                if registered is None:
                    errors.append(
                        f"{gate_rel}: runs unregistered mutant {fault['name']}"
                    )
                    continue
                if (
                    registered["gate"] != gate_rel
                    or registered["config"] != fault["config"]
                    or registered["killed_by"] != fault["killed_by"]
                    or registered["temporal"] != fault["temporal"]
                ):
                    # Failure mode 2: the registry attributes the kill to
                    # the wrong invariant/cfg/gate.
                    errors.append(
                        f"mutant {fault['name']}: registry attributes the kill "
                        f"to {registered['killed_by']} via {registered['config']} "
                        f"({registered['gate']}), but {gate_rel} kills it by "
                        f"{fault['killed_by']} via {fault['config']}"
                    )
    return errors


def cargo_test(crate: str, test: str, mutant: str | None) -> tuple[str, str]:
    """One kill-campaign leg; returns (classification, output tail)."""
    env = dict(os.environ)
    if mutant is not None:
        env["STROP_MUTANT"] = mutant
        env["RUSTFLAGS"] = (env.get("RUSTFLAGS", "") + " --cfg strop_mutant").strip()
    else:
        env.pop("STROP_MUTANT", None)
    try:
        run = subprocess.run(
            ["cargo", "test", "--locked", "-p", crate, test],
            cwd=ROOT,
            env=env,
            capture_output=True,
            text=True,
            timeout=3600,
        )
    except subprocess.TimeoutExpired:
        return "broken", "the campaign timed out — a timeout is a broken gate"
    output = run.stdout + run.stderr
    if any(marker in output for marker in BROKEN_MARKERS):
        return "broken", output[-2000:]
    if re.search(r"test result: FAILED", output) and re.search(
        rf"{re.escape(test)}.*FAILED", output
    ):
        return "killed", output[-2000:]
    if re.search(r"test result: ok", output):
        return "survived", output[-2000:]
    return "broken", output[-2000:]


def execute(registry_path: Path) -> list[str]:
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    errors = []
    for seam in registry["seams"]:
        for mutant in seam["mutants"]:
            if mutant["kind"] != "rust-native":
                continue
            name = mutant["name"]
            for target in mutant["killed_by"]:
                crate, test = target["crate"], target["test"]
                verdict, detail = cargo_test(crate, test, None)
                if verdict != "survived":
                    errors.append(
                        f"{seam['id']}/{name}: BROKEN GATE — the unarmed kill "
                        f"test {test} does not pass; no kill is meaningful:\n"
                        f"{detail}"
                    )
                    continue
                verdict, detail = cargo_test(crate, test, name)
                if verdict == "killed":
                    print(f"kill: {seam['id']}/{name} dies by {test} (as registered)")
                elif verdict == "survived":
                    errors.append(
                        f"{seam['id']}/{name}: the armed mutant SURVIVES its named "
                        f"check {test} — it kills nothing, or something else"
                    )
                else:
                    errors.append(
                        f"{seam['id']}/{name}: BROKEN GATE — the armed run failed "
                        f"for tooling reasons, not the named check:\n{detail}"
                    )
    return errors


SELF_TEST = [
    ("missing-mutant.json", "declares no mutants"),
    ("wrong-invariant.json", "registry attributes the kill to"),
]


def self_test() -> int:
    failures = 0
    for fixture, expected in SELF_TEST:
        errors = validate(FIXTURES / fixture, live_tree=True)
        if any(expected in error for error in errors):
            print(f"self-test: {fixture} fails as required ({expected!r})")
        else:
            print(
                f"self-test FAIL: {fixture} must fail with {expected!r}; got: {errors}"
            )
            failures += 1
    return 1 if failures else 0


def main(argv) -> int:
    execute_mode = "--execute" in argv
    if "--self-test" in argv:
        return self_test()
    registry_path = REGISTRY
    if "--registry" in argv:
        registry_path = Path(argv[argv.index("--registry") + 1])
    errors = validate(registry_path, live_tree=True)
    if execute_mode and not errors:
        errors = execute(registry_path)
    for error in errors:
        print(f"FAIL: {error}")
    if errors:
        print(f"\nmutant registry check FAILED: {len(errors)} failure(s)")
        return 1
    registry = json.loads(registry_path.read_text(encoding="utf-8"))
    obligations = sum(len(s["mutants"]) for s in registry["seams"])
    print(
        f"mutant registry ok: {len(registry['seams'])} seams, "
        f"{obligations} attributed kill obligations"
        + (", native kills executed" if execute_mode else "")
    )
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
