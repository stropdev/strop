import Extracted.StropKernel

/-! # Project boundary ownership

Components are opaque identities for complete native names in one workspace. The
host witnesses this association. The proof knows nothing about encoding, disk,
symlink resolution, declarations or markers: it treats every observed boundary
alike, in any order, even when the records overlap or repeat.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- Pointwise equality of the first `n` complete components. -/
def ComponentsAgree (root path : Slice U64) (n : Nat) : Prop :=
  ∀ j, j < n → ∀ (hr : j < root.val.length) (hp : j < path.val.length),
    root.val[j] = path.val[j]

def ComponentPrefix (root path : Slice U64) : Prop :=
  root.val.length ≤ path.val.length ∧ ComponentsAgree root path root.val.length

theorem enclosing_components (root path : Slice U64) (hlen : root.val.length ≤ path.val.length) :
    boundary.encloses_loop root path 0#usize ⦃ b => b = true ↔ ComponentPrefix root path ⦄ := by
  unfold boundary.encloses_loop
  apply loop.spec_decr_nat (fun i : Usize => root.val.length - i.val)
    (fun i : Usize => i.val ≤ root.val.length ∧ ComponentsAgree root path i.val)
  · intro i ⟨ hi, hbefore ⟩
    unfold boundary.encloses_loop.body
    try dsimp only
    split
    next hlt =>
      have hr : i.val < root.val.length := by scalar_tac
      have hp : i.val < path.val.length := by omega
      step as ⟨ x, hx ⟩
      step as ⟨ y, hy ⟩
      split
      next hne =>
        simp only [WP.spec_ok]
        constructor
        · intro h; cases h
        · intro ⟨ _, heq ⟩
          exfalso
          have hxy : x ≠ y := by simpa using hne
          apply hxy
          rw [hx, hy]
          exact heq i.val hr hr hp
      next heq =>
        step as ⟨ i1, hi1 ⟩
        refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
        intro j hj hjr hjp
        by_cases hji : j < i.val
        · exact hbefore j hji hjr hjp
        · have : j = i.val := by scalar_tac
          subst j
          rw [← hx, ← hy]
          exact UScalar.eq_of_val_eq (by simpa using heq)
    next hdone =>
      have hlen' : i.val = root.val.length := by scalar_tac
      simp only [WP.spec_ok]
      exact ⟨fun _ => ⟨hlen, hlen' ▸ hbefore⟩, fun _ => trivial⟩
  · exact ⟨ by simp, fun j hj => by simp at hj ⟩

@[step]
theorem encloses_iff_prefix (root path : Slice U64) :
    boundary.encloses root path ⦃ b => b = true ↔ ComponentPrefix root path ⦄ := by
  unfold boundary.encloses
  dsimp only
  split
  · have h : ¬root.val.length ≤ path.val.length := by scalar_tac
    simp [WP.spec_ok, ComponentPrefix, h]
  · exact enclosing_components root path (by scalar_tac)

/-- This depth names a supplied boundary, all of whose components enclose `path`. -/
def EnclosingDepth (path : Slice U64) (roots : List (Slice U64)) (depth : Nat) : Prop :=
  ∃ r ∈ roots, ComponentPrefix r path ∧ r.val.length = depth

/-- A miss excludes every supplied root. A hit is witnessed by a record, and
no enclosing record can be deeper. Equal depth then means an identical prefix. -/
def DeepestBoundary (path : Slice U64) (roots : List (Slice U64)) : Option Usize → Prop
  | none => ∀ n, ¬ EnclosingDepth path roots n
  | some d => EnclosingDepth path roots d.val ∧ ∀ n, EnclosingDepth path roots n → n ≤ d.val

theorem enclosingDepth_snoc (path root : Slice U64) (seen : List (Slice U64)) (n : Nat) :
    EnclosingDepth path (seen ++ [root]) n ↔
      EnclosingDepth path seen n ∨ (ComponentPrefix root path ∧ root.val.length = n) := by
  simp only [EnclosingDepth, List.mem_append, List.mem_singleton]
  constructor
  · rintro ⟨ r, h | rfl, hp, hn ⟩
    · exact Or.inl ⟨ r, h, hp, hn ⟩
    · exact Or.inr ⟨ hp, hn ⟩
  · rintro (⟨ r, h, hp, hn ⟩ | ⟨ hp, hn ⟩)
    · exact ⟨ r, Or.inl h, hp, hn ⟩
    · exact ⟨ root, Or.inr rfl, hp, hn ⟩

/-- Each invocation of the extracted fold step preserves maximality, not just
membership. No precomputed “enclosing” or “deeper” flag is supplied by a caller. -/
theorem consider_preserves_maximality (path root : Slice U64) (best : Option Usize)
    (seen : List (Slice U64)) (hbest : DeepestBoundary path seen best) :
    boundary.consider path root best ⦃ r => DeepestBoundary path (seen ++ [root]) r ⦄ := by
  unfold boundary.consider
  cases best with
  | none =>
    simp only at *
    step as ⟨ b, hb ⟩
    split <;> simp only [WP.spec_ok, DeepestBoundary, enclosingDepth_snoc] at * <;> grind
  | some d =>
    simp only at *
    split
    next hle =>
      have h : root.val.length ≤ d.val := by scalar_tac
      simp only [WP.spec_ok, DeepestBoundary, enclosingDepth_snoc] at *
      grind
    next hgt =>
      have h : d.val < root.val.length := by scalar_tac
      step as ⟨ b, hb ⟩
      split <;> simp only [WP.spec_ok, DeepestBoundary, enclosingDepth_snoc] at * <;> grind

/-- The project of `path` is its deepest enclosing observed boundary, over any
mix of declared and discovered roots; `none` means no supplied boundary encloses it. -/
theorem deepest_boundary (path : Slice U64) (roots : Slice (alloc.vec.Vec U64)) :
    boundary.deepest path roots ⦃ result =>
      DeepestBoundary path (roots.val.map alloc.vec.Vec.deref) result ⦄ := by
  unfold boundary.deepest boundary.deepest_loop
  apply loop.spec_decr_nat
    (fun st : Option Usize × Usize => roots.val.length - st.2.val)
    (fun st : Option Usize × Usize => st.2.val ≤ roots.val.length ∧
      DeepestBoundary path ((roots.val.take st.2.val).map alloc.vec.Vec.deref) st.1)
  · intro ⟨ best, i ⟩ ⟨ hi, hbest ⟩
    simp only at hi hbest
    unfold boundary.deepest_loop.body
    try dsimp only
    split
    next hlt =>
      have hr : i.val < roots.val.length := by scalar_tac
      step as ⟨ root, hroot ⟩
      step with consider_preserves_maximality path (alloc.vec.Vec.deref root) best
          ((roots.val.take i.val).map alloc.vec.Vec.deref) hbest as ⟨ nextBest, hnext ⟩
      step as ⟨ i1, hi1 ⟩
      refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
      rw [show i1.val = i.val + 1 by scalar_tac, List.take_succ_eq_append_getElem hr, ← hroot]
      simpa using hnext
    next hdone =>
      have hi' : i.val = roots.val.length := by scalar_tac
      simpa [hi', WP.spec_ok] using hbest
  · simp [DeepestBoundary, EnclosingDepth]

end Strop.Kernel
