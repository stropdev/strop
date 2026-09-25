#!/usr/bin/env python3
"""VF01 whole-core boundary/claim inventory checker (0057).

Reads verification/inventory.json and enforces:

  * structure: every boundary row carries id/kind/promise/exclusions/
    authoritative state/trust assumptions/owning files/claims/gates, and
    every claim carries a status plus at least one evidence pointer;
  * liveness: every evidence pointer resolves — test and proof symbols
    must name functions that exist in the referenced file, models must
    exist together with their TLC configs, scripts must exist;
  * completeness: the VF01-required boundary families are all present;
    a boundary missing from the inventory is a gate failure;
  * hash binding: each row pins the sha256 of its owning source files
    and a digest over its claims+evidence content (named Rust test/proof
    function bodies, whole TLA+/cfg/script files — never mtimes). Check
    mode fails on any drift; --stamp re-pins after validating evidence
    and REFUSES a row whose sources moved but whose evidence digest is
    unchanged (that is the "source changed without evidence changing"
    failure, enforced mechanically). --force <id> overrides per row for
    reviewed no-op extractions; the override is recorded in the output.

Boring tools only: python3 stdlib. Run from anywhere; the repo root is
derived from this file's location.

Usage:
  python3 verification/check.py            # CI mode: validate + verify pins
  python3 verification/check.py --stamp    # re-pin after re-reviewing rows
  python3 verification/check.py --stamp --force mutation-gateway
"""

from __future__ import annotations

import hashlib
import json
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
INVENTORY = Path(__file__).resolve().parent / "inventory.json"

SCHEMA_VERSION = 1

# Claim statuses, per plans/0057 §2. Not interchangeable.
STATUSES = {
    "specified",
    "bounded-model-checked",
    "proved-kernel",
    "proved-theorem",
    "correspondence-tested",
    "native-tested",
    "blocked",
    "superseded",
}

# Gates that can run evidence. Each names an existing runnable lane.
GATES = {
    "test",            # docker compose run --build --rm test
    "model",           # docker compose run --build --rm model (specs/gate.sh)
    "verify",          # docker compose run --build --rm verify (Verus)
    "tlaps",           # docker compose run --build --rm tlaps
    "container-test",  # docker compose run --build --rm container-test
    "core-assurance",  # docker compose run --build --rm core-assurance (VF19)
    "install-fixture", # sh tests/install.sh + sh tests/release-catalog.sh
    "release",         # .github/workflows/release.yml
}

EVIDENCE_KINDS = {"test", "proof", "model", "script"}

BOUNDARY_KINDS = {
    "mutation",     # can mutate text/state
    "admission",    # decides which input/action/view is admitted
    "authority",    # grants/refuses capability or identity
    "io",           # performs filesystem/wire effects
    "persistence",  # preserves/recovers data
    "transport",    # moves events/bytes between owners
    "lifecycle",    # owns spawn/supervision/shutdown of work
}

# The VF01 boundary families (plans/0057 §2 + the domain chapters). A
# named family missing from the inventory fails the gate. Additional
# rows beyond these are welcome; none of these may be dropped.
REQUIRED_BOUNDARIES = {
    "mutation-gateway",
    "action-admission",
    "recovery-store",
    "privacy-classifier",
    "event-transport",
    "lsp-wire",
    "container-admission",
    "ssh-sftp",
    "exec-supervision",
    "session-persistence",
    "install-update",
    "terminal-lifecycle",
    "search-lifecycle",
    "ui-protocol",
    # 0058 native worker wave.
    "worker-protocol",
    "worker-serve",
    "fs-notify",
}

HEX64 = re.compile(r"^[0-9a-f]{64}$")


class Failure(Exception):
    pass


def sha256_bytes(data: bytes) -> str:
    return hashlib.sha256(data).hexdigest()


def read_file(rel: str) -> bytes:
    path = ROOT / rel
    if not path.is_file():
        raise Failure(f"missing file: {rel}")
    return path.read_bytes()


# ---------------------------------------------------------------- Rust
# Comment/string-aware masking so evidence binds to real function bodies,
# never to braces inside strings or comments.


def rust_code_mask(text: str) -> bytearray:
    """bytearray: 1 where the byte is real code, 0 inside comments/strings."""
    mask = bytearray(len(text))
    i, n = 0, len(text)
    in_code = True
    while i < n:
        c = text[i]
        if text.startswith("//", i):
            j = text.find("\n", i)
            i = n if j == -1 else j + 1
            continue
        if text.startswith("/*", i):
            depth, j = 1, i + 2
            while j < n and depth:
                if text.startswith("/*", j):
                    depth, j = depth + 1, j + 2
                elif text.startswith("*/", j):
                    depth, j = depth - 1, j + 2
                else:
                    j += 1
            i = j
            continue
        # raw string: r"..." / r#"..."# / r##"..."## ...
        m = re.match(r'r(#+)"', text[i:])
        if m:
            hashes = m.group(1)
            end = text.find('"' + hashes, i + 1 + len(hashes) + 1)
            i = n if end == -1 else end + 1 + len(hashes)
            continue
        if c == '"':
            j = i + 1
            while j < n:
                if text[j] == "\\":
                    j += 2
                    continue
                if text[j] == '"':
                    j += 1
                    break
                j += 1
            i = j
            continue
        if c == "'":
            # char literal, not a lifetime: 'x', '\n', '\'', '\u{1F600}'
            m = re.match(
                r"'(?:\\(?:[\\nrt0'\"]|x[0-9a-fA-F]{2}|u\{[0-9a-fA-F]+\})|[^\\'])'",
                text[i:],
            )
            if m:
                i += m.end()
                continue
            mask[i] = 1
            i += 1
            continue
        mask[i] = 1
        i += 1
    return mask


def rust_fn_source(text: str, mask: bytearray, name: str):
    """Source of `fn name(...)` through its matching brace, or None."""
    for m in re.finditer(r"\bfn\s+" + re.escape(name) + r"\b", text):
        if not mask[m.start()]:
            continue
        i = m.end()
        while i < len(text) and not (mask[i] and text[i] == "{"):
            i += 1
        if i >= len(text):
            return None  # declaration without body — not evidence
        depth, j = 1, i + 1
        while j < len(text) and depth:
            if mask[j]:
                if text[j] == "{":
                    depth += 1
                elif text[j] == "}":
                    depth -= 1
            j += 1
        if depth:
            return None
        return text[m.start():j]
    return None


# ------------------------------------------------------------- evidence


def evidence_content_hash(entry: dict) -> str:
    """Content binding for one evidence entry, from live tree bytes."""
    kind = entry["kind"]
    data = read_file(entry["path"])
    h = hashlib.sha256()
    if kind == "test" or (kind == "proof" and entry["path"].endswith(".rs")):
        names = entry.get("tests") or entry.get("symbols") or []
        text = data.decode("utf-8")
        mask = rust_code_mask(text)
        for name in sorted(names):
            body = rust_fn_source(text, mask, name)
            if body is None:
                raise Failure(
                    f"{entry['path']}: no function named `{name}` "
                    f"(evidence pointer for kind={kind})"
                )
            h.update(name.encode())
            h.update(b"\0")
            h.update(body.encode())
            h.update(b"\0")
    else:
        # proof (.tla), model, script: bind the whole artifact.
        h.update(data)
        config = entry.get("config")
        if config:
            h.update(read_file(config))
    return h.hexdigest()


def row_evidence_digest(row: dict) -> str:
    """Digest over the row's claims: statement, status and the CONTENT of
    every evidence pointer (named fn bodies / whole model/proof/script
    files). Editing claim text or any evidence body changes this digest."""
    claims = []
    for claim in sorted(row["claims"], key=lambda c: c["id"]):
        entries = []
        for entry in claim["evidence"]:
            entries.append(
                {
                    "kind": entry["kind"],
                    "path": entry["path"],
                    "gate": entry["gate"],
                    "config": entry.get("config"),
                    "tests": sorted(entry.get("tests") or []),
                    "symbols": sorted(entry.get("symbols") or []),
                    "content": evidence_content_hash(entry),
                }
            )
        claims.append(
            {
                "id": claim["id"],
                "statement": claim["statement"],
                "status": claim["status"],
                "evidence": entries,
            }
        )
    return sha256_bytes(
        json.dumps(claims, sort_keys=True, separators=(",", ":")).encode()
    )


def row_source_digests(row: dict) -> dict:
    return {rel: sha256_bytes(read_file(rel)) for rel in sorted(row["files"])}


# ------------------------------------------------------------ validation


def validate_row(row: dict, errors: list) -> None:
    rid = row.get("id", "<missing id>")
    for key in (
        "id",
        "kind",
        "title",
        "promise",
        "exclusions",
        "authoritative_state",
        "trusted",
        "files",
        "claims",
        "gates",
    ):
        if key not in row:
            errors.append(f"{rid}: missing required key `{key}`")
    if errors:
        return
    if row["kind"] not in BOUNDARY_KINDS:
        errors.append(f"{rid}: unknown kind {row['kind']!r}")
    if not isinstance(row["trusted"], list) or not row["trusted"]:
        errors.append(f"{rid}: `trusted` must name the trust assumptions")
    if not row["files"]:
        errors.append(f"{rid}: a boundary row must own at least one file")
    for rel in row["files"]:
        if not (ROOT / rel).is_file():
            errors.append(f"{rid}: owning file does not exist: {rel}")
    unknown_gates = set(row["gates"]) - GATES
    if unknown_gates:
        errors.append(f"{rid}: unknown gates {sorted(unknown_gates)}")
    if not row["claims"]:
        errors.append(f"{rid}: a boundary without claims is not inventoried")

    seen_claims = set()
    for claim in row["claims"]:
        cid = f"{rid}/{claim.get('id', '<missing id>')}"
        if claim.get("id") in seen_claims:
            errors.append(f"{cid}: duplicate claim id")
        seen_claims.add(claim.get("id"))
        for key in ("id", "statement", "status", "evidence"):
            if key not in claim:
                errors.append(f"{cid}: missing required key `{key}`")
        if claim.get("status") not in STATUSES:
            errors.append(f"{cid}: unknown status {claim.get('status')!r}")
        evidence = claim.get("evidence") or []
        if not evidence:
            errors.append(f"{cid}: claim without evidence (a gate failure)")
        for entry in evidence:
            kind = entry.get("kind")
            if kind not in EVIDENCE_KINDS:
                errors.append(f"{cid}: unknown evidence kind {kind!r}")
                continue
            path = entry.get("path", "")
            if not (ROOT / path).is_file():
                errors.append(f"{cid}: evidence file does not exist: {path}")
                continue
            gate = entry.get("gate")
            if gate not in GATES:
                errors.append(f"{cid}: evidence gate {gate!r} is not a lane")
            elif gate not in row["gates"]:
                errors.append(
                    f"{cid}: evidence runs in gate {gate!r} but the row "
                    f"does not list that gate"
                )
            names = entry.get("tests") or entry.get("symbols") or []
            if kind == "test" and not entry.get("tests"):
                errors.append(f"{cid}: test evidence must name test functions")
            if kind == "proof" and path.endswith(".rs") and not entry.get("symbols"):
                errors.append(
                    f"{cid}: Rust proof evidence must name proved symbols"
                )
            if kind == "model" and not entry.get("config"):
                errors.append(f"{cid}: model evidence must name its TLC config")
            if entry.get("config") and not (ROOT / entry["config"]).is_file():
                errors.append(f"{cid}: config does not exist: {entry['config']}")
            # Resolve the content binding: this is what makes a renamed or
            # deleted test/proof function a dangling-pointer failure.
            if names or kind in ("model", "script", "proof"):
                try:
                    evidence_content_hash(entry)
                except Failure as exc:
                    errors.append(f"{cid}: {exc}")


def validate_inventory(inv: dict) -> list:
    errors = []
    if inv.get("schema") != SCHEMA_VERSION:
        errors.append(f"schema must be {SCHEMA_VERSION}")
        return errors
    rows = inv.get("boundaries")
    if not isinstance(rows, list) or not rows:
        errors.append("inventory has no boundary rows")
        return errors
    seen = set()
    for row in rows:
        rid = row.get("id")
        if rid in seen:
            errors.append(f"duplicate boundary id {rid!r}")
        seen.add(rid)
        validate_row(row, errors)
    missing = REQUIRED_BOUNDARIES - seen
    if missing:
        errors.append(
            "required VF01 boundary families missing from the inventory: "
            + ", ".join(sorted(missing))
        )
    return errors


# ----------------------------------------------------------------- modes


def check(inv: dict) -> int:
    errors = validate_inventory(inv)
    drift = []
    if not errors:
        for row in inv["boundaries"]:
            rid = row["id"]
            pins = row.get("pins")
            if not pins:
                drift.append((rid, "unpinned row — run check.py --stamp"))
                continue
            recorded_sources = pins.get("sources") or {}
            if sorted(recorded_sources) != sorted(row["files"]):
                drift.append((rid, "pins.sources does not match the file set"))
                continue
            if not all(HEX64.match(v) for v in recorded_sources.values()):
                drift.append((rid, "pins.sources contains a non-sha256 value"))
                continue
            if not HEX64.match(pins.get("evidence", "")):
                drift.append((rid, "pins.evidence is missing or malformed"))
                continue
            actual_sources = row_source_digests(row)
            actual_evidence = row_evidence_digest(row)
            source_drift = sorted(
                rel
                for rel in row["files"]
                if recorded_sources[rel] != actual_sources[rel]
            )
            evidence_drift = actual_evidence != pins["evidence"]
            if source_drift and not evidence_drift:
                drift.append(
                    (
                        rid,
                        "BOUNDARY SOURCE CHANGED WITHOUT EVIDENCE CHANGE: "
                        + ", ".join(source_drift)
                        + " — re-review the row's claims/evidence, then "
                        "`check.py --stamp` (which will refuse until the "
                        "evidence moves or --force records a reviewed "
                        "no-op extraction)",
                    )
                )
            elif source_drift and evidence_drift:
                drift.append(
                    (
                        rid,
                        "sources and evidence both drifted from the pins: "
                        + ", ".join(source_drift)
                        + " — re-review, then `check.py --stamp`",
                    )
                )
            elif evidence_drift:
                drift.append(
                    (
                        rid,
                        "evidence drifted from its pin (test/proof/model "
                        "content or claim text changed) — re-review, then "
                        "`check.py --stamp`",
                    )
                )
    for message in errors:
        print(f"FAIL: {message}")
    for rid, message in drift:
        print(f"FAIL: {rid}: {message}")
    if errors or drift:
        print(
            f"\ninventory check FAILED: {len(errors)} structural, "
            f"{len(drift)} pin failure(s)"
        )
        return 1
    rows = inv["boundaries"]
    claims = sum(len(r["claims"]) for r in rows)
    print(
        f"inventory check ok: {len(rows)} boundaries, {claims} claims, "
        f"all evidence live, all pins current"
    )
    return 0


def stamp(inv: dict, force: set) -> int:
    errors = validate_inventory(inv)
    if errors:
        for message in errors:
            print(f"FAIL: {message}")
        print("stamp refused: fix the evidence pointers first")
        return 1
    refused = []
    stamped = []
    for row in inv["boundaries"]:
        rid = row["id"]
        actual_sources = row_source_digests(row)
        actual_evidence = row_evidence_digest(row)
        pins = row.get("pins")
        if pins is None:
            row["pins"] = {"sources": actual_sources, "evidence": actual_evidence}
            stamped.append(f"{rid} (new row)")
            continue
        source_drift = any(
            pins["sources"].get(rel) != digest
            for rel, digest in actual_sources.items()
        ) or sorted(pins["sources"]) != sorted(actual_sources)
        evidence_same = pins.get("evidence") == actual_evidence
        if source_drift and evidence_same and rid not in force:
            refused.append(rid)
            continue
        if source_drift or not evidence_same:
            row["pins"] = {"sources": actual_sources, "evidence": actual_evidence}
            note = " [FORCED: reviewed no-op extraction]" if rid in force else ""
            stamped.append(f"{rid}{note}")
    if refused:
        for rid in refused:
            print(
                f"REFUSED: {rid}: boundary sources changed but the row's "
                f"evidence digest is unchanged. Update the claims/evidence "
                f"to reflect the change, or re-run with --force {rid} for a "
                f"reviewed no-op extraction (and say so in the commit)."
            )
        return 1
    INVENTORY.write_text(
        json.dumps(inv, indent=2, ensure_ascii=False) + "\n", encoding="utf-8"
    )
    if stamped:
        print("stamped:")
        for line in stamped:
            print(f"  {line}")
    else:
        print("all pins already current")
    return 0


def main(argv) -> int:
    force = set()
    mode = "check"
    i = 1
    while i < len(argv):
        arg = argv[i]
        if arg == "--stamp":
            mode = "stamp"
        elif arg == "--force":
            i += 1
            if i >= len(argv):
                print("usage: --force BOUNDARY_ID", file=sys.stderr)
                return 2
            force.add(argv[i])
        else:
            print(__doc__, file=sys.stderr)
            return 2
        i += 1
    try:
        inv = json.loads(INVENTORY.read_text(encoding="utf-8"))
    except (OSError, json.JSONDecodeError) as exc:
        print(f"FAIL: cannot read {INVENTORY}: {exc}")
        return 1
    if mode == "stamp":
        return stamp(inv, force)
    if force:
        print("--force only applies to --stamp", file=sys.stderr)
        return 2
    return check(inv)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
