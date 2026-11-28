import Lean
import Kernel

/-!
# Axiom and claim audit (plans/0086 §2.7, plans/0108 D4)

Every theorem under `Strop.Kernel` is audited: its transitive axioms must be a
subset of Lean's standard three. Anything else — `sorryAx`, a project axiom,
an Aeneas `axiom` standing in for an unmodelled external — fails the build.
The headline claims must also exist, so a deleted or renamed theorem cannot
pass silently, and every public kernel function the translation contains must
appear in the statement of some claim, so a new `pub fn` cannot ship unproved.
The `AXIOMS` lines this prints are the generated claim list; the `CLAIM_DEPS`
lines name every kernel theorem each claim rests on, which the mutant
calibration (`proofs/mutants.py`) reads to decide which claims a mutant breaks.
Every public function is also classified (#282): `TOTAL <theorem> <function>`
when a theorem states it never fails on any input, `CONDITIONAL <function>`
with the hypothesis and the production site that establishes it otherwise.
-/
open Lean Elab Command Meta

/-- Lean's standard axioms. -/
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The headline claims (plans/0086 §4, ring 0; plans/0094 §3.1; plans/0099 §3.1;
plans/0104 §9; plans/0108 D4; plans/0111 D9; line-geometry kernel RFC). -/
def claims : List Name := [
  ``Strop.Kernel.admission_resource_exact,
  ``Strop.Kernel.admission_stamp_exact,
  ``Strop.Kernel.admission_origin_exact,
  ``Strop.Kernel.admission_authorized_producer,
  ``Strop.Kernel.admit_live_answer,
  ``Strop.Kernel.refresh_settles_only_captured,
  ``Strop.Kernel.deepest_boundary,
  ``Strop.Kernel.candidate_hints_only_prioritize,
  ``Strop.Kernel.settle_certifies_verified_walk,
  ``Strop.Kernel.workspace_all_and_loose,
  ``Strop.Kernel.vacant_spec,
  ``Strop.Kernel.holds_spec,
  ``Strop.Kernel.find_spec,
  ``Strop.Kernel.retire_spec,
  ``Strop.Kernel.find_after_retire,
  ``Strop.Kernel.touch_spec,
  ``Strop.Kernel.selects_spec,
  ``Strop.Kernel.victim_spec,
  ``Strop.Kernel.place_spec,
  ``Strop.Kernel.total_cost_le,
  ``Strop.Kernel.placed_bounded,
  ``Strop.Kernel.placed_entries,
  ``Strop.Kernel.find_after_place,
  ``Strop.Kernel.place_evicts_oldest,
  ``Strop.Kernel.map_position_spec,
  ``Strop.Kernel.mapSpec_monotone,
  ``Strop.Kernel.mapSpec_in_bounds,
  ``Strop.Kernel.validate_batch_spec,
  ``Strop.Kernel.map_through_edits_spec,
  ``Strop.Kernel.map_through_edits_monotone,
  ``Strop.Kernel.map_through_edits_composition,
  ``Strop.Kernel.consecutive_of_batchOk,
  ``Strop.Kernel.map_span_spec,
  ``Strop.Kernel.map_span_keeps_order,
  ``Strop.Kernel.keep_disjoint_spec,
  ``Strop.Kernel.admits_spec,
  ``Strop.Kernel.source_edit_spec,
  ``Strop.Kernel.toSource_in_span,
  ``Strop.Kernel.source_edit_commutes,
  ``Strop.Kernel.next_generation_spec,
  ``Strop.Kernel.index_for_len_spec,
  ``Strop.Kernel.project_spec,
  ``Strop.Kernel.cover_spec,
  ``Strop.Kernel.uncover_spec,
  ``Strop.Kernel.replace_window_spec,
  ``Strop.Kernel.clip_spec,
  ``Strop.Kernel.minimum_spec,
  ``Strop.Kernel.total_spec,
  ``Strop.Kernel.share_spec,
  ``Strop.Kernel.tile_spec,
  ``Strop.Kernel.tile_in_bounds,
  ``Strop.Kernel.tile_exact,
  ``Strop.Kernel.tile_minimum,
  ``Strop.Kernel.tile_fair,
  ``Strop.Kernel.tile_fair_leaves,
  ``Strop.Kernel.runs_spec,
  ``Strop.Kernel.runs_coherent,
  ``Strop.Kernel.coherent_spec,
]

/-- The public kernel functions no theorem proves total (plans/0108 #282):
each is covered only by claims under a hypothesis, so outside it the Aeneas
model fails where the release build (no overflow checks) wraps. Every entry
names the function, the hypothesis its claims carry, and the production site
that establishes it. A public function with no total theorem fails the audit
until it is listed here; an entry for a function that is total fails too. -/
def conditional : List (Name × String × String) := [
  (``strop_kernel.editmap.keep_disjoint,
    "keep_disjoint_spec: keep has one slot per range and every range is in bounds",
    "strop-grammar resolve/mod.rs: `keep` is allocated with `ranges.len()` slots from targets the resolver clamped to the buffer"),
  (``strop_kernel.editmap.map_span,
    "map_span_spec: a ≤ b, s ≤ e, s ≤ ne and ne + (b - e) ≤ usize::MAX",
    "strop-engine transact.rs, collections/updates.rs, completion/words/journal.rs, structural_indent.rs: spans and edits are byte ranges of one live buffer, ordered by construction"),
  (``strop_kernel.intervals.clip,
    "clip_spec: empty out, sorted spans, spans.len() + 1 < usize::MAX",
    "strop-engine analysis/presentation.rs `clip`: reach spans come sorted from the accepted presentation"),
  (``strop_kernel.intervals.cover,
    "cover_spec: empty out, sorted coverage, 2 * coverage.len() + 2 < usize::MAX",
    "strop-engine analysis/presentation.rs `cover`: coverage is only ever written by cover/uncover, which keep it sorted"),
  (``strop_kernel.intervals.project,
    "project_spec: empty out, s ≤ oe, s ≤ ne, sorted ranges, room for 2 * len + 2 pushes, ne + (end - oe) ≤ usize::MAX",
    "strop-engine analysis/presentation.rs `project`: token ranges of one accepted frame and an edit of the same buffer"),
  (``strop_kernel.intervals.replace_window,
    "replace_window_spec: empty out, first ≤ last, sorted spans and fresh, fresh inside the window, room for the pushes",
    "strop-engine analysis/presentation.rs `replace_window`: a frame's spans are sorted and clipped to its own window"),
  (``strop_kernel.intervals.uncover,
    "uncover_spec: empty out, first ≤ last, sorted coverage, 2 * coverage.len() + 2 < usize::MAX",
    "strop-engine analysis/presentation.rs `uncover`: windows are first ≤ last by construction; coverage as for cover"),
  (``strop_kernel.lines.runs,
    "runs_spec: old_start + body.len() and new_start + body.len() ≤ usize::MAX, out.len() + body.len() < usize::MAX (Aeneas models Vec::push as failing at usize::MAX)",
    "strop-git diff.rs `Hunk::regions`: starts and body are one libgit2 hunk of a file held in memory, so every line number and the region count are below the file's line count"),
  (``strop_kernel.layout.minimum,
    "minimum_spec: tracks * unit + (tracks - 1) * gap ≤ usize::MAX",
    "strop-engine panes/layout.rs `minimum` and `place_frame`: track counts are window counts and (unit, gap) is (1, 1) or (2, 0)"),
  (``strop_kernel.layout.share,
    "share_spec: tracks * room ≤ usize::MAX",
    "layout::tile, its only caller, under TileBounded (room ≤ cells ≤ 65535, tracks ≤ sum ≤ 65535)"),
  (``strop_kernel.layout.tile,
    "TileBounded: cells ≤ 65535, sum of tracks ≤ 65535, unit ≤ 2, gap ≤ 1",
    "strop-engine panes/layout.rs `place_frame`: cells is a u16 frame size and (unit, gap) is (1, 1) or (2, 0); the track sum is not checked (#282)"),
  (``strop_kernel.layout.total,
    "total_spec: the sum of tracks ≤ usize::MAX",
    "layout::tile, its only caller, under TileBounded (sum ≤ 65535)"),
]

/-- The namespace the theorems live in. -/
def audited : Name := `Strop.Kernel

/-- Every theorem under `Strop.Kernel`, sorted by name. -/
def kernelTheorems (env : Environment) : Array Name :=
  (env.constants.fold (init := (#[] : Array Name)) fun acc decl info =>
    if audited.isPrefixOf decl && decl.isInternal = false && info.isThm then acc.push decl
    else acc).qsort (fun a b => a.toString < b.toString)

/-- Every `Strop.Kernel` theorem the proof of `decl` rests on, itself included
(auxiliary `_proof` lemmas are walked through but are part of their parent). -/
partial def kernelDeps (env : Environment) (decl : Name) : NameSet :=
  go {} [decl]
where
  go (seen : NameSet) : List Name → NameSet
    | [] => seen
    | n :: rest =>
      if seen.contains n then go seen rest else
      match env.find? n with
      | some info =>
        let used := info.getUsedConstantsAsSet.toList.filter fun m =>
          audited.isPrefixOf m && (env.find? m).any (·.isThm)
        go (seen.insert n) (used ++ rest)
      | none => go (seen.insert n) rest

/-- The public functions of the translated crate: Aeneas documents each
extracted definition with its Rust path and visibility; loops are the
translation's own helpers, not functions of the crate. -/
def publicFunctions : CommandElabM (Array Name) := do
  let env ← getEnv
  let mut found := #[]
  for (decl, info) in env.constants.toList do
    -- trait implementations (`Insts`) are derived, not functions of the crate
    unless (`strop_kernel).isPrefixOf decl && info matches .defnInfo _ do continue
    if decl.components.contains `Insts then continue
    let some doc ← liftIO (findDocString? env decl) | continue
    let has (part : String) := (doc.splitOn part).length > 1
    if has "[strop_kernel::" && has "Visibility: public" && !has "Trait implementation"
        && !has "loop" then
      found := found.push decl
  return found.qsort (fun a b => a.toString < b.toString)

/-- The function a theorem states never fails on any input, if it has the
shape `∀ (x₁ : α₁) … (xₙ : αₙ), f x₁ … xₙ ⦃ _ ⦄` where no `αᵢ` is a
proposition and `f` is applied to exactly those variables (the check of
tauten's proofs template, MIT). -/
def totalOver (type : Expr) : MetaM (Option Name) :=
  forallTelescope type fun xs body => do
    for x in xs do
      if ← isProp (← inferType x) then return none
    unless body.isAppOfArity ``Aeneas.Std.WP.spec 3 do return none
    let program := body.getAppArgs[1]!
    let fn := program.getAppFn
    unless fn.isConst do return none
    let args := program.getAppArgs
    unless args.size == xs.size do return none
    for (arg, x) in args.zip xs do
      unless arg == x do return none
    return some fn.constName!

/-- Classify every public function as total (a theorem of `totalOver`'s
shape names it) or conditional (listed in `conditional`), and refuse a
function that is neither, or listed although total. -/
def auditTotality (sorted : Array Name) (functions : Array Name) : CommandElabM Unit := do
  let env ← getEnv
  let mut total : NameMap Name := {}
  for decl in sorted do
    let some info := env.find? decl | continue
    if let some fn ← liftTermElabM (totalOver info.type) then
      unless total.contains fn do total := total.insert fn decl
  let mut unlisted := #[]
  for fn in functions do
    match total.find? fn, conditional.find? (·.1 == fn) with
    | some thm, none => logInfo m!"TOTAL {thm} {fn}"
    | some thm, some _ =>
      throwError "totality audit: {fn} is total ({thm}) but listed in `conditional`"
    | none, some (_, hyp, site) => logInfo m!"CONDITIONAL {fn}: {hyp}; established at {site}"
    | none, none => unlisted := unlisted.push fn
  for (fn, _) in conditional do
    unless functions.contains fn do
      throwError "totality audit: `conditional` lists {fn}, which is not a public kernel function"
  unless unlisted.isEmpty do
    throwError "totality audit: public kernel functions with no total theorem and not listed in `conditional`: {unlisted.toList}"

-- `name` is a reserved token once the Aeneas library is imported (its attribute
-- syntax), so declarations are called `decl` here.
def auditAxioms : CommandElabM Unit := do
  let env ← getEnv
  let sorted := kernelTheorems env
  if sorted.isEmpty then
    throwError "axiom audit: no theorems under Strop.Kernel"
  for decl in sorted do
    let axioms := (← collectAxioms decl).qsort (fun a b => a.toString < b.toString)
    logInfo m!"AXIOMS {decl}: {axioms.toList}"
    for ax in axioms do
      unless allowedAxioms.contains ax do
        throwError "axiom audit: {decl} depends on {ax}, which is not a standard Lean axiom"
  let mut stated : NameSet := {}
  for claim in claims do
    unless sorted.contains claim do
      throwError "axiom audit: required claim {claim} is missing or is not a theorem"
    let some info := env.find? claim | throwError "axiom audit: {claim} vanished"
    for used in info.type.getUsedConstants do
      stated := stated.insert used
    let deps := (kernelDeps env claim).toList.filter (!·.isInternal) |>.map toString
      |>.toArray.qsort (· < ·)
    logInfo m!"CLAIM_DEPS {claim}: {String.intercalate " " deps.toList}"
  let functions ← publicFunctions
  if functions.isEmpty then
    throwError "claim audit: the translation documents no public kernel function"
  let unclaimed := functions.filter (!stated.contains ·)
  unless unclaimed.isEmpty do
    throwError "claim audit: public kernel functions in no claim's statement: {unclaimed.toList}"
  auditTotality sorted functions
  logInfo m!"AXIOM_AUDIT ok: {sorted.size} theorems, {claims.length} required claims, {functions.size} public functions claimed, axioms ⊆ {allowedAxioms}"

run_cmd auditAxioms
