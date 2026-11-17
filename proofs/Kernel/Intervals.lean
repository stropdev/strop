import Kernel.EditMap

/-!
# Byte intervals of retained syntax (plans/0111 D9, K1 + K2)

Theorems over the Aeneas translation of `strop_kernel::intervals`
(`Extracted/StropKernel.lean`). An interval list is *sorted and disjoint*
(`Sorted`): every interval is non-empty and ends at or before the next one
starts. Outputs carry, per interval, the index of the input interval it came
from (`Holds`: byte `y` is covered by an output interval from source `i`).

- `project_spec` (K1): through an edit `s..oe` → `s..ne`, a byte is covered
  by source `i` after the edit exactly when it lies before the edit and was
  covered by `i`, or after the inserted text and its pre-image was: inserted
  text never borrows a class and surviving text never loses one. A sorted
  list stays sorted.
- `cover_spec`, `uncover_spec`, `clip_spec` (K2): union with a window,
  difference, intersection — each keeping a list sorted.
- `replace_window_spec` (K2): inside the window every byte is exactly the
  fresh result's, outside it exactly the old spans'; the result is sorted.

Each loop is reduced, once, to the list it appends (`push_loop_spec`); the
properties are stated over the extracted functions.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-! ## Push loops -/

/-- What a push loop appends from index `k` on: `F j x` for the element `x` at
index `j`, in order. -/
def pushedFrom {α γ : Type} (F : Nat → α → List γ) : Nat → List α → List γ
  | _, [] => []
  | k, x :: rest => F k x ++ pushedFrom F (k + 1) rest

theorem pushedFrom_length_le {α γ : Type} (F : Nat → α → List γ) (m : Nat)
    (hm : ∀ k x, (F k x).length ≤ m) :
    ∀ (k : Nat) (l : List α), (pushedFrom F k l).length ≤ m * l.length
  | _, [] => by simp [pushedFrom]
  | k, x :: rest => by
    have := pushedFrom_length_le F m hm (k + 1) rest
    have := hm k x
    simp only [pushedFrom, List.length_append, List.length_cons, Nat.mul_succ]
    omega

/-- A loop over `xs` whose body, at index `k`, appends `F k xs[k]` (seen
through `view`) to `out` and moves to `k + 1`, appends `pushedFrom F` over the
rest of `xs`. `m` bounds what one step appends. -/
theorem push_loop_spec {α β γ : Type} (xs : List α) (view : β → γ) (F : Nat → α → List γ)
    (m : Nat) (hm : ∀ k x, (F k x).length ≤ m)
    (body : alloc.vec.Vec β × Usize → Result (ControlFlow (alloc.vec.Vec β × Usize) (alloc.vec.Vec β)))
    (hstep : ∀ (out : alloc.vec.Vec β) (k : Usize) (hk : k.val < xs.length),
      out.val.length + m < Usize.max →
      body (out, k) ⦃ r => ∃ out' k', r = .cont (out', k') ∧ k'.val = k.val + 1 ∧
        out'.val.map view = out.val.map view ++ F k.val xs[k.val] ∧
        out'.val.length = out.val.length + (F k.val xs[k.val]).length ⦄)
    (hdone : ∀ (out : alloc.vec.Vec β) (k : Usize), xs.length ≤ k.val →
      body (out, k) ⦃ r => r = .done out ⦄)
    (out : alloc.vec.Vec β) (k : Usize) (hk : k.val ≤ xs.length)
    (hroom : out.val.length + m * (xs.length - k.val) + m < Usize.max) :
    loop body (out, k) ⦃ r =>
      r.val.map view = out.val.map view ++ pushedFrom F k.val (xs.drop k.val) ⦄ := by
  apply loop.spec_decr_nat (fun st => xs.length - st.2.val)
    (fun st => st.2.val ≤ xs.length ∧
      st.1.val.length + m * (xs.length - st.2.val) + m < Usize.max ∧
      st.1.val.map view ++ pushedFrom F st.2.val (xs.drop st.2.val) =
        out.val.map view ++ pushedFrom F k.val (xs.drop k.val))
  · intro ⟨ cur, j ⟩ ⟨ hj, hr, hinv ⟩
    simp only at hj hr hinv
    by_cases hlt : j.val < xs.length
    · apply WP.spec_mono (hstep cur j hlt (by
        have : 1 ≤ xs.length - j.val := by omega
        have := Nat.mul_le_mul_left m this
        omega))
      rintro r ⟨ out', k', rfl, hk', hout, hlen ⟩
      simp only
      have hdrop : xs.drop j.val = xs[j.val] :: xs.drop (j.val + 1) := List.drop_eq_getElem_cons hlt
      refine ⟨ ⟨ by omega, ?_, ?_ ⟩, by omega ⟩
      · have := hm j.val xs[j.val]
        have : m * (xs.length - j.val) = m * (xs.length - k'.val) + m := by
          rw [hk', show xs.length - j.val = xs.length - (j.val + 1) + 1 by omega, Nat.mul_succ]
        omega
      · rw [← hinv, hout, hk', hdrop]
        simp [pushedFrom]
    · apply WP.spec_mono (hdone cur j (by omega))
      rintro r rfl
      simp only
      rw [← hinv, List.drop_of_length_le (by omega)]
      simp [pushedFrom]
  · exact ⟨ hk, hroom, rfl ⟩

/-! ## What the appended lists mean -/

/-- Byte `y` lies in an interval of `l` (seen through `ends`) whose source is
`i` (seen through `src`). -/
def HoldsBy {γ : Type} (ends : γ → Nat × Nat) (src : γ → Nat) (l : List γ) (y i : Nat) : Prop :=
  ∃ t ∈ l, src t = i ∧ (ends t).1 ≤ y ∧ y < (ends t).2

/-- Sorted and disjoint: every interval non-empty, each ending by the start of
every later one. -/
def Sorted (l : List (Nat × Nat)) : Prop :=
  (∀ p ∈ l, p.1 < p.2) ∧ l.Pairwise (fun p q => p.2 ≤ q.1)

/-- An input list as numbers. -/
def pairs (rs : List (Usize × Usize)) : List (Nat × Nat) := rs.map fun p => (p.1.val, p.2.val)

/-- Byte `y` is in input interval `i`. -/
def InAt (rs : List (Usize × Usize)) (i y : Nat) : Prop :=
  ∃ h : i < rs.length, rs[i].1.val ≤ y ∧ y < rs[i].2.val

theorem mem_pushedFrom {α γ : Type} (F : Nat → α → List γ) (t : γ) :
    ∀ (k : Nat) (l : List α),
      t ∈ pushedFrom F k l ↔ ∃ j, ∃ h : j < l.length, t ∈ F (k + j) l[j]
  | _, [] => by simp [pushedFrom]
  | k, x :: rest => by
    simp only [pushedFrom, List.mem_append, mem_pushedFrom F t (k + 1) rest]
    constructor
    · rintro (h | ⟨ j, hj, h ⟩)
      · exact ⟨ 0, by simp, by simpa using h ⟩
      · exact ⟨ j + 1, by simp; omega, by simpa [Nat.add_assoc, Nat.add_comm 1 j] using h ⟩
    · rintro ⟨ j, hj, h ⟩
      cases j with
      | zero => exact Or.inl (by simpa using h)
      | succ j =>
        exact Or.inr ⟨ j, by simp at hj; omega, by simpa [Nat.add_assoc, Nat.add_comm 1 j] using h ⟩

theorem holdsBy_pushedFrom {α γ : Type} (ends : γ → Nat × Nat) (src : γ → Nat)
    (F : Nat → α → List γ) (k : Nat) (l : List α) (y i : Nat) :
    HoldsBy ends src (pushedFrom F k l) y i ↔
      ∃ j, ∃ h : j < l.length, HoldsBy ends src (F (k + j) l[j]) y i := by
  unfold HoldsBy
  constructor
  · rintro ⟨ t, ht, hi ⟩
    obtain ⟨ j, hj, htj ⟩ := (mem_pushedFrom F t k l).mp ht
    exact ⟨ j, hj, t, htj, hi ⟩
  · rintro ⟨ j, hj, t, htj, hi ⟩
    exact ⟨ t, (mem_pushedFrom F t k l).mpr ⟨ j, hj, htj ⟩, hi ⟩

/-- A push loop's output is sorted when each element's own output is, and
the outputs of an earlier and a later element (related by `R`) are in order. -/
theorem pushedFrom_sorted {α γ : Type} (ends : γ → Nat × Nat) (F : Nat → α → List γ)
    (R : α → α → Prop) :
    ∀ (k : Nat) (l : List α), l.Pairwise R →
      (∀ x ∈ l, ∀ k, Sorted ((F k x).map ends)) →
      (∀ x ∈ l, ∀ z ∈ l, R x z → ∀ k k', ∀ a ∈ F k x, ∀ b ∈ F k' z, (ends a).2 ≤ (ends b).1) →
      Sorted ((pushedFrom F k l).map ends)
  | _, [], _, _, _ => by simp [pushedFrom, Sorted]
  | k, x :: rest, hl, hin, hcross => by
    have hrest := pushedFrom_sorted ends F R (k + 1) rest (List.pairwise_cons.mp hl).2
      (fun z hz => hin z (by simp [hz]))
      (fun a ha b hb => hcross a (by simp [ha]) b (by simp [hb]))
    have hx := hin x (by simp) k
    simp only [pushedFrom, List.map_append, Sorted, List.mem_append, List.pairwise_append] at hx hrest ⊢
    refine ⟨ fun p hp => hp.elim (hx.1 p) (hrest.1 p), hx.2, hrest.2, ?_ ⟩
    intro a ha b hb
    obtain ⟨ a', ha', rfl ⟩ := List.mem_map.mp ha
    obtain ⟨ b', hb', rfl ⟩ := List.mem_map.mp hb
    obtain ⟨ j, hj, hb'' ⟩ := (mem_pushedFrom F b' (k + 1) rest).mp hb'
    exact hcross x (by simp) rest[j] (by simp) ((List.pairwise_cons.mp hl).1 _ (List.getElem_mem hj))
      k _ a' ha' b' hb''

/-! ## Small helpers -/

@[step]
theorem least_val_spec (a b : Usize) : intervals.least a b ⦃ r => r.val = min a.val b.val ⦄ := by
  unfold intervals.least
  split_ifs <;> simp only [WP.spec_ok] <;> scalar_tac

@[step]
theorem most_val_spec (a b : Usize) : intervals.most a b ⦃ r => r.val = max a.val b.val ⦄ := by
  unfold intervals.most
  split_ifs <;> simp only [WP.spec_ok] <;> scalar_tac

/-! ## K1: projection through an edit -/

/-- The fragments of interval `k` that survive the edit `s..oe` → `s..ne`. -/
def projFrag (s oe ne : Nat) (k : Nat) (p : Usize × Usize) : List (Nat × Nat × Nat) :=
  (if p.1.val < s then [(p.1.val, min p.2.val s, k)] else []) ++
  (if oe < p.2.val then [(mapSpec (max p.1.val oe) s oe ne, mapSpec p.2.val s oe ne, k)] else [])

/-- A pushed triple as numbers. -/
def nat3 (t : Usize × Usize × Usize) : Nat × Nat × Nat := (t.1.val, t.2.1.val, t.2.2.val)

theorem project_loop_spec (ranges : Slice (Usize × Usize)) (s oe ne : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ ranges.val.length)
    (hroom : out.val.length + 2 * (ranges.val.length - k.val) + 2 < Usize.max) :
    intervals.project_loop ranges s oe ne out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++
        pushedFrom (projFrag s.val oe.val ne.val) k.val (ranges.val.drop k.val) ⦄ := by
  unfold intervals.project_loop
  apply push_loop_spec ranges.val nat3 (projFrag s.val oe.val ne.val) 2
    (fun _ _ => by unfold projFrag; split_ifs <;> simp)
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.project_loop.body
    simp only [show j < Slice.len ranges by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : ranges.val[j.val] = (a, b) := hab.symm
    split_ifs with h1 h2 h2
    · step as ⟨ lo, hlo ⟩
      step as ⟨ o1, ho1 ⟩
      step as ⟨ mx, hmx ⟩
      step with map_position_spec as ⟨ fr, hfr ⟩
      step with map_position_spec as ⟨ t2, hto ⟩
      have : o1.val.length < Usize.max := by simp [ho1]; omega
      step as ⟨ o2, ho2 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o2, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
        simp [ho2, ho1, hab', projFrag, nat3, hlo, hmx, hfr, hto,
          show a.val < s.val by scalar_tac, show oe.val < b.val by scalar_tac]
    · step as ⟨ lo, hlo ⟩
      step as ⟨ o1, ho1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
        simp [ho1, hab', projFrag, nat3, hlo,
          show a.val < s.val by scalar_tac, show ¬ oe.val < b.val by scalar_tac]
    · step as ⟨ mx, hmx ⟩
      step with map_position_spec as ⟨ fr, hfr ⟩
      step with map_position_spec as ⟨ t2, hto ⟩
      step as ⟨ o2, ho2 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o2, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
        simp [ho2, hab', projFrag, nat3, hmx, hfr, hto,
          show ¬ a.val < s.val by scalar_tac, show oe.val < b.val by scalar_tac]
    · step as ⟨ j1, hj1 ⟩
      refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
        simp [hab', projFrag,
          show ¬ a.val < s.val by scalar_tac, show ¬ oe.val < b.val by scalar_tac]
  · intro cur j hj
    unfold intervals.project_loop.body
    simp only [show ¬ j < Slice.len ranges by scalar_tac, ↓reduceIte, WP.spec_ok]


/-- Ends and source of a projected triple. -/
abbrev ends3 (t : Nat × Nat × Nat) : Nat × Nat := (t.1, t.2.1)
abbrev src3 (t : Nat × Nat × Nat) : Nat := t.2.2

theorem projFrag_holds (s oe ne k : Nat) (p : Usize × Usize) (y i : Nat)
    (hse : s ≤ oe) (hsn : s ≤ ne) (hfit : ne + (p.2.val - oe) ≤ Usize.max) :
    HoldsBy ends3 src3 (projFrag s oe ne k p) y i ↔
      i = k ∧ ((y < s ∧ p.1.val ≤ y ∧ y < p.2.val) ∨
        (ne ≤ y ∧ p.1.val ≤ y - ne + oe ∧ y - ne + oe < p.2.val)) := by
  unfold HoldsBy projFrag mapSpec
  split_ifs <;> simp [src3] <;> omega

theorem projFrag_sorted (s oe ne k : Nat) (p : Usize × Usize)
    (hse : s ≤ oe) (hsn : s ≤ ne) (hp : p.1.val < p.2.val)
    (hfit : ne + (p.2.val - oe) ≤ Usize.max) :
    Sorted ((projFrag s oe ne k p).map ends3) := by
  unfold Sorted projFrag mapSpec
  split_ifs <;> simp [ends3] <;> omega

theorem mem_projFrag (s oe ne k : Nat) (p : Usize × Usize) (a : Nat × Nat × Nat) :
    a ∈ projFrag s oe ne k p ↔
      (p.1.val < s ∧ a = (p.1.val, min p.2.val s, k)) ∨
      (oe < p.2.val ∧ a = (mapSpec (max p.1.val oe) s oe ne, mapSpec p.2.val s oe ne, k)) := by
  unfold projFrag
  split_ifs <;> simp_all

theorem projFrag_cross (s oe ne k k' : Nat) (p q : Usize × Usize)
    (hse : s ≤ oe) (hsn : s ≤ ne) (hpq : p.2.val ≤ q.1.val)
    (hfit : ne + (q.2.val - oe) ≤ Usize.max) :
    ∀ a ∈ projFrag s oe ne k p, ∀ b ∈ projFrag s oe ne k' q, (ends3 a).2 ≤ (ends3 b).1 := by
  intro a ha b hb
  rcases (mem_projFrag _ _ _ _ _ _).mp ha with ⟨ h1, rfl ⟩ | ⟨ h1, rfl ⟩ <;>
  rcases (mem_projFrag _ _ _ _ _ _).mp hb with ⟨ h2, rfl ⟩ | ⟨ h2, rfl ⟩ <;>
  simp only [mapSpec] <;> (try split_ifs) <;> omega

/-- K1: following retained intervals through one edit `s..oe` → `s..ne`
(appended to an empty `out`). A sorted list stays sorted, and a byte `y` of
the edited buffer is covered by interval `i` exactly when it lies before the
edit and was covered by `i`, or lies after the inserted text and its
pre-image was: inserted text never borrows a class, surviving text never
loses one. -/
theorem project_spec (ranges : Slice (Usize × Usize)) (s oe ne : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (hout : out.val = [])
    (hse : s.val ≤ oe.val) (hsn : s.val ≤ ne.val)
    (hroom : 2 * ranges.val.length + 2 < Usize.max)
    (hfit : ∀ p ∈ ranges.val, ne.val + (p.2.val - oe.val) ≤ Usize.max)
    (hsorted : Sorted (pairs ranges.val)) :
    intervals.project ranges s oe ne out ⦃ r =>
      Sorted ((r.val.map nat3).map ends3) ∧
      ∀ y i, HoldsBy ends3 src3 (r.val.map nat3) y i ↔
        (y < s.val ∧ InAt ranges.val i y) ∨
        (ne.val ≤ y ∧ InAt ranges.val i (y - ne.val + oe.val)) ⦄ := by
  unfold intervals.project
  apply WP.spec_mono (project_loop_spec ranges s oe ne out 0#usize (by simp)
    (by simp [hout]; omega))
  intro r hr
  have hz : ((0#usize : Usize) : Nat) = 0 := by simp
  simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at hr
  rw [hr]
  have hne : ∀ p ∈ ranges.val, p.1.val < p.2.val := fun p hp =>
    hsorted.1 _ (List.mem_map.mpr ⟨ p, hp, rfl ⟩)
  have hpw : ranges.val.Pairwise (fun p q => p.2.val ≤ q.1.val) := by
    have := hsorted.2; unfold pairs at this; rw [List.pairwise_map] at this; exact this
  refine ⟨ pushedFrom_sorted ends3 _ _ 0 _ hpw
      (fun p hp k => projFrag_sorted _ _ _ k p hse hsn (hne p hp) (hfit p hp))
      (fun p _ q hq hpq k k' => projFrag_cross _ _ _ k k' p q hse hsn hpq (hfit q hq)), ?_ ⟩
  intro y i
  rw [holdsBy_pushedFrom]
  constructor
  · rintro ⟨ j, hj, hh ⟩
    rw [projFrag_holds _ _ _ _ _ _ _ hse hsn (hfit _ (List.getElem_mem hj))] at hh
    obtain ⟨ rfl, hb | ha ⟩ := hh
    · exact Or.inl ⟨ hb.1, by simpa using hj, by simpa using hb.2 ⟩
    · exact Or.inr ⟨ ha.1, by simpa using hj, by simpa using ha.2 ⟩
  · rintro (⟨ hy, hi, hb ⟩ | ⟨ hy, hi, ha ⟩)
    · refine ⟨ i, hi, ?_ ⟩
      rw [projFrag_holds _ _ _ _ _ _ _ hse hsn (hfit _ (List.getElem_mem hi))]
      exact ⟨ by simp, Or.inl ⟨ hy, hb ⟩ ⟩
    · refine ⟨ i, hi, ?_ ⟩
      rw [projFrag_holds _ _ _ _ _ _ _ hse hsn (hfit _ (List.getElem_mem hi))]
      exact ⟨ by simp, Or.inr ⟨ hy, ha ⟩ ⟩

/-! ## K2: coverage -/

/-- A pair as numbers. -/
abbrev nat2 (p : Usize × Usize) : Nat × Nat := (p.1.val, p.2.val)

/-- Byte `y` lies in some interval of `l`. -/
def Covers (l : List (Nat × Nat)) (y : Nat) : Prop := ∃ p ∈ l, p.1 ≤ y ∧ y < p.2

theorem covers_append (l l' : List (Nat × Nat)) (y : Nat) :
    Covers (l ++ l') y ↔ Covers l y ∨ Covers l' y := by
  unfold Covers; simp only [List.mem_append]
  constructor
  · rintro ⟨ p, hp | hp, h ⟩
    · exact Or.inl ⟨ p, hp, h ⟩
    · exact Or.inr ⟨ p, hp, h ⟩
  · rintro (⟨ p, hp, h ⟩ | ⟨ p, hp, h ⟩)
    · exact ⟨ p, Or.inl hp, h ⟩
    · exact ⟨ p, Or.inr hp, h ⟩

/-- A push loop that appends at most the element itself is a filter. -/
theorem pushedFrom_filter {α γ : Type} (c : α → Bool) (v : α → γ) :
    ∀ (k : Nat) (l : List α),
      pushedFrom (fun _ x => if c x then [v x] else []) k l = (l.filter c).map v
  | _, [] => by simp [pushedFrom]
  | k, x :: rest => by
    simp only [pushedFrom, pushedFrom_filter c v (k + 1) rest, List.filter_cons]
    split_ifs <;> simp

/-- A sublist of a sorted list, in order, is sorted. -/
theorem sorted_filter (c : Usize × Usize → Bool) (l : List (Usize × Usize))
    (h : Sorted (pairs l)) : Sorted ((l.filter c).map nat2) := by
  unfold Sorted pairs at *
  refine ⟨ fun p hp => ?_, ?_ ⟩
  · obtain ⟨ q, hq, rfl ⟩ := List.mem_map.mp hp
    exact h.1 _ (List.mem_map.mpr ⟨ q, (List.mem_filter.mp hq).1, rfl ⟩)
  · rw [List.pairwise_map] at h ⊢
    exact h.2.filter c

theorem cover_copy_spec (coverage : Slice (Usize × Usize)) (out : alloc.vec.Vec (Usize × Usize))
    (k : Usize) (hk : k.val ≤ coverage.val.length)
    (hroom : out.val.length + 1 * (coverage.val.length - k.val) + 1 < Usize.max) :
    intervals.cover_loop0 coverage out k ⦃ r =>
      r.val.map nat2 = out.val.map nat2 ++ (coverage.val.drop k.val).map nat2 ⦄ := by
  unfold intervals.cover_loop0
  have := push_loop_spec coverage.val nat2 (fun _ p => if true then [nat2 p] else []) 1
    (fun _ _ => by simp)
    (fun st => intervals.cover_loop0.body coverage st.1 st.2) ?_ ?_ out k hk hroom
  · rw [pushedFrom_filter (fun _ => true) nat2, List.filter_true] at this
    exact this
  · intro cur j hj hroom'
    unfold intervals.cover_loop0.body
    simp only [show j < Slice.len coverage by scalar_tac, ↓reduceIte]
    step as ⟨ p, hp ⟩
    step as ⟨ o1, ho1 ⟩
    step as ⟨ j1, hj1 ⟩
    refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, hp]
  · intro cur j hj
    unfold intervals.cover_loop0.body
    simp only [show ¬ j < Slice.len coverage by scalar_tac, ↓reduceIte, WP.spec_ok]

theorem cover_after_spec (coverage : Slice (Usize × Usize)) (last : Usize)
    (out : alloc.vec.Vec (Usize × Usize))
    (k : Usize) (hk : k.val ≤ coverage.val.length)
    (hroom : out.val.length + 1 * (coverage.val.length - k.val) + 1 < Usize.max) :
    intervals.cover_loop2 coverage last out k ⦃ r =>
      r.val.map nat2 = out.val.map nat2 ++
        ((coverage.val.drop k.val).filter (fun p => last.val < p.1.val)).map nat2 ⦄ := by
  unfold intervals.cover_loop2
  have := push_loop_spec coverage.val nat2
    (fun _ p => if decide (last.val < p.1.val) then [nat2 p] else []) 1
    (fun _ _ => by split_ifs <;> simp)
    (fun st => intervals.cover_loop2.body coverage last st.1 st.2) ?_ ?_ out k hk hroom
  · rw [pushedFrom_filter (fun p => decide (last.val < p.1.val)) nat2] at this
    exact this
  · intro cur j hj hroom'
    unfold intervals.cover_loop2.body
    simp only [show j < Slice.len coverage by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : coverage.val[j.val] = (a, b) := hab.symm
    by_cases h : last.val < a.val
    · simp only [show a > last by scalar_tac, ↓reduceIte]
      step as ⟨ o1, ho1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, hab', h]
    · simp only [show ¬ a > last by scalar_tac, ↓reduceIte]
      step as ⟨ j1, hj1 ⟩
      refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [hab', h]
  · intro cur j hj
    unfold intervals.cover_loop2.body
    simp only [show ¬ j < Slice.len coverage by scalar_tac, ↓reduceIte, WP.spec_ok]

set_option hygiene false in
/-- `scalar_tac` without the loop invariant's list facts, which it would try
to normalise forever. -/
local macro "lin" : tactic =>
  `(tactic| ((try clear hmem) <;> (try clear hout) <;> (try clear hdrop) <;> (try clear hcurk) <;>
    scalar_tac))

/-- Interval `p` overlaps or touches the window `first..last`. -/
def Touch (first last : Nat) (p : Usize × Usize) : Prop := first ≤ p.2.val ∧ p.1.val ≤ last

/-- The merging loop: it appends the intervals wholly before the window and
widens `low..high` by every interval that overlaps or touches it. -/
theorem cover_merge_spec (coverage : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize)) (k low high : Usize)
    (hk : k.val ≤ coverage.val.length) (hlow : low.val ≤ first.val) (hhigh : last.val ≤ high.val)
    (hroom : out.val.length + (coverage.val.length - k.val) < Usize.max) :
    intervals.cover_loop1 coverage first last out k low high ⦃ r =>
      r.1.val.map nat2 = out.val.map nat2 ++
        ((coverage.val.drop k.val).filter (fun p => p.2.val < first.val)).map nat2 ∧
      r.2.1.val ≤ first.val ∧ last.val ≤ r.2.2.val ∧
      (∀ y, (r.2.1.val ≤ y ∧ y < r.2.2.val) ↔ (low.val ≤ y ∧ y < high.val) ∨
        ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ p.1.val ≤ y ∧ y < p.2.val) ∧
      (r.2.1 = low ∨ ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ r.2.1 = p.1) ∧
      (r.2.2 = high ∨ ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ r.2.2 = p.2) ⦄ := by
  unfold intervals.cover_loop1
  apply loop.spec_decr_nat (fun st => coverage.val.length - st.2.1.val)
    (fun st => st.2.1.val ≤ coverage.val.length ∧
      k.val ≤ st.2.1.val ∧
      st.1.val.length + (coverage.val.length - st.2.1.val) < Usize.max ∧
      st.1.val.map nat2 ++
          ((coverage.val.drop st.2.1.val).filter (fun p => p.2.val < first.val)).map nat2 =
        out.val.map nat2 ++
          ((coverage.val.drop k.val).filter (fun p => p.2.val < first.val)).map nat2 ∧
      st.2.2.1.val ≤ first.val ∧ last.val ≤ st.2.2.2.val ∧
      (∀ y, ((st.2.2.1.val ≤ y ∧ y < st.2.2.2.val) ∨
          ∃ p ∈ coverage.val.drop st.2.1.val, Touch first.val last.val p ∧ p.1.val ≤ y ∧ y < p.2.val) ↔
        (low.val ≤ y ∧ y < high.val) ∨
          ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ p.1.val ≤ y ∧ y < p.2.val) ∧
      (st.2.2.1 = low ∨ ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ st.2.2.1 = p.1) ∧
      (st.2.2.2 = high ∨ ∃ p ∈ coverage.val.drop k.val, Touch first.val last.val p ∧ st.2.2.2 = p.2))
  · intro ⟨ cur, j, lo, hi ⟩ ⟨ hj, hkj, hr, hout, hlo, hhi, hmem, hwlo, hwhi ⟩
    simp only at hj hkj hr hout hlo hhi hmem hwlo hwhi
    try dsimp only
    unfold intervals.cover_loop1.body
    by_cases hlt' : j.val < coverage.val.length
    · simp only [show j < Slice.len coverage by lin, ↓reduceIte]
      have hdrop : coverage.val.drop j.val = coverage.val[j.val] :: coverage.val.drop (j.val + 1) :=
        List.drop_eq_getElem_cons hlt'
      have hcurk : coverage.val[j.val] ∈ coverage.val.drop k.val := by
        have : (coverage.val.drop j.val).Sublist (coverage.val.drop k.val) :=
          List.drop_sublist_drop_left _ (by omega)
        exact this.subset (hdrop ▸ List.mem_cons_self)
      step as ⟨ a, b, hab ⟩
      have hab' : coverage.val[j.val] = (a, b) := hab.symm
      rw [hab'] at hdrop hcurk
      split_ifs with h1 h2
      · step as ⟨ o1, ho1 ⟩
        step as ⟨ j1, hj1 ⟩
        refine ⟨ by lin, by lin, by simp only [ho1, List.length_append, List.length_singleton]; omega,
          ?_, hlo, hhi, ?_, hwlo, hwhi, by rw [hj1]; omega ⟩
        · rw [← hout, hdrop, hj1]; simp [ho1, show b.val < first.val by lin]
        · intro y; rw [← hmem y, hdrop, hj1]
          simp only [List.mem_cons, Touch]
          constructor
          · rintro (h | ⟨ p, hp, hh ⟩)
            · exact Or.inl h
            · exact Or.inr ⟨ p, Or.inr hp, hh ⟩
          · rintro (h | ⟨ p, rfl | hp, hh ⟩)
            · exact Or.inl h
            · exfalso; simp at hh; lin
            · exact Or.inr ⟨ p, hp, hh ⟩
      · step as ⟨ lo2, hlo2 ⟩
        step as ⟨ hi2, hhi2 ⟩
        step as ⟨ j1, hj1 ⟩
        have h1' : first.val ≤ b.val := by lin
        have h2' : a.val ≤ last.val := by lin
        have htouch : Touch first.val last.val (a, b) := ⟨ h1', h2' ⟩
        refine ⟨ by lin, by lin, by omega, ?_, by omega, by omega, ?_, ?_, ?_,
          by rw [hj1]; omega ⟩
        · rw [← hout, hdrop, hj1]; simp [show ¬ b.val < first.val by lin]
        · intro y; rw [← hmem y, hdrop, hj1]
          simp only [List.mem_cons]
          constructor
          · rintro (h | ⟨ p, hp, hh ⟩)
            · by_cases hy : lo.val ≤ y ∧ y < hi.val
              · exact Or.inl hy
              · exact Or.inr ⟨ (a, b), Or.inl rfl, htouch, by simp; omega ⟩
            · exact Or.inr ⟨ p, Or.inr hp, hh ⟩
          · rintro (h | ⟨ p, rfl | hp, hh ⟩)
            · exact Or.inl (by omega)
            · exact Or.inl (by simp at hh; omega)
            · exact Or.inr ⟨ p, hp, hh ⟩
        · by_cases hl : a.val < lo.val
          · exact Or.inr ⟨ (a, b), hcurk, htouch, UScalar.eq_of_val_eq (by simp only [hlo2]; omega) ⟩
          · have : lo2 = lo := UScalar.eq_of_val_eq (by simp only [hlo2]; omega)
            rw [this]; exact hwlo
        · by_cases hl : hi.val < b.val
          · exact Or.inr ⟨ (a, b), hcurk, htouch, UScalar.eq_of_val_eq (by simp only [hhi2]; omega) ⟩
          · have : hi2 = hi := UScalar.eq_of_val_eq (by simp only [hhi2]; omega)
            rw [this]; exact hwhi
      · step as ⟨ j1, hj1 ⟩
        refine ⟨ by lin, by lin, by omega, ?_, hlo, hhi, ?_, hwlo, hwhi,
          by rw [hj1]; omega ⟩
        · rw [← hout, hdrop, hj1]; simp [show ¬ b.val < first.val by lin]
        · intro y; rw [← hmem y, hdrop, hj1]
          simp only [List.mem_cons, Touch]
          constructor
          · rintro (h | ⟨ p, hp, hh ⟩)
            · exact Or.inl h
            · exact Or.inr ⟨ p, Or.inr hp, hh ⟩
          · rintro (h | ⟨ p, rfl | hp, hh ⟩)
            · exact Or.inl h
            · exfalso; simp at hh; lin
            · exact Or.inr ⟨ p, hp, hh ⟩
    · simp only [show ¬ j < Slice.len coverage by lin, ↓reduceIte, WP.spec_ok]
      have hend : coverage.val.drop j.val = [] := List.drop_of_length_le (by lin)
      refine ⟨ by rw [← hout, hend]; simp, hlo, hhi, fun y => ?_, hwlo, hwhi ⟩
      rw [← hmem y, hend]; simp
  · exact ⟨ hk, le_refl _, by omega, rfl, hlow, hhigh, fun y => Iff.rfl, Or.inl rfl, Or.inl rfl ⟩

/-- In a sorted list an interval starting before another ends by its start. -/
theorem sorted_starts (l : List (Usize × Usize)) (h : Sorted (pairs l)) {p q : Usize × Usize}
    (hp : p ∈ l) (hq : q ∈ l) (hlt : p.1.val < q.1.val) : p.2.val ≤ q.1.val := by
  obtain ⟨ hne, hpw ⟩ := h
  unfold pairs at hne hpw
  rw [List.pairwise_map, List.pairwise_iff_getElem] at hpw
  obtain ⟨ i, hi, rfl ⟩ := List.mem_iff_getElem.mp hp
  obtain ⟨ j, hj, rfl ⟩ := List.mem_iff_getElem.mp hq
  rcases Nat.lt_trichotomy i j with hij | rfl | hij
  · exact hpw i j hi hj hij
  · exact absurd hlt (Nat.lt_irrefl _)
  · have := hpw j i hj hi hij
    have := hne _ (List.mem_map.mpr ⟨ l[j], List.getElem_mem hj, rfl ⟩)
    simp only at this
    omega

/-- In a sorted list an interval ending before another ends by its start. -/
theorem sorted_ends (l : List (Usize × Usize)) (h : Sorted (pairs l)) {p q : Usize × Usize}
    (hp : p ∈ l) (hq : q ∈ l) (hlt : p.2.val < q.2.val) : p.2.val ≤ q.1.val := by
  have hne := h.1
  unfold pairs at hne
  have hpn := hne _ (List.mem_map.mpr ⟨ p, hp, rfl ⟩)
  have hqn := hne _ (List.mem_map.mpr ⟨ q, hq, rfl ⟩)
  simp only at hpn hqn
  by_cases hs : p.1.val < q.1.val
  · exact sorted_starts l h hp hq hs
  · by_cases hs' : q.1.val < p.1.val
    · have := sorted_starts l h hq hp hs'
      omega
    · -- equal starts: the same interval, or two overlapping ones
      obtain ⟨ _, hpw ⟩ := h
      unfold pairs at hpw
      rw [List.pairwise_map, List.pairwise_iff_getElem] at hpw
      obtain ⟨ i, hi, rfl ⟩ := List.mem_iff_getElem.mp hp
      obtain ⟨ j, hj, rfl ⟩ := List.mem_iff_getElem.mp hq
      rcases Nat.lt_trichotomy i j with hij | rfl | hij
      · exact hpw i j hi hj hij
      · exact absurd hlt (Nat.lt_irrefl _)
      · have := hpw j i hj hi hij
        omega

theorem covers_pairs_filter (c : Usize × Usize → Bool) (l : List (Usize × Usize)) (y : Nat) :
    Covers ((l.filter c).map nat2) y ↔ ∃ p ∈ l, c p ∧ p.1.val ≤ y ∧ y < p.2.val := by
  unfold Covers
  simp only [List.mem_map, List.mem_filter]
  constructor
  · rintro ⟨ _, ⟨ p, ⟨ hp, hc ⟩, rfl ⟩, h ⟩
    exact ⟨ p, hp, hc, h ⟩
  · rintro ⟨ p, hp, hc, h ⟩
    exact ⟨ _, ⟨ p, ⟨ hp, hc ⟩, rfl ⟩, h ⟩

/-- Three sorted pieces in order make a sorted list. -/
theorem sorted_three (A B : List (Nat × Nat)) (m : Nat × Nat) (hA : Sorted A) (hB : Sorted B)
    (hm : m.1 < m.2) (hAm : ∀ p ∈ A, p.2 ≤ m.1) (hmB : ∀ q ∈ B, m.2 ≤ q.1) :
    Sorted (A ++ [m] ++ B) := by
  unfold Sorted at *
  refine ⟨ fun p hp => ?_, ?_ ⟩
  · simp only [List.mem_append, List.mem_singleton] at hp
    rcases hp with (hp | rfl) | hp
    · exact hA.1 p hp
    · exact hm
    · exact hB.1 p hp
  · rw [List.pairwise_append, List.pairwise_append]
    refine ⟨ ⟨ hA.2, List.pairwise_singleton _ _, fun p hp q hq => ?_ ⟩, hB.2, fun p hp q hq => ?_ ⟩
    · simp only [List.mem_singleton] at hq; subst hq; exact hAm p hp
    · simp only [List.mem_append, List.mem_singleton] at hp
      rcases hp with hp | rfl
      · have := hAm p hp; have := hmB q hq; have := hm; omega
      · exact hmB q hq

/-- K2: adding the window `first..last` to a sorted coverage (appended to an
empty `out`) is set union, and the result is sorted. -/
theorem cover_spec (coverage : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize)) (hout : out.val = [])
    (hroom : 2 * coverage.val.length + 2 < Usize.max) (hsorted : Sorted (pairs coverage.val)) :
    intervals.cover coverage first last out ⦃ r =>
      Sorted (r.val.map nat2) ∧
      ∀ y, Covers (r.val.map nat2) y ↔
        Covers (pairs coverage.val) y ∨ (first.val ≤ y ∧ y < last.val) ⦄ := by
  have hz : ((0#usize : Usize) : Nat) = 0 := by simp
  unfold intervals.cover
  split
  · apply WP.spec_mono (cover_copy_spec coverage out 0#usize (by simp)
      (by simp [hout]; omega))
    intro r hr
    simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at hr
    rw [hr]
    refine ⟨ hsorted, fun y => ⟨ fun h => Or.inl h, ?_ ⟩ ⟩
    rintro (h | h)
    · exact h
    · exfalso; scalar_tac
  · rename_i hlt
    have hfl : first.val < last.val := by scalar_tac
    apply WP.spec_bind (cover_merge_spec coverage first last out 0#usize first last (by simp)
      (le_refl _) (le_refl _) (by simp [hout]; omega))
    rintro ⟨ o1, low, high ⟩ ⟨ ho1, hlow, hhigh, hmid, hwlo, hwhi ⟩
    simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at ho1 hmid hwlo hwhi
    have hl1 : o1.val.length < Usize.max := by
      have := congrArg List.length ho1
      simp only [List.length_map] at this
      have := List.length_filter_le (fun p : Usize × Usize => decide (p.2.val < first.val)) coverage.val
      omega
    step as ⟨ o2, ho2 ⟩
    apply WP.spec_mono (cover_after_spec coverage last o2 0#usize (by simp) (by
      have := congrArg List.length ho1
      simp only [List.length_map] at this
      have := List.length_filter_le (fun p : Usize × Usize => decide (p.2.val < first.val)) coverage.val
      simp only [ho2, List.length_append, List.length_singleton, hz]
      omega))
    intro r hr
    simp only [ho2, List.map_append, ho1, hz, List.drop_zero, List.map_cons, List.map_nil] at hr
    rw [hr]
    have hne : ∀ p ∈ coverage.val, p.1.val < p.2.val := fun p hp =>
      hsorted.1 _ (List.mem_map.mpr ⟨ p, hp, rfl ⟩)
    constructor
    · -- sorted: the intervals before, the merged one, the intervals after
      have hb := sorted_filter (fun p => decide (p.2.val < first.val)) coverage.val hsorted
      have ha := sorted_filter (fun p => decide (last.val < p.1.val)) coverage.val hsorted
      apply sorted_three _ _ _ hb ha (by scalar_tac)
      · intro p hp
        obtain ⟨ p', hp', rfl ⟩ := List.mem_map.mp hp
        have hp'' := List.mem_filter.mp hp'
        simp only [decide_eq_true_eq] at hp''
        show p'.2.val ≤ low.val
        rcases hwlo with h | ⟨ q, hq, ht, he ⟩
        · rw [h]; exact Nat.le_of_lt hp''.2
        · rw [he]
          exact sorted_ends coverage.val hsorted (q := q) hp''.1 hq (by unfold Touch at ht; omega)
      · intro q hq
        obtain ⟨ q', hq', rfl ⟩ := List.mem_map.mp hq
        have hq'' := List.mem_filter.mp hq'
        simp only [decide_eq_true_eq] at hq''
        show high.val ≤ q'.1.val
        rcases hwhi with h | ⟨ p, hp, ht, he ⟩
        · rw [h]; exact Nat.le_of_lt hq''.2
        · rw [he]
          exact sorted_starts coverage.val hsorted (p := p) hp hq''.1 (by unfold Touch at ht; omega)
    · intro y
      rw [covers_append, covers_append, covers_pairs_filter, covers_pairs_filter]
      have hm := hmid y
      have hsing : Covers [(low.val, high.val)] y ↔ low.val ≤ y ∧ y < high.val := by
        unfold Covers; simp
      rw [hsing, hm]
      unfold Covers pairs
      simp only [List.mem_map, decide_eq_true_eq]
      constructor
      · rintro ((⟨ p, hp, _, h ⟩ | (h | ⟨ p, hp, _, h ⟩)) | ⟨ p, hp, _, h ⟩)
        · exact Or.inl ⟨ _, ⟨ p, hp, rfl ⟩, h ⟩
        · exact Or.inr h
        · exact Or.inl ⟨ _, ⟨ p, hp, rfl ⟩, h ⟩
        · exact Or.inl ⟨ _, ⟨ p, hp, rfl ⟩, h ⟩
      · rintro (⟨ _, ⟨ p, hp, rfl ⟩, h ⟩ | h)
        · by_cases h1 : p.2.val < first.val
          · exact Or.inl (Or.inl ⟨ p, hp, h1, h ⟩)
          · by_cases h2 : last.val < p.1.val
            · exact Or.inr ⟨ p, hp, h2, h ⟩
            · exact Or.inl (Or.inr (Or.inr ⟨ p, hp, ⟨ by omega, by omega ⟩, h ⟩))
        · exact Or.inl (Or.inr (Or.inl h))

/-! ## K2: windows -/

/-- The part of interval `p` inside `L..H`, from source `k`, if any. -/
def cut (L H k : Nat) (p : Usize × Usize) : List (Nat × Nat × Nat) :=
  if max p.1.val L < min p.2.val H then [(max p.1.val L, min p.2.val H, k)] else []

theorem cut_holds (L H k : Nat) (p : Usize × Usize) (y i : Nat) :
    HoldsBy ends3 src3 (cut L H k p) y i ↔
      i = k ∧ L ≤ y ∧ y < H ∧ p.1.val ≤ y ∧ y < p.2.val := by
  unfold HoldsBy cut
  split_ifs <;> simp [src3] <;> omega

theorem cut_sorted (L H k : Nat) (p : Usize × Usize) : Sorted ((cut L H k p).map ends3) := by
  unfold Sorted cut
  split_ifs
  · simp; omega
  · simp

theorem cut_ends (L H k : Nat) (p : Usize × Usize) :
    ∀ a ∈ cut L H k p, L ≤ a.1 ∧ a.2.1 ≤ H ∧ p.1.val ≤ a.1 ∧ a.2.1 ≤ p.2.val ∧ a.2.2 = k := by
  unfold cut
  split_ifs
  · simp
  · simp

theorem holdsBy_append {γ : Type} (ends : γ → Nat × Nat) (src : γ → Nat) (l l' : List γ) (y i : Nat) :
    HoldsBy ends src (l ++ l') y i ↔ HoldsBy ends src l y i ∨ HoldsBy ends src l' y i := by
  unfold HoldsBy
  simp only [List.mem_append]
  constructor
  · rintro ⟨ t, ht | ht, h ⟩
    · exact Or.inl ⟨ t, ht, h ⟩
    · exact Or.inr ⟨ t, ht, h ⟩
  · rintro (⟨ t, ht, h ⟩ | ⟨ t, ht, h ⟩)
    · exact ⟨ t, Or.inl ht, h ⟩
    · exact ⟨ t, Or.inr ht, h ⟩

theorem sorted_append (A B : List (Nat × Nat)) (hA : Sorted A) (hB : Sorted B)
    (hAB : ∀ p ∈ A, ∀ q ∈ B, p.2 ≤ q.1) : Sorted (A ++ B) := by
  unfold Sorted at *
  refine ⟨ fun p hp => ?_, List.pairwise_append.mpr ⟨ hA.2, hB.2, hAB ⟩ ⟩
  rcases List.mem_append.mp hp with hp | hp
  · exact hA.1 p hp
  · exact hB.1 p hp

/-- The intervals cut from a sorted list by one window are sorted. -/
theorem cut_list_sorted (L H : Nat) (l : List (Usize × Usize)) (h : Sorted (pairs l)) :
    Sorted ((pushedFrom (cut L H) 0 l).map ends3) := by
  have hpw : l.Pairwise (fun p q => p.2.val ≤ q.1.val) := by
    have := h.2; unfold pairs at this; rw [List.pairwise_map] at this; exact this
  exact pushedFrom_sorted ends3 _ _ 0 l hpw (fun p _ k => cut_sorted L H k p)
    (fun p _ q _ hpq k k' a ha b hb => by
      have := cut_ends L H k p a ha
      have := cut_ends L H k' q b hb
      show a.2.1 ≤ b.1
      omega)

theorem cut_list_holds (L H : Nat) (l : List (Usize × Usize)) (y i : Nat) :
    HoldsBy ends3 src3 (pushedFrom (cut L H) 0 l) y i ↔ L ≤ y ∧ y < H ∧ InAt l i y := by
  rw [holdsBy_pushedFrom]
  constructor
  · rintro ⟨ j, hj, h ⟩
    rw [cut_holds] at h
    obtain ⟨ rfl, h1, h2, h3, h4 ⟩ := h
    exact ⟨ h1, h2, by simpa using hj, by simpa using h3, by simpa using h4 ⟩
  · rintro ⟨ h1, h2, hi, h3, h4 ⟩
    exact ⟨ i, hi, (cut_holds _ _ _ _ _ _).mpr ⟨ by simp, h1, h2, h3, h4 ⟩ ⟩

/-- Every interval cut from `l` by `L..H` lies in that window. -/
theorem cut_list_ends (L H : Nat) (l : List (Usize × Usize)) :
    ∀ a ∈ (pushedFrom (cut L H) 0 l).map ends3, L ≤ a.1 ∧ a.2 ≤ H := by
  intro a ha
  obtain ⟨ t, ht, rfl ⟩ := List.mem_map.mp ha
  obtain ⟨ j, hj, htj ⟩ := (mem_pushedFrom _ t 0 l).mp ht
  have := cut_ends L H _ _ t htj
  exact ⟨ this.1, this.2.1 ⟩

theorem cut_of_lt {L H k : Nat} {p : Usize × Usize} (h : max p.1.val L < min p.2.val H) :
    cut L H k p = [(max p.1.val L, min p.2.val H, k)] := by
  unfold cut; simp [h]

theorem cut_of_ge {L H k : Nat} {p : Usize × Usize} (h : ¬ max p.1.val L < min p.2.val H) :
    cut L H k p = [] := by
  unfold cut; simp [h]

theorem uncover_loop_spec (coverage : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ coverage.val.length)
    (hroom : out.val.length + 2 * (coverage.val.length - k.val) + 2 < Usize.max) :
    intervals.uncover_loop coverage first last out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++
        pushedFrom (fun j p => cut 0 first.val j p ++ cut last.val Usize.max j p) k.val
          (coverage.val.drop k.val) ⦄ := by
  unfold intervals.uncover_loop
  apply push_loop_spec coverage.val nat3 (fun j p => cut 0 first.val j p ++ cut last.val Usize.max j p) 2
    (fun _ _ => by unfold cut; split_ifs <;> simp)
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.uncover_loop.body
    simp only [show j < Slice.len coverage by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : coverage.val[j.val] = (a, b) := hab.symm
    rw [hab']
    have hbm : b.val ≤ Usize.max := by scalar_tac
    step as ⟨ below, hbelow ⟩
    have hc1 : (a.val < below.val) = (max (a, b).1.val 0 < min (a, b).2.val first.val) := by
      simp [hbelow]
    by_cases h1 : a.val < below.val
    · rw [cut_of_lt (hc1 ▸ h1)]
      simp only [show a < below by scalar_tac, ↓reduceIte]
      have : cur.val.length < Usize.max := by omega
      step as ⟨ o1, ho1 ⟩
      step as ⟨ above, habove ⟩
      by_cases h2 : above.val < b.val
      · rw [cut_of_lt (by simp only; omega)]
        simp only [show above < b by scalar_tac, ↓reduceIte]
        have : o1.val.length < Usize.max := by simp [ho1]; omega
        step as ⟨ o2, ho2 ⟩
        step as ⟨ j1, hj1 ⟩
        refine ⟨ o2, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
          simp [ho2, ho1, nat3, hbelow, habove, Nat.min_eq_left hbm]
      · rw [cut_of_ge (by simp only; omega)]
        simp only [show ¬ above < b by scalar_tac, ↓reduceIte]
        step as ⟨ j1, hj1 ⟩
        refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, nat3, hbelow]
    · rw [cut_of_ge (hc1 ▸ h1)]
      simp only [show ¬ a < below by scalar_tac, ↓reduceIte]
      step as ⟨ above, habove ⟩
      by_cases h2 : above.val < b.val
      · rw [cut_of_lt (by simp only; omega)]
        simp only [show above < b by scalar_tac, ↓reduceIte]
        have : cur.val.length < Usize.max := by omega
        step as ⟨ o2, ho2 ⟩
        step as ⟨ j1, hj1 ⟩
        refine ⟨ o2, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
          simp [ho2, nat3, habove, Nat.min_eq_left hbm]
      · rw [cut_of_ge (by simp only; omega)]
        simp only [show ¬ above < b by scalar_tac, ↓reduceIte]
        step as ⟨ j1, hj1 ⟩
        refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp
  · intro cur j hj
    unfold intervals.uncover_loop.body
    simp only [show ¬ j < Slice.len coverage by scalar_tac, ↓reduceIte, WP.spec_ok]

theorem replace_below_spec (spans : Slice (Usize × Usize)) (first : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ spans.val.length)
    (hroom : out.val.length + 1 * (spans.val.length - k.val) + 1 < Usize.max) :
    intervals.replace_window_loop0 spans first out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++ pushedFrom (cut 0 first.val) k.val (spans.val.drop k.val) ⦄ := by
  unfold intervals.replace_window_loop0
  apply push_loop_spec spans.val nat3 (cut 0 first.val) 1
    (fun _ _ => by unfold cut; split_ifs <;> simp)
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.replace_window_loop0.body
    simp only [show j < Slice.len spans by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : spans.val[j.val] = (a, b) := hab.symm
    rw [hab']
    step as ⟨ below, hbelow ⟩
    have hc1 : (a.val < below.val) = (max (a, b).1.val 0 < min (a, b).2.val first.val) := by
      simp [hbelow]
    by_cases h1 : a.val < below.val
    · rw [cut_of_lt (hc1 ▸ h1)]
      simp only [show a < below by scalar_tac, ↓reduceIte]
      have : cur.val.length < Usize.max := by omega
      step as ⟨ o1, ho1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, nat3, hbelow]
    · rw [cut_of_ge (hc1 ▸ h1)]
      simp only [show ¬ a < below by scalar_tac, ↓reduceIte]
      step as ⟨ j1, hj1 ⟩
      refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp
  · intro cur j hj
    unfold intervals.replace_window_loop0.body
    simp only [show ¬ j < Slice.len spans by scalar_tac, ↓reduceIte, WP.spec_ok]

theorem replace_above_spec (spans : Slice (Usize × Usize)) (last : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ spans.val.length)
    (hroom : out.val.length + 1 * (spans.val.length - k.val) + 1 < Usize.max) :
    intervals.replace_window_loop2 spans last out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++
        pushedFrom (cut last.val Usize.max) k.val (spans.val.drop k.val) ⦄ := by
  unfold intervals.replace_window_loop2
  apply push_loop_spec spans.val nat3 (cut last.val Usize.max) 1
    (fun _ _ => by unfold cut; split_ifs <;> simp)
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.replace_window_loop2.body
    simp only [show j < Slice.len spans by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : spans.val[j.val] = (a, b) := hab.symm
    rw [hab']
    have hbm : b.val ≤ Usize.max := by scalar_tac
    step as ⟨ above, habove ⟩
    by_cases h2 : above.val < b.val
    · rw [cut_of_lt (by simp only; omega)]
      simp only [show above < b by scalar_tac, ↓reduceIte]
      have : cur.val.length < Usize.max := by omega
      step as ⟨ o1, ho1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;>
        simp [ho1, nat3, habove, Nat.min_eq_left hbm]
    · rw [cut_of_ge (by simp only; omega)]
      simp only [show ¬ above < b by scalar_tac, ↓reduceIte]
      step as ⟨ j1, hj1 ⟩
      refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp
  · intro cur j hj
    unfold intervals.replace_window_loop2.body
    simp only [show ¬ j < Slice.len spans by scalar_tac, ↓reduceIte, WP.spec_ok]

/-- A fresh interval with its source past the old ones. -/
def freshAt (n k : Nat) (p : Usize × Usize) : List (Nat × Nat × Nat) := [(p.1.val, p.2.val, n + k)]

theorem replace_fresh_spec (spans fresh : Slice (Usize × Usize))
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ fresh.val.length)
    (hidx : spans.val.length + fresh.val.length ≤ Usize.max)
    (hroom : out.val.length + 1 * (fresh.val.length - k.val) + 1 < Usize.max) :
    intervals.replace_window_loop1 spans fresh out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++
        pushedFrom (freshAt spans.val.length) k.val (fresh.val.drop k.val) ⦄ := by
  unfold intervals.replace_window_loop1
  apply push_loop_spec fresh.val nat3 (freshAt spans.val.length) 1
    (fun _ _ => by simp [freshAt])
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.replace_window_loop1.body
    simp only [show j < Slice.len fresh by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : fresh.val[j.val] = (a, b) := hab.symm
    rw [hab']
    step as ⟨ n, hn ⟩
    have : cur.val.length < Usize.max := by omega
    step as ⟨ o1, ho1 ⟩
    step as ⟨ j1, hj1 ⟩
    refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, nat3, freshAt, hn]
  · intro cur j hj
    unfold intervals.replace_window_loop1.body
    simp only [show ¬ j < Slice.len fresh by scalar_tac, ↓reduceIte, WP.spec_ok]

theorem clip_loop_spec (spans : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (k : Usize) (hk : k.val ≤ spans.val.length)
    (hroom : out.val.length + 1 * (spans.val.length - k.val) + 1 < Usize.max) :
    intervals.clip_loop spans first last out k ⦃ r =>
      r.val.map nat3 = out.val.map nat3 ++
        pushedFrom (cut first.val last.val) k.val (spans.val.drop k.val) ⦄ := by
  unfold intervals.clip_loop
  apply push_loop_spec spans.val nat3 (cut first.val last.val) 1
    (fun _ _ => by unfold cut; split_ifs <;> simp)
    _ _ _ out k hk hroom
  · intro cur j hj hroom'
    unfold intervals.clip_loop.body
    simp only [show j < Slice.len spans by scalar_tac, ↓reduceIte]
    step as ⟨ a, b, hab ⟩
    have hab' : spans.val[j.val] = (a, b) := hab.symm
    rw [hab']
    step as ⟨ lo, hlo ⟩
    step as ⟨ hi, hhi ⟩
    by_cases h1 : lo.val < hi.val
    · rw [cut_of_lt (by simp only; omega)]
      simp only [show lo < hi by scalar_tac, ↓reduceIte]
      have : cur.val.length < Usize.max := by omega
      step as ⟨ o1, ho1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ o1, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp [ho1, nat3, hlo, hhi]
    · rw [cut_of_ge (by simp only; omega)]
      simp only [show ¬ lo < hi by scalar_tac, ↓reduceIte]
      step as ⟨ j1, hj1 ⟩
      refine ⟨ cur, j1, rfl, by scalar_tac, ?_, ?_ ⟩ <;> simp
  · intro cur j hj
    unfold intervals.clip_loop.body
    simp only [show ¬ j < Slice.len spans by scalar_tac, ↓reduceIte, WP.spec_ok]

theorem input_pairwise (l : List (Usize × Usize)) (h : Sorted (pairs l)) :
    l.Pairwise (fun p q => p.2.val ≤ q.1.val) := by
  have := h.2; unfold pairs at this; rw [List.pairwise_map] at this; exact this

/-- K2: removing the window `first..last` from a sorted coverage (appended to
an empty `out`) is set difference — per source interval — and the result is
sorted. -/
theorem uncover_spec (coverage : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (hout : out.val = [])
    (hfl : first.val ≤ last.val) (hroom : 2 * coverage.val.length + 2 < Usize.max)
    (hsorted : Sorted (pairs coverage.val)) :
    intervals.uncover coverage first last out ⦃ r =>
      Sorted ((r.val.map nat3).map ends3) ∧
      ∀ y i, HoldsBy ends3 src3 (r.val.map nat3) y i ↔
        InAt coverage.val i y ∧ ¬ (first.val ≤ y ∧ y < last.val) ⦄ := by
  have hz : ((0#usize : Usize) : Nat) = 0 := by simp
  unfold intervals.uncover
  apply WP.spec_mono (uncover_loop_spec coverage first last out 0#usize (by simp)
    (by simp [hout]; omega))
  intro r hr
  simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at hr
  rw [hr]
  constructor
  · refine pushedFrom_sorted ends3 _ _ 0 _ (input_pairwise _ hsorted) (fun p _ k => ?_)
      (fun p _ q _ hpq k k' a ha b hb => ?_)
    · rw [List.map_append]
      refine sorted_append _ _ (cut_sorted _ _ _ _) (cut_sorted _ _ _ _) (fun a ha b hb => ?_)
      obtain ⟨ a', ha', rfl ⟩ := List.mem_map.mp ha
      obtain ⟨ b', hb', rfl ⟩ := List.mem_map.mp hb
      have := cut_ends _ _ _ _ a' ha'
      have := cut_ends _ _ _ _ b' hb'
      show a'.2.1 ≤ b'.1
      omega
    · rcases List.mem_append.mp ha with ha | ha <;> rcases List.mem_append.mp hb with hb | hb <;>
      · have := cut_ends _ _ _ _ a ha
        have := cut_ends _ _ _ _ b hb
        show a.2.1 ≤ b.1
        omega
  · intro y i
    rw [holdsBy_pushedFrom]
    constructor
    · rintro ⟨ j, hj, h ⟩
      rw [holdsBy_append, cut_holds, cut_holds] at h
      rcases h with ⟨ rfl, _, h2, h3, h4 ⟩ | ⟨ rfl, h1, _, h3, h4 ⟩
      · exact ⟨ ⟨ by simpa using hj, by simpa using h3, by simpa using h4 ⟩, by omega ⟩
      · exact ⟨ ⟨ by simpa using hj, by simpa using h3, by simpa using h4 ⟩, by omega ⟩
    · rintro ⟨ ⟨ hi, h3, h4 ⟩, hw ⟩
      refine ⟨ i, by simpa using hi, ?_ ⟩
      rw [holdsBy_append, cut_holds, cut_holds]
      have hbm : coverage.val[i].2.val ≤ Usize.max := by scalar_tac
      by_cases hy : y < first.val
      · exact Or.inl ⟨ by simp, by omega, hy, by simpa using h3, by simpa using h4 ⟩
      · exact Or.inr ⟨ by simp, by omega, by omega, by simpa using h3, by simpa using h4 ⟩

/-- K2: the parts of sorted spans inside `first..last` (appended to an empty
`out`) are exactly the intersection — per source span — and sorted. -/
theorem clip_spec (spans : Slice (Usize × Usize)) (first last : Usize)
    (out : alloc.vec.Vec (Usize × Usize × Usize)) (hout : out.val = [])
    (hroom : spans.val.length + 1 < Usize.max) (hsorted : Sorted (pairs spans.val)) :
    intervals.clip spans first last out ⦃ r =>
      Sorted ((r.val.map nat3).map ends3) ∧
      ∀ y i, HoldsBy ends3 src3 (r.val.map nat3) y i ↔
        first.val ≤ y ∧ y < last.val ∧ InAt spans.val i y ⦄ := by
  have hz : ((0#usize : Usize) : Nat) = 0 := by simp
  unfold intervals.clip
  apply WP.spec_mono (clip_loop_spec spans first last out 0#usize (by simp)
    (by simp [hout]; omega))
  intro r hr
  simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at hr
  rw [hr]
  exact ⟨ cut_list_sorted _ _ _ hsorted, fun y i => cut_list_holds _ _ _ y i ⟩

theorem fresh_list_holds (n : Nat) (l : List (Usize × Usize)) (y i : Nat) :
    HoldsBy ends3 src3 (pushedFrom (freshAt n) 0 l) y i ↔ ∃ j, i = n + j ∧ InAt l j y := by
  rw [holdsBy_pushedFrom]
  simp only [HoldsBy, freshAt, List.mem_singleton, exists_eq_left, src3, Nat.zero_add]
  constructor
  · rintro ⟨ j, hj, rfl, h ⟩
    exact ⟨ j, rfl, hj, h ⟩
  · rintro ⟨ j, rfl, hj, h ⟩
    exact ⟨ j, hj, rfl, h ⟩

theorem fresh_list_sorted (n : Nat) (l : List (Usize × Usize)) (h : Sorted (pairs l)) :
    Sorted ((pushedFrom (freshAt n) 0 l).map ends3) := by
  refine pushedFrom_sorted ends3 _ _ 0 l (input_pairwise l h) (fun p hp k => ?_)
    (fun p _ q _ hpq k k' a ha b hb => ?_)
  · have := h.1 _ (List.mem_map.mpr ⟨ p, hp, rfl ⟩)
    simp [Sorted, freshAt]
    exact this
  · simp only [freshAt, List.mem_singleton] at ha hb
    subst ha; subst hb
    exact hpq

theorem fresh_list_ends (n first last : Nat) (l : List (Usize × Usize))
    (hin : ∀ p ∈ l, first ≤ p.1.val ∧ p.2.val ≤ last) :
    ∀ a ∈ (pushedFrom (freshAt n) 0 l).map ends3, first ≤ a.1 ∧ a.2 ≤ last := by
  intro a ha
  obtain ⟨ t, ht, rfl ⟩ := List.mem_map.mp ha
  obtain ⟨ j, hj, htj ⟩ := (mem_pushedFrom _ t 0 l).mp ht
  simp only [freshAt, List.mem_singleton] at htj
  subst htj
  exact hin _ (List.getElem_mem hj)

/-- K2: an exact result `fresh` (sorted, inside `first..last`) replacing that
window of sorted `spans` (appended to an empty `out`): inside the window every
byte is exactly the fresh result's — source `spans.length + j` for fresh span
`j` — outside it exactly the old spans'; the result is sorted. -/
theorem replace_window_spec (spans : Slice (Usize × Usize)) (first last : Usize)
    (fresh : Slice (Usize × Usize)) (out : alloc.vec.Vec (Usize × Usize × Usize))
    (hout : out.val = []) (hfl : first.val ≤ last.val)
    (hroom : 2 * spans.val.length + fresh.val.length + 3 < Usize.max)
    (hsorted : Sorted (pairs spans.val)) (hfresh : Sorted (pairs fresh.val))
    (hin : ∀ p ∈ fresh.val, first.val ≤ p.1.val ∧ p.2.val ≤ last.val) :
    intervals.replace_window spans first last fresh out ⦃ r =>
      Sorted ((r.val.map nat3).map ends3) ∧
      ∀ y i, HoldsBy ends3 src3 (r.val.map nat3) y i ↔
        (first.val ≤ y ∧ y < last.val ∧ ∃ j, i = spans.val.length + j ∧ InAt fresh.val j y) ∨
        (¬ (first.val ≤ y ∧ y < last.val) ∧ InAt spans.val i y) ⦄ := by
  have hz : ((0#usize : Usize) : Nat) = 0 := by simp
  have hlenA : ∀ l : List (Usize × Usize), ∀ L H, (pushedFrom (cut L H) 0 l).length ≤ l.length :=
    fun l L H => by
      have := pushedFrom_length_le (cut L H) 1 (fun _ _ => by unfold cut; split_ifs <;> simp) 0 l
      omega
  have hlenF : ∀ l : List (Usize × Usize), ∀ n, (pushedFrom (freshAt n) 0 l).length ≤ l.length :=
    fun l n => by
      have := pushedFrom_length_le (freshAt n) 1 (fun _ _ => by simp [freshAt]) 0 l
      omega
  unfold intervals.replace_window
  apply WP.spec_bind (replace_below_spec spans first out 0#usize (by simp) (by simp [hout]; omega))
  intro o1 h1
  simp only [hout, List.map_nil, List.nil_append, hz, List.drop_zero] at h1
  have hl1 : o1.val.length ≤ spans.val.length := by
    have := congrArg List.length h1; simp only [List.length_map] at this
    have := hlenA spans.val 0 first.val; omega
  apply WP.spec_bind (replace_fresh_spec spans fresh o1 0#usize (by simp) (by omega) (by omega))
  intro o2 h2
  simp only [h1, hz, List.drop_zero] at h2
  have hl2 : o2.val.length ≤ spans.val.length + fresh.val.length := by
    have := congrArg List.length h2; simp only [List.length_map, List.length_append] at this
    have := hlenA spans.val 0 first.val; have := hlenF fresh.val spans.val.length; omega
  apply WP.spec_mono (replace_above_spec spans last o2 0#usize (by simp) (by omega))
  intro r hr
  simp only [h2, hz, List.drop_zero] at hr
  rw [hr]
  have hA := cut_list_sorted 0 first.val spans.val hsorted
  have hF := fresh_list_sorted spans.val.length fresh.val hfresh
  have hB := cut_list_sorted last.val Usize.max spans.val hsorted
  have eA := cut_list_ends 0 first.val spans.val
  have eF := fresh_list_ends spans.val.length first.val last.val fresh.val hin
  have eB := cut_list_ends last.val Usize.max spans.val
  constructor
  · simp only [List.map_append]
    refine sorted_append _ _ (sorted_append _ _ hA hF (fun a ha b hb => ?_)) hB (fun a ha b hb => ?_)
    · have := eA a ha; have := eF b hb; omega
    · rcases List.mem_append.mp ha with ha | ha
      · have := eA a ha; have := eB b hb; omega
      · have := eF a ha; have := eB b hb; omega
  · intro y i
    rw [holdsBy_append, holdsBy_append, cut_list_holds, cut_list_holds, fresh_list_holds]
    constructor
    · rintro ((⟨ _, h2, hi ⟩ | ⟨ j, rfl, hj ⟩) | ⟨ h1, _, hi ⟩)
      · exact Or.inr ⟨ by omega, hi ⟩
      · obtain ⟨ hjl, h3, h4 ⟩ := hj
        have := hin _ (List.getElem_mem hjl)
        exact Or.inl ⟨ by omega, by omega, j, rfl, hjl, h3, h4 ⟩
      · exact Or.inr ⟨ by omega, hi ⟩
    · rintro (⟨ _, _, j, rfl, hj ⟩ | ⟨ hw, hi ⟩)
      · exact Or.inl (Or.inr ⟨ j, rfl, hj ⟩)
      · obtain ⟨ hil, h3, h4 ⟩ := hi
        have hbm : spans.val[i].2.val ≤ Usize.max := by scalar_tac
        by_cases hy : y < first.val
        · exact Or.inl (Or.inl ⟨ by omega, hy, hil, h3, h4 ⟩)
        · exact Or.inr ⟨ by omega, by omega, hil, h3, h4 ⟩

end Strop.Kernel
