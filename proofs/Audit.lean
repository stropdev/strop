import Lean
import Kernel

/-!
# Axiom audit (plans/0086 §2.7)

Every theorem under `Strop.Kernel` is audited: its transitive axioms must be a
subset of Lean's standard three. Anything else — `sorryAx`, a project axiom,
an Aeneas `axiom` standing in for an unmodelled external — fails the build.
The headline claims must also exist, so a deleted or renamed theorem cannot
pass silently. The `AXIOMS` lines this prints are the generated claim list.
-/
open Lean Elab Command

/-- Lean's standard axioms. -/
def allowedAxioms : List Name := [``propext, ``Classical.choice, ``Quot.sound]

/-- The headline claims (plans/0086 §4, ring 0). -/
def claims : List Name := [
  ``Strop.Kernel.map_position_spec,
  ``Strop.Kernel.mapSpec_monotone,
  ``Strop.Kernel.mapSpec_in_bounds,
  ``Strop.Kernel.validate_batch_spec,
  ``Strop.Kernel.batch_map_composition,
  ``Strop.Kernel.consecutive_of_batchOk,
  ``Strop.Kernel.header_admitted_spec,
  ``Strop.Kernel.body_admitted_spec,
  ``Strop.Kernel.frame_ready_spec,
  ``Strop.Kernel.next_generation_spec,
  ``Strop.Kernel.index_for_len_spec,
]

/-- Every theorem under `Strop.Kernel`, sorted by name. -/
def kernelTheorems (env : Environment) : Array Name :=
  let audited : Name := `Strop.Kernel
  (env.constants.fold (init := (#[] : Array Name)) fun acc decl info =>
    if audited.isPrefixOf decl && decl.isInternal = false && info.isThm then acc.push decl
    else acc).qsort (fun a b => a.toString < b.toString)

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
  for claim in claims do
    unless sorted.contains claim do
      throwError "axiom audit: required claim {claim} is missing or is not a theorem"
  logInfo m!"AXIOM_AUDIT ok: {sorted.size} theorems, {claims.length} required claims, axioms ⊆ {allowedAxioms}"

run_cmd auditAxioms
