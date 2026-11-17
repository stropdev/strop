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
-/
open Lean Elab Command

/-- Lean's standard axioms. -/
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The headline claims (plans/0086 §4, ring 0; plans/0094 §3.1; plans/0099 §3.1;
plans/0104 §9; plans/0108 D4; plans/0111 D9). -/
def claims : List Name := [
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
  ``Strop.Kernel.minimum_spec,
  ``Strop.Kernel.total_spec,
  ``Strop.Kernel.share_spec,
  ``Strop.Kernel.tile_spec,
  ``Strop.Kernel.tile_in_bounds,
  ``Strop.Kernel.tile_exact,
  ``Strop.Kernel.tile_minimum,
  ``Strop.Kernel.tile_fair,
  ``Strop.Kernel.tile_fair_leaves,
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
  logInfo m!"AXIOM_AUDIT ok: {sorted.size} theorems, {claims.length} required claims, {functions.size} public functions claimed, axioms ⊆ {allowedAxioms}"

run_cmd auditAxioms
