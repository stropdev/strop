#!/usr/bin/env python3
"""Mutant calibration for the `proofs` gate (plans/0086 §8, plans/0108 D4).

Every moved kernel function gets at least one semantic mutation of its Rust
source. For each: copy the crate, apply the mutation, translate it exactly as
the gate does (proofs/extract.sh), build the theorems against the mutated
translation in a scratch copy of the lake project, and require the build to
FAIL inside the named theorem(s) — no other theorem may fail, and the
translation itself must still type-check. A proof that survives its mutant
proves nothing; a failure anywhere else is noise, not calibration.

A Rust mutant breaks the bridge lemma that unfolds the code (a `_spec`);
property theorems stated over a model of the code sit behind that bridge and
never see it. A *paired* mutant also patches the model (`lean`) the way a
developer changing the code would, so the bridge still proves and the
property itself must fail.

Every claim in `proofs/Audit.lean` must be broken by some mutant. The audit
prints, per claim, every kernel theorem its proof rests on (`CLAIM_DEPS`). A
mutant breaks a claim when one of the theorems it makes fail is among them —
for a property claim (a name not ending in `_spec`), a failing bridge does not
count unless it is the claim itself: the failure must reach the property.
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
class Patch:
    """A textual replacement in a theorem file under proofs/, made `count` times."""
    file: str
    before: str
    after: str
    count: int = 1


@dataclass(frozen=True)
class Mutant:
    name: str
    file: str  # the kernel source under crates/strop-kernel/src; "" patches the model only
    before: str
    after: str
    fails: tuple[str, ...]
    lean: tuple[Patch, ...] = ()


MUTANTS = (
    Mutant("index-members-only-walk", "coverage.rs",
           "Hint::None => Schedule::Examine,\n            Hint::LikelyHit | Hint::Declared { .. } => Schedule::ExamineFirst,",
           "Hint::None | Hint::LikelyHit => Schedule::Skip,\n            Hint::Declared { .. } => Schedule::ExamineFirst,",
           ("Strop.Kernel.candidate_hints_only_prioritize",)),
    Mutant("index-exclude-unverified-candidate", "coverage.rs",
           "Evidence::Unknown | Evidence::Eligible => match hint {",
           "Evidence::Unknown => Schedule::Skip,\n        Evidence::Eligible => match hint {",
           ("Strop.Kernel.candidate_hints_only_prioritize",)),
    Mutant("index-complete-before-walk-exhaustion", "coverage.rs",
           " && exhausted && all_verified(paths)", " && all_verified(paths)",
           ("Strop.Kernel.settle_certifies_verified_walk",)),
    Mutant("index-queued-work-counts-as-verified", "coverage.rs",
           "if !matches!(paths[k], PathWork::Verified) {",
           "if !matches!(paths[k], PathWork::Verified | PathWork::Queued) {",
           ("Strop.Kernel.all_paths_verified",)),
    Mutant("index-cap-or-error-completes-query", "coverage.rs",
           "matches!(terminal, Terminal::Success)",
           "matches!(terminal, Terminal::Success | Terminal::Truncated | Terminal::Failed)",
           ("Strop.Kernel.settle_certifies_verified_walk",)),
    Mutant("index-unavailable-or-pending-certifies-empty", "coverage.rs",
           "matches!(terminal, Terminal::Success)",
           "matches!(terminal, Terminal::Success | Terminal::Unsupported | Terminal::Running)",
           ("Strop.Kernel.settle_certifies_verified_walk",)),
    Mutant("index-other-query-certifies-coverage", "coverage.rs",
           "        && query.generation == scan.query.generation\n", "",
           ("Strop.Kernel.certificate_current",)),
    Mutant("index-complete-after-any-sibling", "coverage.rs",
           "if !current(query, Some(child.project), &child.scan) {\n            return false;\n        }\n        k += 1;\n    }\n    true",
           "if current(query, Some(child.project), &child.scan) {\n            return true;\n        }\n        k += 1;\n    }\n    children.len() == 0",
           ("Strop.Kernel.every_required_child",)),
    Mutant("index-omit-loose-paths", "coverage.rs",
           "    if !current(query, None, loose) {\n        return false;\n    }\n", "",
           ("Strop.Kernel.workspace_all_and_loose",)),
    Mutant("boundary-keep-shallower-ancestor", "boundary.rs",
           "Some(depth) if root.len() <= depth", "Some(depth) if root.len() >= depth",
           ("Strop.Kernel.consider_preserves_maximality",)),
    Mutant("boundary-accept-component-mismatch", "boundary.rs",
           "        if root[i] != path[i] {\n            return false;",
           "        if root[i] != path[i] {\n            return true;",
           ("Strop.Kernel.enclosing_components",)),
    Mutant("index-omit-root-incarnation", "admission.rs",
           "&& a.root_incarnation == b.root_incarnation", "",
           ("Strop.Kernel.admission_resource_exact",)),
    Mutant("index-revive-recreated-path", "admission.rs",
           "&& a.path_incarnation == b.path_incarnation", "",
           ("Strop.Kernel.admission_resource_exact",)),
    Mutant("index-ignore-content-revision", "admission.rs",
           "&& a.content == b.content", "",
           ("Strop.Kernel.admission_stamp_exact",)),
    Mutant("index-drop-child-owner", "admission.rs",
           "&& a.child == b.child", "",
           ("Strop.Kernel.admission_stamp_exact",)),
    Mutant("index-admit-old-manifest", "admission.rs",
           "&& a.declaration == b.declaration", "",
           ("Strop.Kernel.admission_stamp_exact",)),
    Mutant("index-admit-retired-query", "admission.rs",
           "&& a.query == b.query", "",
           ("Strop.Kernel.admission_stamp_exact",)),
    Mutant("index-display-cancelled-request", "admission.rs",
           "None => false,", "None => true,",
           ("Strop.Kernel.admit_live_answer",)),
    Mutant("index-use-disk-after-empty-buffer", "admission.rs",
           "Origin::Disk => matches!(surface, Surface::Disk)",
           "Origin::Disk => matches!(surface, Surface::Disk | Surface::BufferEmpty)",
           ("Strop.Kernel.admission_origin_exact",)),
    Mutant("index-use-disk-while-buffer-pending", "admission.rs",
           "Origin::Disk => matches!(surface, Surface::Disk)",
           "Origin::Disk => matches!(surface, Surface::Disk | Surface::BufferPending)",
           ("Strop.Kernel.admission_origin_exact",)),
    Mutant("index-use-syntax-for-server", "admission.rs",
           "_ => false,\n    }\n}",
           "(Designation::Semantic(_), Producer::Syntax(_)) => true,\n        _ => false,\n    }\n}",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-admit-old-server", "admission.rs",
           "&& live.server == *server", "",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-admit-moved-attachment", "admission.rs",
           "&& live.attachment == *attachment", "",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-hinted-language-authorized", "admission.rs",
           "Binding::LanguageHint(_) => false", "Binding::LanguageHint(lang) => *lang == target.language",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-bypass-attachment-trust", "admission.rs",
           "live.trusted", "true",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-bypass-symbol-capability", "admission.rs",
           "&& live.capable", "",
           ("Strop.Kernel.admission_authorized_producer",)),
    Mutant("index-clear-newer-invalidation", "admission.rs",
           "&& now.invalidation == started.invalidation", "",
           ("Strop.Kernel.refresh_settles_only_captured",)),
    Mutant("index-settle-after-watch-loss", "admission.rs",
           "&& now.watch == started.watch", "",
           ("Strop.Kernel.refresh_settles_only_captured",)),
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
    # a context line inside a run is absorbed instead of ending it
    Mutant("lines-runs-merge-across-context", "lines.rs",
           "                close(run, out);\n                run = Region {\n"
           "                    old: run.old.saturating_add(run.old_len).saturating_add(1),\n"
           "                    old_len: 0,\n"
           "                    new: run.new.saturating_add(run.new_len).saturating_add(1),\n"
           "                    new_len: 0,\n                };\n",
           "                run.old_len = run.old_len.saturating_add(1);\n"
           "                run.new_len = run.new_len.saturating_add(1);\n",
           ("Strop.Kernel.runs_loop_spec", "Strop.Kernel.runs_spec")),
    # a region's new start is the counter at the end of its run: a pure
    # deletion lands after the hunk's trailing context (the v0.77 `changed_region`)
    Mutant("lines-runs-deletion-at-hunk-end", "lines.rs",
           "        out.push(run);",
           "        out.push(Region {\n            new: run.new.saturating_add(run.new_len),\n            ..run\n        });",
           ("Strop.Kernel.close_spec",)),
    # a region shifted on one side only is accepted as a diff
    Mutant("lines-coherent-accepts-skew", "lines.rs",
           "        if r.old - old_base != r.new - new_base {\n            return false;\n        }\n",
           "",
           ("Strop.Kernel.coherent_loop_spec",)),
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


# --- paired and model mutants: the bridge still proves, the property must fail ---
PAIRED = (
    # a new table serves nothing: every slot starts dead
    Mutant("cache-vacant-live", "cache.rs",
           "    Slot {\n        live: false,\n        subject: 0,",
           "    Slot {\n        live: true,\n        subject: 0,",
           ("Strop.Kernel.vacant_spec",)),
    # an invalidation that revives what it covers, with the spec updated to match:
    # a retired pair is still served, a refreshed pair keeps its old entry, a
    # revived slot's stale cost escapes the bound, eviction sees no dead slot
    Mutant("cache-retire-revives", "cache.rs",
           "            slot.live = false;\n",
           "            slot.live = true;\n",
           ("Strop.Kernel.retired_not_holds", "Strop.Kernel.placed_bounded",
            "Strop.Kernel.placed_entries", "Strop.Kernel.place_evicts_oldest"),
           lean=(Patch("Kernel/Cache.lean",
                       "(Selects old s p bs bp → new = { old with live := false })",
                       "(Selects old s p bs bp → new = { old with live := true })"),
                 Patch("Kernel/Cache.lean",
                       "if k.val = j then { x with live := false } else",
                       "if k.val = j then { x with live := true } else"))),
    # most-recently-used eviction, with the spec updated to match
    Mutant("cache-evicts-newest-paired", "cache.rs",
           "        if best == slots.len() || slot.used < oldest {",
           "        if best == slots.len() || slot.used > oldest {",
           # the loop proof follows the old comparison, so the bridge breaks too
           ("Strop.Kernel.victim_loop_spec", "Strop.Kernel.place_evicts_oldest"),
           lean=(Patch("Kernel/Cache.lean",
                       "      ∀ j (hj : j < l.length), l[r].used.val ≤ l[j].used.val)",
                       "      ∀ j (hj : j < l.length), l[j].used.val ≤ l[r].used.val)"),)),
    # the memory bound counts only live entries: a dead slot's stale cost is free
    Mutant("cache-cost-counts-dead", "", "", "",
           ("Strop.Kernel.total_cost_le",),
           lean=(Patch("Kernel/Cache.lean",
                       "  | x :: rest => (if x.live = true then x.cost.val else 0) + liveCost rest",
                       "  | x :: rest => x.cost.val + liveCost rest"),)),
    # interior anchors collapse to the old end, with the model updated to match:
    # anchors cross, leave the buffer, and the journal fold stops composing
    Mutant("map-collapse-to-end-paired", "editmap.rs",
           "    } else {\n        start\n    }\n}",
           "    } else {\n        old_end\n    }\n}",
           ("Strop.Kernel.mapSpec_monotone", "Strop.Kernel.mapSpec_in_bounds",
            "Strop.Kernel.mapSpec_eq_mapNat"),
           lean=(Patch("Kernel/EditMap.lean",
                       "  if p < s then p else if oe ≤ p then min (ne + (p - oe)) Usize.max else s",
                       "  if p < s then p else if oe ≤ p then min (ne + (p - oe)) Usize.max else oe"),)),
    # an overlap is an edit reaching past the next one's end, not its start,
    # with the batch predicate updated to match: accepted batches stop composing
    Mutant("batch-reach-to-end-paired", "editmap.rs",
           "            if previous.0 == current.0 || previous.1 > current.0 {",
           "            if previous.0 == current.0 || previous.1 > current.1 {",
           ("Strop.Kernel.consecutive_of_batchOk",),
           lean=(Patch("Kernel/EditMap.lean",
                       "  a.1.val < b.1.val ∧ a.2.val ≤ b.1.val",
                       "  a.1.val < b.1.val ∧ a.2.val ≤ b.2.val"),)),
    # an edit starting at a span's end belongs to it unasked (bug 0108 B4)
    Mutant("span-end-claimed", "editmap.rs",
           "    let mapped_end = if end < s || (end == s && !grow_at_end) {",
           "    let mapped_end = if end < s {",
           ("Strop.Kernel.map_span_spec", "Strop.Kernel.map_span_end_before")),
    # a span's start is left-affine: an insertion at the start lands inside it
    Mutant("span-start-right-affine", "editmap.rs",
           "    let mapped_start = if start <= s {",
           "    let mapped_start = if start < s {",
           ("Strop.Kernel.map_span_spec",)),
    # a range starting exactly where the kept one ends does not overlap it
    Mutant("keep-touching-dropped", "editmap.rs",
           "        if range.0 >= reach {",
           "        if range.0 > reach {",
           ("Strop.Kernel.keep_disjoint_loop_spec",)),
    # the filter compares against the last kept range, not the previous input range
    Mutant("keep-against-previous-input", "editmap.rs",
           "            keep[k] = true;\n            reach = range.1;\n        } else {\n"
           "            keep[k] = false;\n        }\n",
           "            keep[k] = true;\n        } else {\n"
           "            keep[k] = false;\n        }\n        reach = range.1;\n",
           ("Strop.Kernel.keep_disjoint_loop_spec",)),
    # the translation is clamped onto the source, with the model updated to match
    Mutant("excerpt-unclamped-paired", "excerpt.rs",
           "    if value > bound {\n        bound\n    } else {",
           "    if value > bound {\n        value\n    } else {",
           # the clamp's own proof steps follow the old branch, so the bridges break too
           ("Strop.Kernel.clamp_spec", "Strop.Kernel.source_edit_spec",
            "Strop.Kernel.toSource_in_span"),
           lean=(Patch("Kernel/Excerpt.lean",
                       "excerpt.clamp value bound ⦃ r => r.val = min value.val bound.val ⦄",
                       "excerpt.clamp value bound ⦃ r => r.val = value.val ⦄"),
                 Patch("Kernel/Excerpt.lean",
                       "def toSource (vs ss se p : Nat) : Nat := ss + min (p - vs) (se - ss)",
                       "def toSource (vs ss se p : Nat) : Nat := ss + (p - vs)"))),
    # the shown source is the whole excerpt, with the model updated to match:
    # anchors past the clamp stop agreeing between view and source
    Mutant("excerpt-half-shown-paired", "excerpt.rs",
           "    let shown = source_end - source_start;",
           "    let shown = (source_end - source_start) / 2;",
           ("Strop.Kernel.source_edit_spec", "Strop.Kernel.source_edit_commutes"),
           lean=(Patch("Kernel/Excerpt.lean",
                       "def toSource (vs ss se p : Nat) : Nat := ss + min (p - vs) (se - ss)",
                       "def toSource (vs ss se p : Nat) : Nat := ss + min (p - vs) ((se - ss) / 2)"),)),
    # K1: only the part strictly before the edit keeps its coordinates
    Mutant("project-empty-fragment", "intervals.rs",
           "        if range.0 < start {",
           "        if range.0 <= start {",
           ("Strop.Kernel.project_loop_spec",)),
    # K1: the replaced bytes never inherit a class
    Mutant("project-paints-insert", "intervals.rs",
           "            out.push((range.0, least(range.1, start), k));",
           "            out.push((range.0, range.1, k));",
           ("Strop.Kernel.project_loop_spec",)),
    # K1: an interval ending at the old end has nothing after the edit
    Mutant("project-after-at-end", "intervals.rs",
           "        if range.1 > old_end {",
           "        if range.1 >= old_end {",
           ("Strop.Kernel.project_loop_spec",)),
    # K2: the merged interval is part of the union
    Mutant("cover-drops-merged", "intervals.rs",
           "    out.push((low, high));\n",
           "",
           ("Strop.Kernel.cover_merge_spec", "Strop.Kernel.cover_spec")),
    # K2: an interval touching the window merges with it
    Mutant("cover-touching-kept-apart", "intervals.rs",
           "        } else if interval.0 <= last {",
           "        } else if interval.0 < last {",
           ("Strop.Kernel.cover_merge_spec",)),
    # K2: what survives a window is non-empty
    Mutant("uncover-empty-part", "intervals.rs",
           "        if interval.0 < below {\n            out.push((interval.0, below, k));",
           "        if interval.0 <= below {\n            out.push((interval.0, below, k));",
           ("Strop.Kernel.uncover_loop_spec",)),
    # K2: an old span's part below the window ends at the window
    Mutant("replace-below-overlaps", "intervals.rs",
           "        let below = least(span.1, first);\n        if span.0 < below {\n            out.push((span.0, below, k));",
           "        let below = least(span.1, first);\n        if span.0 < below {\n            out.push((span.0, span.1, k));",
           ("Strop.Kernel.replace_below_spec",)),
    # K2: an old span's part above the window starts at the window
    Mutant("replace-above-overlaps", "intervals.rs",
           "        if above < span.1 {\n            out.push((above, span.1, k));",
           "        if above < span.1 {\n            out.push((span.0, span.1, k));",
           ("Strop.Kernel.replace_above_spec",)),
    # K2: a clipped part never reaches past the window
    Mutant("clip-unclamped", "intervals.rs",
           "        let high = least(span.1, last);",
           "        let high = span.1;",
           ("Strop.Kernel.clip_loop_spec", "Strop.Kernel.clip_spec")),
    # a region's new start taken at the end of its run, with the model updated
    # to match: the regions stop lining up on both sides, so a hunk is no diff
    Mutant("lines-runs-new-start-at-run-end", "lines.rs",
           "        out.push(run);",
           "        out.push(Region {\n            new: run.new.saturating_add(run.new_len),\n            ..run\n        });",
           # the push's own proof follows the old region, so the bridge breaks too
           ("Strop.Kernel.close_spec", "Strop.Kernel.coherent_closeRun_append"),
           lean=(Patch("Kernel/Lines.lean",
                       "  if q.2.1 = 0 ∧ q.2.2.2 = 0 then [] else [q]",
                       "  if q.2.1 = 0 ∧ q.2.2.2 = 0 then [] else [(q.1, q.2.1, q.2.2.1 + q.2.2.2, q.2.2.2)]"),)),
    # T1 with the model updated to match: an unclamped span passes the frame
    Mutant("layout-overrun-paired", "layout.rs",
           "            least(minimum(tracks[k], unit, gap) + extra, left)\n",
           "            minimum(tracks[k], unit, gap) + extra\n",
           ("Strop.Kernel.tile_loop_body_spec", "Strop.Kernel.tileFrom_in_bounds",
            "Strop.Kernel.tileFrom_fits"),
           lean=(Patch("Kernel/Layout.lean",
                       "      let len := min (minSpec t unit gap + extra) (cells - a)",
                       "      let len := minSpec t unit gap + extra"),
                 Patch("Kernel/Layout.lean",
                       "min (minSpec t u g + t * r / w) (c - a)",
                       "(minSpec t u g + t * r / w)", count=2))),
    # T2-T4 with the model updated to match: spans without the divider between
    # them stop covering the frame, so the minimum and the fair share go too
    Mutant("layout-gap-dropped-paired", "layout.rs",
           "        at = least(at + len + gap, cells);\n",
           "        at = least(at + len, cells);\n",
           ("Strop.Kernel.tile_loop_body_spec", "Strop.Kernel.tileFrom_fits"),
           lean=(Patch("Kernel/Layout.lean",
                       "(a, len) :: tileFrom cells unit gap (min (a + len + gap) cells)",
                       "(a, len) :: tileFrom cells unit gap (min (a + len) cells)"),
                 Patch("Kernel/Layout.lean",
                       "(min (a + min (minSpec t u g + t * r / w) (c - a) + g) c)",
                       "(min (a + min (minSpec t u g + t * r / w) (c - a)) c)"))),
)

# lake reports `error: <path>.lean:<line>:<col>: <message>`
ERROR = re.compile(r"^error:\s*(?P<file>[^\s:]+\.lean):(?P<line>\d+):(?P<col>\d+):", re.M)
DECLARATION = re.compile(r"^(?:@\[[^\]]*\]\s*)?(?:private\s+|protected\s+)?(?:theorem|lemma|def|abbrev|instance)\s+(?P<name>[^\s:(]+)", re.M)
NAMESPACE = re.compile(r"^namespace\s+(?P<name>\S+)", re.M)
END = re.compile(r"^end(\s+[\w.]+)?\s*$")


def declarations(path: Path) -> list[tuple[int, str]]:
    """(line, fully qualified name) of every declaration in a theorem file."""
    namespace = ""
    found = []
    for number, line in enumerate(path.read_text().splitlines(), start=1):
        if match := NAMESPACE.match(line):
            namespace = match.group("name") + "."
            continue
        if END.match(line):
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
    if mutant.file:
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
    for patch in mutant.lean:
        theorems = project / patch.file
        text = theorems.read_text()
        if text.count(patch.before) != patch.count:
            raise Calibration(f"FAIL {mutant.name}: model patch site in {patch.file} does not "
                              f"match exactly {patch.count} time(s)")
        theorems.write_text(text.replace(patch.before, patch.after))
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


CLAIM_DEPS = re.compile(r"CLAIM_DEPS (?P<claim>\S+): (?P<deps>.*)$", re.M)


def claim_dependencies() -> dict[str, set[str]]:
    """Every claim of the audit, with the kernel theorems its proof rests on."""
    audit = run(["lake", "env", "lean", "Audit.lean"], PROOFS)
    if audit.returncode != 0:
        raise SystemExit(audit.stdout + "\nFAIL: the audit does not run on the unmutated proofs")
    claims = {m.group("claim"): set(m.group("deps").split()) for m in CLAIM_DEPS.finditer(audit.stdout)}
    if not claims:
        raise SystemExit("FAIL: the audit printed no CLAIM_DEPS lines")
    return claims


def is_bridge(theorem: str) -> bool:
    """A lemma that unfolds extracted code into a statement about it."""
    return theorem.rsplit(".", 1)[-1].endswith("_spec")


def breaks(mutant: Mutant, claim: str, deps: set[str]) -> bool:
    reached = set(mutant.fails) & deps
    if is_bridge(claim):
        return bool(reached)
    return claim in reached or any(not is_bridge(theorem) for theorem in reached)


def coverage(mutants: list[Mutant]) -> list[str]:
    """Print which mutants break each claim; return the claims none breaks."""
    missing = []
    for claim, deps in claim_dependencies().items():
        by = [m.name for m in mutants if breaks(m, claim, deps)]
        print(f"claim: {claim} broken by {', '.join(by) if by else 'NOTHING'}", flush=True)
        if not by:
            missing.append(claim)
    return missing


def main() -> None:
    selected = set(sys.argv[1:])
    mutants = [m for m in MUTANTS + PAIRED if not selected or m.name in selected]
    if selected and len(mutants) != len(selected):
        raise SystemExit(f"unknown mutant(s): {sorted(selected - {m.name for m in mutants})}")
    # Every claim must be breakable before any mutant runs: a gap is a
    # property nothing calibrates, whatever the mutants then report.
    missing = [] if selected else coverage(mutants)
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
    if missing:
        raise SystemExit(f"FAIL: {len(missing)} claim(s) no mutant breaks: {missing}")
    print(f"PROOF_CALIBRATIONS={len(mutants)}", flush=True)


if __name__ == "__main__":
    main()
