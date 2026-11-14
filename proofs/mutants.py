#!/usr/bin/env python3
"""Mutant calibration for the `proofs` gate (plans/0086 §8).

Every moved kernel function gets at least one semantic mutation of its Rust
source. For each: copy the crate, apply the mutation, translate it exactly as
the gate does (proofs/extract.sh), build the theorems against the mutated
translation in a scratch copy of the lake project, and require the build to
FAIL inside the named theorem(s) — no other theorem may fail, and the
translation itself must still type-check. A proof that survives its mutant
proves nothing; a failure anywhere else is noise, not calibration.
"""
from concurrent.futures import ThreadPoolExecutor
from dataclasses import dataclass
from pathlib import Path
import os
import re
import shutil
import subprocess
import sys
import tempfile

ROOT = Path(__file__).resolve().parent.parent
PROOFS = ROOT / "proofs"
KERNEL = ROOT / "crates" / "strop-kernel"


@dataclass(frozen=True)
class Mutant:
    name: str
    file: str
    before: str
    after: str
    fails: tuple[str, ...]


MUTANTS = (
    # an entry computed for another version of its subject is never served
    Mutant("cache-version-ignored", "cache.rs",
           "    slot.live && slot.subject == subject && slot.producer == producer && slot.version == version\n",
           "    slot.live && slot.subject == subject && slot.producer == producer\n",
           ("Strop.Kernel.holds_spec",)),
    # a retired slot is never served
    Mutant("cache-dead-served", "cache.rs",
           "    slot.live && slot.subject == subject && slot.producer == producer && slot.version == version\n",
           "    slot.subject == subject && slot.producer == producer && slot.version == version\n",
           ("Strop.Kernel.holds_spec",)),
    # an invalidation by subject and producer covers only slots matching both
    Mutant("cache-retire-overreach", "cache.rs",
           "    (!by_subject || slot.subject == subject) && (!by_producer || slot.producer == producer)\n",
           "    (!by_subject || slot.subject == subject) || (!by_producer || slot.producer == producer)\n",
           ("Strop.Kernel.selects_spec",)),
    # a covered slot is dead after the invalidation
    Mutant("cache-retire-keeps-live", "cache.rs",
           "            slot.live = false;\n",
           "            slot.live = true;\n",
           ("Strop.Kernel.retire_loop_spec",)),
    # a use changes the tick and nothing else: it must not revive a dead slot
    Mutant("cache-touch-revives", "cache.rs",
           "        slot.used = now;\n",
           "        slot.used = now;\n        slot.live = true;\n",
           ("Strop.Kernel.touch_spec",)),
    # with no dead slot the least recently used entry goes, not the most recent
    Mutant("cache-evicts-newest", "cache.rs",
           "        if best == slots.len() || slot.used < oldest {",
           "        if best == slots.len() || slot.used > oldest {",
           ("Strop.Kernel.victim_loop_spec",)),
    # a value over the limit is refused: the cost bound is the memory bound
    Mutant("cache-cost-unbounded", "cache.rs",
           "    if cost > limit {\n        return None;\n    }\n",
           "",
           ("Strop.Kernel.place_spec",)),
    # a refresh retires the pair's previous entry before storing the new one
    Mutant("cache-place-keeps-previous", "cache.rs",
           "    retire(slots, subject, producer, true, true);\n",
           "",
           ("Strop.Kernel.place_spec",)),
    # right affinity: an anchor exactly at the old end moves with the text after it
    Mutant("map-left-affinity", "editmap.rs",
           "    } else if position >= old_end {",
           "    } else if position > old_end {",
           ("Strop.Kernel.map_position_spec",)),
    # an anchor inside the replaced range collapses to the edit start, not its old end
    Mutant("map-collapse-to-end", "editmap.rs",
           "    } else {\n        start\n    }\n}",
           "    } else {\n        old_end\n    }\n}",
           ("Strop.Kernel.map_position_spec",)),
    # an edit ending past the buffer is refused
    Mutant("batch-end-unbounded", "editmap.rs",
           "        if current.0 > current.1 || current.1 > len {",
           "        if current.0 > current.1 {",
           # `len` leaves the loop's signature, so the top-level proof breaks too
           ("Strop.Kernel.validate_batch_loop_spec", "Strop.Kernel.validate_batch_spec")),
    # two edits at one offset (two inserts included) are an overlap
    Mutant("batch-shared-start", "editmap.rs",
           "            if previous.0 == current.0 || previous.1 > current.0 {",
           "            if previous.1 > current.0 {",
           ("Strop.Kernel.validate_batch_loop_spec",)),
    # an edit reaching past the next one's start is an overlap
    Mutant("batch-reach-ignored", "editmap.rs",
           "            if previous.0 == current.0 || previous.1 > current.0 {",
           "            if previous.0 == current.0 || previous.1 > current.1 {",
           ("Strop.Kernel.validate_batch_loop_spec",)),
    # a journal maps in application order: the batch's last edit first
    Mutant("journal-reversed", "editmap.rs",
           "        let edit = edits[k];\n        mapped = map_position",
           "        let edit = edits[edits.len() - 1 - k];\n        mapped = map_position",
           ("Strop.Kernel.map_through_edits_loop_spec",)),
    # a view edit running past the excerpt body's end reaches the card border
    Mutant("excerpt-end-unbounded", "excerpt.rs",
           "    if start < view_start || start > end || end > view_end {",
           "    if start < view_start || start > end {",
           ("Strop.Kernel.admits_spec",)),
    # an edit that joins the last body line with the row below it is refused
    Mutant("excerpt-terminator-ignored", "excerpt.rs",
           "    end < view_end || keeps_terminator\n",
           "    end <= view_end || keeps_terminator\n",
           ("Strop.Kernel.admits_spec",)),
    # the source edit starts at the view edit's own offset, not its end's
    Mutant("excerpt-start-from-end", "excerpt.rs",
           "    let from = clamp(start - view_start, shown);",
           "    let from = clamp(end - view_start, shown);",
           ("Strop.Kernel.source_edit_spec",)),
    # the view's added terminator maps onto the source's end, never past it
    Mutant("excerpt-unclamped", "excerpt.rs",
           "    let to = clamp(end - view_start, shown);",
           "    let to = end - view_start;",
           ("Strop.Kernel.source_edit_spec",)),
    # a generation retires at u32::MAX, not one early
    Mutant("generation-early-retire", "generation.rs",
           "    if current == u32::MAX {",
           "    if current == u32::MAX - 1 {",
           ("Strop.Kernel.next_generation_spec",)),
    # the last representable index is still a valid slot
    Mutant("index-space-short", "generation.rs",
           "    if len > u32::MAX as usize {",
           "    if len >= u32::MAX as usize {",
           ("Strop.Kernel.index_for_len_spec",)),
    # T2: the next child starts after the divider, not on it
    Mutant("layout-gap-dropped", "layout.rs",
           "        at = least(at + len + gap, cells);\n",
           "        at = least(at + len, cells);\n",
           ("Strop.Kernel.tile_loop_body_spec",)),
    # T3: every child first gets its minimum
    Mutant("layout-minimum-forgotten", "layout.rs",
           "            least(minimum(tracks[k], unit, gap) + extra, left)\n",
           "            least(extra, left)\n",
           # `unit` leaves the loop's signature, so the proofs that call it break too
           ("Strop.Kernel.tile_loop_body_spec", "Strop.Kernel.tile_loop_spec", "Strop.Kernel.tile_spec")),
    # T1: a span never passes the frame, even when the minimum does not fit
    Mutant("layout-overrun", "layout.rs",
           "            least(minimum(tracks[k], unit, gap) + extra, left)\n",
           "            minimum(tracks[k], unit, gap) + extra\n",
           ("Strop.Kernel.tile_loop_body_spec",)),
    # T2: the last child takes what is left, so the frame's last cells are not lost
    Mutant("layout-last-not-remainder", "layout.rs",
           "{\n            left\n        } else {",
           "{\n            least(minimum(tracks[k], unit, gap), left)\n        } else {",
           ("Strop.Kernel.tile_loop_body_spec",)),
    # the minimum counts the gaps between neighbours
    Mutant("layout-minimum-no-gaps", "layout.rs",
           "        tracks * unit + (tracks - 1) * gap\n",
           "        tracks * unit\n",
           ("Strop.Kernel.minimum_spec",)),
    # T4: a child's share is by window count: a group of two gets two windows' share
    Mutant("layout-share-unweighted", "layout.rs",
           "    (tracks * room).checked_div(weight).unwrap_or(0)\n",
           "    room.checked_div(weight).unwrap_or(0)\n",
           ("Strop.Kernel.share_spec",)),
    # the windows across all children include the first child's
    Mutant("layout-total-skips-first", "layout.rs",
           "    let mut sum = 0;\n    let mut k = 0;\n",
           "    let mut sum = 0;\n    let mut k = 1;\n",
           ("Strop.Kernel.total_spec",)),
)

# lake reports `error: <path>.lean:<line>:<col>: <message>`
ERROR = re.compile(r"^error:\s*(?P<file>[^\s:]+\.lean):(?P<line>\d+):(?P<col>\d+):", re.M)
DECLARATION = re.compile(r"^(?:@\[[^\]]*\]\s*)?(?:private\s+|protected\s+)?(?:theorem|lemma|def|abbrev|instance)\s+(?P<name>[^\s:(]+)", re.M)
NAMESPACE = re.compile(r"^namespace\s+(?P<name>\S+)", re.M)


def declarations(path: Path) -> list[tuple[int, str]]:
    """(line, fully qualified name) of every declaration in a theorem file."""
    namespace = ""
    found = []
    for number, line in enumerate(path.read_text().splitlines(), start=1):
        if match := NAMESPACE.match(line):
            namespace = match.group("name") + "."
            continue
        if line.startswith("end "):
            namespace = ""
            continue
        if match := DECLARATION.match(line):
            found.append((number, namespace + match.group("name")))
    return found


def attribute(file: Path, line: int) -> str | None:
    enclosing = None
    for start, name in declarations(file):
        if start <= line:
            enclosing = name
    return enclosing


def failing_theorems(output: str, project: Path) -> tuple[set[str], list[str]]:
    failures: set[str] = set()
    unattributed: list[str] = []
    for match in ERROR.finditer(output):
        file = (project / match.group("file")).resolve()
        try:
            relative = file.relative_to(project)
        except ValueError:
            unattributed.append(match.group(0))
            continue
        if relative.parts[0] != "Kernel" or not file.exists():
            unattributed.append(str(relative))
            continue
        name = attribute(file, int(match.group("line")))
        if name is None:
            unattributed.append(str(relative))
        else:
            failures.add(name)
    return failures, unattributed


def run(command: list[str], cwd: Path) -> subprocess.CompletedProcess:
    return subprocess.run(command, cwd=cwd, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                          text=True, timeout=1800)


class Calibration(Exception):
    """One mutant's verdict: the message is the gate line, `log` the tool output."""

    def __init__(self, message: str, log: str = ""):
        super().__init__(message)
        self.log = log


def calibrate(mutant: Mutant, scratch: Path) -> str:
    """Return the calibration line, or raise `Calibration` with the evidence."""
    crate = scratch / "crate"
    shutil.copytree(KERNEL, crate, ignore=shutil.ignore_patterns("target"))
    source = crate / "src" / mutant.file
    text = source.read_text()
    if text.count(mutant.before) != 1:
        raise Calibration(f"FAIL {mutant.name}: mutation site does not match exactly once")
    source.write_text(text.replace(mutant.before, mutant.after))

    extraction = run(["sh", str(PROOFS / "extract.sh"), str(crate), str(scratch / "Extracted")], ROOT)
    if extraction.returncode != 0:
        raise Calibration(f"FAIL {mutant.name}: the mutated kernel did not translate", extraction.stdout)

    project = scratch / "proofs"
    project.mkdir()
    for name in ("lakefile.toml", "lean-toolchain", "lake-manifest.json", "Kernel.lean", "Audit.lean"):
        shutil.copy2(PROOFS / name, project / name)
    shutil.copytree(PROOFS / "Kernel", project / "Kernel")
    shutil.copytree(scratch / "Extracted", project / "Extracted")
    (project / ".lake").mkdir()
    os.symlink(PROOFS / ".lake" / "packages", project / ".lake" / "packages")

    build = run(["lake", "build"], project)
    if build.returncode == 0:
        raise Calibration(f"FAIL {mutant.name}: the proofs still build against the mutated kernel")
    failures, unattributed = failing_theorems(build.stdout, project)
    if unattributed:
        raise Calibration(f"FAIL {mutant.name}: errors outside the theorem files: {unattributed}", build.stdout)
    expected = set(mutant.fails)
    if not failures or not expected <= failures:
        raise Calibration(f"FAIL {mutant.name}: expected {sorted(expected)} to fail, got {sorted(failures)}",
                          build.stdout + f"\nattributed failures: {sorted(failures)}")
    if failures - expected:
        raise Calibration(f"FAIL {mutant.name}: unexpected failing theorems {sorted(failures - expected)}",
                          build.stdout)
    return f"calibration: {mutant.name} rejected by {', '.join(sorted(failures))}"


def verdict(mutant: Mutant) -> tuple[bool, str]:
    """(calibrated, text to print) for one mutant, in its own scratch directory."""
    with tempfile.TemporaryDirectory(prefix=f"mutant-{mutant.name}-") as directory:
        try:
            return True, calibrate(mutant, Path(directory))
        except Calibration as error:
            return False, (error.log + "\n" if error.log else "") + str(error)


def jobs(count: int) -> int:
    """Concurrent calibrations: each is one Charon/Aeneas run plus a scratch
    `lake build`; PROOF_MUTANT_JOBS overrides the default of 4."""
    configured = os.environ.get("PROOF_MUTANT_JOBS")
    return max(1, min(count, int(configured) if configured else 4))


def main() -> None:
    selected = set(sys.argv[1:])
    mutants = [m for m in MUTANTS if not selected or m.name in selected]
    if selected and len(mutants) != len(selected):
        raise SystemExit(f"unknown mutant(s): {sorted(selected - {m.name for m in mutants})}")
    # Every mutant runs even after a failure, so one gate run reports them all;
    # verdicts are printed in declaration order whatever order they finish in.
    failed: list[str] = []
    with ThreadPoolExecutor(max_workers=jobs(len(mutants))) as pool:
        for mutant, (calibrated, text) in zip(mutants, pool.map(verdict, mutants)):
            print(text, flush=True)
            if not calibrated:
                failed.append(mutant.name)
    if failed:
        raise SystemExit(f"FAIL: {len(failed)} mutant(s) not calibrated: {failed}")
    print(f"PROOF_CALIBRATIONS={len(mutants)}", flush=True)


if __name__ == "__main__":
    main()
