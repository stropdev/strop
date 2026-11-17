import Kernel.EditMap

/-!
# Spans and plans through edits (plans/0108 D4)

Theorems over the Aeneas translation of `strop_kernel::editmap::map_span` and
`strop_kernel::editmap::keep_disjoint`, stated directly over the extracted
functions (no model in between):

- `map_span_spec`: a span stays ordered and inside the edited buffer; an edit
  wholly before it shifts it by the delta, one wholly after it (or starting at
  its end, unless the caller claims the insertion) leaves it alone, and one
  inside it changes its length by exactly the delta. Collection excerpts,
  collection view excerpts and completion journal windows map through it.
- `map_span_keeps_order`: two ordered spans stay ordered through any edit that
  does not straddle the second one's start, so excerpts never come to overlap.
- `keep_disjoint_spec`: the multicursor plan filter keeps a range exactly when
  it starts at or after the end of every range kept before it, and the kept
  non-empty ranges form a batch `validate_batch` accepts (`BatchOk`).
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-! ## One span through one edit -/

/-- `map_span` over an edit `s..e` → `s..ne`, for every span `a..b` whose
shifted end fits a `usize` (byte offsets into a rope always do). -/
theorem map_span_spec (a b s e ne : Usize) (g : Bool) (hab : a.val ≤ b.val)
    (hse : s.val ≤ e.val) (hsn : s.val ≤ ne.val)
    (hfit : ne.val + (b.val - e.val) ≤ Usize.max) :
    editmap.map_span a b s e ne g ⦃ r =>
      -- a span stays a span
      r.1.val ≤ r.2.val ∧
      -- and stays inside the edited buffer
      (∀ len : Nat, b.val ≤ len → e.val ≤ len → r.2.val ≤ len - e.val + ne.val) ∧
      -- an edit wholly before the span shifts it by the delta
      (e.val ≤ a.val → s.val < a.val →
        r.1.val = ne.val + (a.val - e.val) ∧ r.2.val = ne.val + (b.val - e.val)) ∧
      -- an edit wholly after the span, or starting at its end without being
      -- claimed, leaves it alone
      (b.val < s.val ∨ (b.val = s.val ∧ g = false) → r.1.val = a.val ∧ r.2.val = b.val) ∧
      -- an edit inside the span takes its replacement in: the length changes
      -- by exactly the delta
      (a.val ≤ s.val → e.val ≤ b.val → (s.val < b.val ∨ g = true) →
        r.1.val = a.val ∧ r.2.val = ne.val + (b.val - e.val)) ∧
      -- a start at or before the edit stays; one inside the replaced range
      -- lands on the edit's start
      (a.val ≤ s.val → r.1.val = a.val) ∧ (s.val < a.val → a.val < e.val → r.1.val = s.val) ⦄ := by
  unfold editmap.map_span
  cases g <;> split_ifs <;> (try simp only [lift]) <;> step* <;>
    (try simp only [saturating_add_val]) <;>
    (repeat' (first | apply And.intro | intro)) <;>
    first | trivial | scalar_tac

/-- The end of a span lands no later than where the start of a span after it
lands: the half of `map_span_keeps_order` about the first span. -/
theorem map_span_end_before (a b s e ne : Usize) (g : Bool) (hse : s.val ≤ e.val)
    (hsn : s.val ≤ ne.val) :
    editmap.map_span a b s e ne g ⦃ r => ∀ a₂ : Nat, b.val ≤ a₂ → ne.val + (a₂ - e.val) ≤ Usize.max →
      (a₂ ≤ s.val ∨ e.val ≤ a₂) → (g = true → b.val < a₂) →
      r.2.val ≤ (if a₂ ≤ s.val then a₂ else ne.val + (a₂ - e.val)) ⦄ := by
  unfold editmap.map_span
  cases g <;> split_ifs <;> (try simp only [lift]) <;> step* <;>
    (try simp only [saturating_add_val]) <;> intro a₂ h12 hfit hstraddle hgrow <;>
    split_ifs <;> scalar_tac

/-- Two ordered spans stay ordered through one edit, unless the edit straddles
the second span's start or the first span claims an insertion where the second
one begins: excerpts never come to overlap. -/
theorem map_span_keeps_order (a₁ b₁ a₂ b₂ s e ne : Usize) (g₁ g₂ : Bool)
    (h12 : b₁.val ≤ a₂.val) (h2 : a₂.val ≤ b₂.val) (hse : s.val ≤ e.val) (hsn : s.val ≤ ne.val)
    (hfit : ne.val + (b₂.val - e.val) ≤ Usize.max)
    (hstraddle : a₂.val ≤ s.val ∨ e.val ≤ a₂.val) (hgrow : g₁ = true → b₁.val < a₂.val) :
    editmap.map_span a₁ b₁ s e ne g₁ ⦃ r₁ =>
      editmap.map_span a₂ b₂ s e ne g₂ ⦃ r₂ => r₁.2.val ≤ r₂.1.val ⦄ ⦄ := by
  apply WP.spec_mono (map_span_end_before a₁ b₁ s e ne g₁ hse hsn)
  intro r₁ h₁
  apply WP.spec_mono (map_span_spec a₂ b₂ s e ne g₂ h2 hse hsn hfit)
  intro r₂ ⟨ _, _, hafter, _, _, hstay, _ ⟩
  have := h₁ a₂.val h12 (by omega) hstraddle hgrow
  split_ifs at this with h
  · rw [hstay h]; exact this
  · rw [(hafter (by omega) (by omega)).1]; exact this

/-! ## The multicursor plan filter -/

/-- The greedy verdict: a range is kept exactly when it starts at or after the
end of every range kept before it. -/
def Greedy (rs : List (Usize × Usize)) (keep : List Bool) : Prop :=
  ∃ hl : keep.length = rs.length,
  ∀ i (hi : i < rs.length), (keep[i]'(by omega) = true ↔
    ∀ j (hj : j < i), keep[j]'(by omega) = true → (rs[j]'(by omega)).2.val ≤ rs[i].1.val)

/-- The kept non-empty ranges, in input order: the batch a plan edits. -/
def keptNonEmpty (rs : List (Usize × Usize)) (keep : List Bool) : List (Usize × Usize) :=
  ((rs.zip keep).filter (fun p => p.2 && decide (p.1.1.val < p.1.2.val))).map Prod.fst

theorem keep_disjoint_loop_spec (ranges : Slice (Usize × Usize)) (keep : Slice Bool)
    (hlen : keep.val.length = ranges.val.length)
    (hwf : ∀ r ∈ ranges.val, r.1.val ≤ r.2.val) :
    editmap.keep_disjoint_loop ranges keep 0#usize 0#usize ⦃ out => Greedy ranges.val out.val ⦄ := by
  unfold editmap.keep_disjoint_loop
  apply loop.spec_decr_nat
    (fun (st : Slice Bool × Usize × Usize) => ranges.val.length - st.2.2.val)
    (fun (st : Slice Bool × Usize × Usize) =>
      ∃ hl : st.1.val.length = ranges.val.length, st.2.2.val ≤ ranges.val.length ∧
        (∀ i (hi : i < ranges.val.length), i < st.2.2.val →
          (st.1.val[i]'(by omega) = true ↔
            ∀ j (hj : j < i), st.1.val[j]'(by omega) = true →
              (ranges.val[j]'(by omega)).2.val ≤ ranges.val[i].1.val)) ∧
        (∀ j (hj : j < ranges.val.length), j < st.2.2.val → st.1.val[j]'(by omega) = true →
          ranges.val[j].2.val ≤ st.2.1.val) ∧
        (st.2.1.val = 0 ∨ ∃ j, ∃ hj : j < ranges.val.length, j < st.2.2.val ∧
          st.1.val[j]'(by omega) = true ∧ ranges.val[j].2.val = st.2.1.val))
  · intro ⟨ cur, reach, k ⟩ ⟨ hl, hk, hchar, hreach, hwit ⟩
    simp only at hl hk hchar hreach hwit
    unfold editmap.keep_disjoint_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : k.val < ranges.val.length := by scalar_tac
      split
      next hlt2 =>
        step as ⟨ st, en, hcur ⟩
        have hcur' : ranges.val[k.val] = (st, en) := hcur.symm
        have hwfk : st.val ≤ en.val := by
          have := hwf _ (List.getElem_mem hlt'); rw [hcur'] at this; exact this
        split
        next hge =>
          step as ⟨ nv, hnv ⟩
          step as ⟨ k1, hk1 ⟩
          have hnvl : nv.val.length = ranges.val.length := by simp [hnv, hl]
          have hget : ∀ j (hj : j < ranges.val.length), nv.val[j]'(by omega) =
              if k.val = j then true else cur.val[j]'(by omega) := by
            intro j hj; simp [hnv, List.getElem_set]
          refine ⟨ ⟨ hnvl, by scalar_tac, ?_, ?_, ?_ ⟩, by scalar_tac ⟩
          · intro i hi hik
            rw [hget i hi]
            by_cases hik' : i < k.val
            · simp only [show ¬ k.val = i by omega, ↓reduceIte]
              rw [hchar i hi hik']
              constructor
              · intro h j hj hkj
                rw [hget j (by omega)] at hkj
                simp only [show ¬ k.val = j by omega, ↓reduceIte] at hkj
                exact h j hj hkj
              · intro h j hj hkj
                apply h j hj
                rw [hget j (by omega)]
                simp only [show ¬ k.val = j by omega, ↓reduceIte]
                exact hkj
            · have hik : i = k.val := by scalar_tac
              subst hik
              simp only [↓reduceIte, true_iff]
              intro j hj hkj
              rw [hget j (by omega)] at hkj
              simp only [show ¬ k.val = j by omega, ↓reduceIte] at hkj
              have := hreach j (by omega) hj hkj
              rw [hcur']; scalar_tac
          · intro j hj hjk hkj
            rw [hget j hj] at hkj
            by_cases hjk' : j < k.val
            · simp only [show ¬ k.val = j by omega, ↓reduceIte] at hkj
              have := hreach j hj hjk' hkj
              have : ranges.val[k.val].1.val ≥ reach.val := by rw [hcur']; scalar_tac
              have : ranges.val[k.val].1.val ≤ ranges.val[k.val].2.val := by rw [hcur']; exact hwfk
              have : ranges.val[k.val].2.val = en.val := by rw [hcur']
              omega
            · have : j = k.val := by scalar_tac
              subst this
              rw [hcur']
          · right
            refine ⟨ k.val, hlt', by scalar_tac, ?_, by rw [hcur'] ⟩
            rw [hget k.val hlt']; simp
        next hlt3 =>
          step as ⟨ nv, hnv ⟩
          step as ⟨ k1, hk1 ⟩
          have hnvl : nv.val.length = ranges.val.length := by simp [hnv, hl]
          have hget : ∀ j (hj : j < ranges.val.length), nv.val[j]'(by omega) =
              if k.val = j then false else cur.val[j]'(by omega) := by
            intro j hj; simp [hnv, List.getElem_set]
          -- some kept range reaches past this one's start
          have hpast : reach.val ≠ 0 := by scalar_tac
          obtain ⟨ w, hw, hwk, hwkept, hwreach ⟩ := hwit.resolve_left hpast
          refine ⟨ ⟨ hnvl, by scalar_tac, ?_, ?_, ?_ ⟩, by scalar_tac ⟩
          · intro i hi hik
            rw [hget i hi]
            by_cases hik' : i < k.val
            · simp only [show ¬ k.val = i by omega, ↓reduceIte]
              rw [hchar i hi hik']
              constructor
              · intro h j hj hkj
                rw [hget j (by omega)] at hkj
                simp only [show ¬ k.val = j by omega, ↓reduceIte] at hkj
                exact h j hj hkj
              · intro h j hj hkj
                apply h j hj
                rw [hget j (by omega)]
                simp only [show ¬ k.val = j by omega, ↓reduceIte]
                exact hkj
            · have hik : i = k.val := by scalar_tac
              subst hik
              simp only [↓reduceIte, Bool.false_eq_true, false_iff, not_forall]
              refine ⟨ w, hwk, ?_, ?_ ⟩
              · rw [hget w (by omega)]
                simp only [show ¬ k.val = w by omega, ↓reduceIte]
                exact hwkept
              · rw [hcur']; scalar_tac
          · intro j hj hjk hkj
            rw [hget j hj] at hkj
            by_cases hjk' : j < k.val
            · simp only [show ¬ k.val = j by omega, ↓reduceIte] at hkj
              exact hreach j hj hjk' hkj
            · have : j = k.val := by scalar_tac
              subst this
              simp at hkj
          · right
            refine ⟨ w, hw, by scalar_tac, ?_, hwreach ⟩
            rw [hget w hw]
            simp only [show ¬ k.val = w by omega, ↓reduceIte]
            exact hwkept
      next hge2 =>
        exfalso; scalar_tac
    next hge =>
      simp only [WP.spec_ok]
      exact ⟨ hl, fun i hi => hchar i hi (by scalar_tac) ⟩
  · exact ⟨ hlen, by simp, fun i _ hi => by simp at hi, fun j _ hj => by simp at hj, Or.inl rfl ⟩

/-- A greedy verdict over in-bounds ranges keeps a batch `validate_batch`
accepts: its non-empty kept ranges are in bounds, strictly ascending and
disjoint. -/
theorem greedy_batchOk (len : Nat) (rs : List (Usize × Usize)) (keep : List Bool)
    (hg : Greedy rs keep) (hin : ∀ r ∈ rs, InBounds len r) :
    BatchOk len (keptNonEmpty rs keep) := by
  obtain ⟨ hlen, hchar ⟩ := hg
  have hpair : List.Pairwise (fun p q => (p.2 = true ∧ p.1.1.val < p.1.2.val) →
      (q.2 = true ∧ q.1.1.val < q.1.2.val) → Disjoint p.1 q.1) (rs.zip keep) := by
    rw [List.pairwise_iff_getElem]
    intro i j hi hj hij hp hq
    have hzl : (rs.zip keep).length = rs.length := by simp [List.length_zip, hlen]
    simp only [List.getElem_zip] at hp hq ⊢
    have hjk := (hchar j (by omega)).mp hq.1 i hij hp.1
    exact ⟨ by omega, hjk ⟩
  have hf := hpair.filter (fun p => p.2 && decide (p.1.1.val < p.1.2.val))
  have hd : List.Pairwise Disjoint (keptNonEmpty rs keep) := by
    unfold keptNonEmpty
    rw [List.pairwise_map]
    refine List.Pairwise.imp_of_mem (fun {p q} hp hq h => ?_) hf
    simp only [List.mem_filter, Bool.and_eq_true, decide_eq_true_eq] at hp hq
    exact h hp.2 hq.2
  refine ⟨ fun r hr => ?_, fun i hi => ?_ ⟩
  · unfold keptNonEmpty at hr
    simp only [List.mem_map, List.mem_filter] at hr
    obtain ⟨ ⟨ r', k ⟩, ⟨ hmem, _ ⟩, rfl ⟩ := hr
    exact hin _ (List.of_mem_zip hmem).1
  · exact List.pairwise_iff_getElem.mp hd i (i + 1) (by omega) hi (by omega)

/-- The multicursor plan filter keeps a range exactly when it starts at or
after the end of every range kept before it — so the first range is always
kept, and a dropped range overlaps a kept one — and the kept non-empty ranges
form a batch `validate_batch` accepts. -/
theorem keep_disjoint_spec (len : Nat) (ranges : Slice (Usize × Usize)) (keep : Slice Bool)
    (hlen : keep.val.length = ranges.val.length)
    (hin : ∀ r ∈ ranges.val, InBounds len r) :
    editmap.keep_disjoint ranges keep ⦃ out =>
      Greedy ranges.val out.val ∧ BatchOk len (keptNonEmpty ranges.val out.val) ⦄ := by
  unfold editmap.keep_disjoint
  apply WP.spec_mono (keep_disjoint_loop_spec ranges keep hlen (fun r hr => (hin r hr).1))
  intro out hg
  exact ⟨ hg, greedy_batchOk len ranges.val out.val hg hin ⟩

end Strop.Kernel
