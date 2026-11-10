import Extracted.StropKernel

/-!
# Edit geometry (plans/0086 ring 0)

Theorems over the Aeneas translation of `strop_kernel::editmap`
(`Extracted/StropKernel.lean`):

- `map_position` is right-affinity anchor mapping (`map_position_spec`): an
  anchor before the edit stays, one inside the replaced range collapses to its
  start, one at/after the old end shifts by the delta (saturating at the
  platform bound). The mapping is monotone (`mapSpec_monotone`) and stays
  inside the post-edit buffer (`mapSpec_in_bounds`).
- `validate_batch` accepts exactly the batches production can apply back to
  front (`validate_batch_spec`): strictly ascending starts, each edit in
  bounds, no edit reaching past the next one's start. Each refusal names a
  real defect of the input.
- `map_through_edits` folds `map_position` over a journal in application
  order (`map_through_edits_spec`) and is monotone
  (`map_through_edits_monotone`). Composition
  (`map_through_edits_composition`): over the journal of a batch applied
  back to front — `apply_prepared` publishes the last edit first, and
  `editor/transact.rs`, collection rebuilds and snippet stops map through
  that journal with this function — it equals classifying the anchor once
  against the original coordinates and adding the deltas of the edits wholly
  before it. `consecutive_of_batchOk` connects `validate_batch`'s acceptance
  to the composition's premise.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-! ## One anchor through one edit -/

/-- Right-affinity mapping of `p` through an edit that replaced `s..oe` with
text ending at `ne`, saturating at the platform bound like the shipped code. -/
def mapSpec (p s oe ne : Nat) : Nat :=
  if p < s then p else if oe ≤ p then min (ne + (p - oe)) Usize.max else s

theorem saturating_add_val (x y : Usize) :
    (core.num.Usize.saturating_add x y).val = min (x.val + y.val) Usize.max := by
  simp only [core.num.Usize.saturating_add, UScalar.saturating_add, UScalar.val,
    BitVec.toNat_ofNat, UScalar.max_USize_eq]
  have hmax : Usize.max < 2 ^ UScalarTy.Usize.numBits := by
    simp [Usize.max, Usize.numBits]
  rw [Nat.mod_eq_of_lt (by omega), Nat.min_comm]

/-- `map_position` computes `mapSpec`, for every input. -/
theorem map_position_spec (p s oe ne : Usize) :
    editmap.map_position p s oe ne ⦃ r => r.val = mapSpec p.val s.val oe.val ne.val ⦄ := by
  unfold editmap.map_position
  split_ifs with h1 h2
  · simp only [WP.spec_ok, mapSpec]
    simp only [show p.val < s.val by scalar_tac, ↓reduceIte]
  · step as ⟨ d, hd ⟩
    simp only [saturating_add_val, hd, mapSpec]
    simp only [show ¬ p.val < s.val by scalar_tac, show oe.val ≤ p.val by scalar_tac, ↓reduceIte]
  · simp only [WP.spec_ok, mapSpec]
    simp only [show ¬ p.val < s.val by scalar_tac, show ¬ oe.val ≤ p.val by scalar_tac, ↓reduceIte]

/-- Anchors keep their relative order through one edit. -/
theorem mapSpec_monotone {a b s oe ne : Nat} (hsn : s ≤ ne)
    (hsmax : s ≤ Usize.max) (hab : a ≤ b) :
    mapSpec a s oe ne ≤ mapSpec b s oe ne := by
  unfold mapSpec
  split_ifs <;> omega

/-- An anchor inside a buffer of `len` bytes stays inside the edited buffer,
whose length is `len - oe + ne`. -/
theorem mapSpec_in_bounds {p s oe ne len : Nat} (hsn : s ≤ ne)
    (hoe : oe ≤ len) (hp : p ≤ len) :
    mapSpec p s oe ne ≤ len - oe + ne := by
  unfold mapSpec
  split_ifs <;> omega

/-! ## Batch admission -/

/-- An edit `(start, end)` that addresses a buffer of `len` bytes. -/
def InBounds (len : Nat) (e : Usize × Usize) : Prop :=
  e.1.val ≤ e.2.val ∧ e.2.val ≤ len

/-- `a` lies strictly before `b`: a later start, and `a` ends by `b`'s start. -/
def Disjoint (a b : Usize × Usize) : Prop :=
  a.1.val < b.1.val ∧ a.2.val ≤ b.1.val

/-- A batch production can apply back to front. -/
def BatchOk (len : Nat) (es : List (Usize × Usize)) : Prop :=
  (∀ e ∈ es, InBounds len e) ∧ (∀ i (h : i + 1 < es.length), Disjoint es[i] es[i + 1])

/-- What each outcome of `validate_batch` means. -/
def Verdict (len : Nat) (es : List (Usize × Usize)) :
    core.result.Result Unit editmap.BatchRefusal → Prop
  | .Ok () => BatchOk len es
  | .Err .OutOfBounds => ∃ e ∈ es, ¬ InBounds len e
  | .Err .Unsorted => ∃ i, ∃ h : i + 1 < es.length, es[i + 1].1.val < es[i].1.val
  | .Err .Overlap => ∃ i, ∃ h : i + 1 < es.length,
      es[i].1.val ≤ es[i + 1].1.val ∧ ¬ Disjoint es[i] es[i + 1]

/-- Every refusal contradicts `BatchOk`: a refused batch was not applicable. -/
theorem verdict_err_not_ok (len : Nat) (es : List (Usize × Usize)) (refusal : editmap.BatchRefusal)
    (h : Verdict len es (.Err refusal)) : ¬ BatchOk len es := by
  intro ⟨ hin, hdis ⟩
  cases refusal with
  | OutOfBounds =>
    obtain ⟨ e, he, hne ⟩ := h
    exact hne (hin e he)
  | Unsorted =>
    obtain ⟨ i, hi, hlt ⟩ := h
    have := (hdis i hi).1
    omega
  | Overlap =>
    obtain ⟨ i, hi, _, hnd ⟩ := h
    exact hnd (hdis i hi)

/-- The loop from position `k`, every earlier position already checked. -/
theorem validate_batch_loop_spec (len : Usize) (edits : Slice (Usize × Usize)) (k : Usize)
    (hk : k.val ≤ edits.val.length)
    (hin : ∀ j (h : j < edits.val.length), j < k.val → InBounds len.val edits.val[j])
    (hdis : ∀ j (h : j + 1 < edits.val.length), j + 1 < k.val →
      Disjoint edits.val[j] edits.val[j + 1]) :
    editmap.validate_batch_loop len edits k ⦃ r => Verdict len.val edits.val r ⦄ := by
  unfold editmap.validate_batch_loop
  apply loop.spec_decr_nat (fun k : Usize => edits.val.length - k.val)
    (fun k : Usize => k.val ≤ edits.val.length ∧
      (∀ j (h : j < edits.val.length), j < k.val → InBounds len.val edits.val[j]) ∧
      (∀ j (h : j + 1 < edits.val.length), j + 1 < k.val →
        Disjoint edits.val[j] edits.val[j + 1]))
  · intro k ⟨ hk, hin, hdis ⟩
    unfold editmap.validate_batch_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : k.val < edits.val.length := by scalar_tac
      step as ⟨ s, e, hcur ⟩
      have hcur' : edits.val[k.val] = (s, e) := hcur.symm
      have hmem : (s, e) ∈ edits.val := by rw [← hcur']; exact List.getElem_mem _
      split
      next hse =>
        simp only [WP.spec_ok]
        exact ⟨ (s, e), hmem, fun h => by unfold InBounds at h; scalar_tac ⟩
      next hse =>
        split
        next hlen =>
          simp only [WP.spec_ok]
          exact ⟨ (s, e), hmem, fun h => by unfold InBounds at h; scalar_tac ⟩
        next hlen =>
          have hinb : InBounds len.val (s, e) := by
            simp only [InBounds]; constructor <;> scalar_tac
          split
          next hpos =>
            step as ⟨ km, hkm ⟩
            have hkm' : km.val = k.val - 1 := by scalar_tac
            step as ⟨ ps, pe, hprev ⟩
            have hidx : k.val - 1 + 1 = k.val := by scalar_tac
            have hprev' : edits.val[k.val - 1]'(by scalar_tac) = (ps, pe) := by
              rw [hprev]; simp only [hkm']
            have hcur'' : edits.val[k.val - 1 + 1]'(by scalar_tac) = (s, e) := by
              simp only [hidx]; exact hcur'
            split
            next hun =>
              simp only [WP.spec_ok]
              refine ⟨ k.val - 1, by scalar_tac, ?_ ⟩
              rw [hcur'', hprev']
              scalar_tac
            next hun =>
              split
              next hsame =>
                simp only [WP.spec_ok]
                refine ⟨ k.val - 1, by scalar_tac, ?_ ⟩
                rw [hcur'', hprev']
                simp only [Disjoint]
                constructor
                · scalar_tac
                · intro h; scalar_tac
              next hsame =>
                split
                next hreach =>
                  simp only [WP.spec_ok]
                  refine ⟨ k.val - 1, by scalar_tac, ?_ ⟩
                  rw [hcur'', hprev']
                  simp only [Disjoint]
                  constructor
                  · scalar_tac
                  · intro h; scalar_tac
                next hreach =>
                  step as ⟨ k1, hk1 ⟩
                  refine ⟨ by scalar_tac, ?_, ?_, by scalar_tac ⟩
                  · intro j hj hjk
                    by_cases hjlt : j < k.val
                    · exact hin j hj hjlt
                    · have : j = k.val := by scalar_tac
                      subst this
                      rw [hcur']; exact hinb
                  · intro j hj hjk
                    by_cases hjlt : j + 1 < k.val
                    · exact hdis j hj hjlt
                    · have hjeq : j = k.val - 1 := by scalar_tac
                      subst hjeq
                      rw [hcur'', hprev']
                      simp only [Disjoint]
                      constructor <;> scalar_tac
          next hpos =>
            step as ⟨ k1, hk1 ⟩
            have hk0 : k.val = 0 := by scalar_tac
            refine ⟨ by scalar_tac, ?_, ?_, by scalar_tac ⟩
            · intro j hj hjk
              have : j = k.val := by scalar_tac
              subst this
              rw [hcur']; exact hinb
            · intro j hj hjk
              scalar_tac
    next hlt =>
      have hend : k.val = edits.val.length := by scalar_tac
      simp only [WP.spec_ok]
      show BatchOk len.val edits.val
      exact ⟨ fun e he => by
          obtain ⟨ j, hj, rfl ⟩ := List.getElem_of_mem he
          exact hin j hj (by omega),
        fun j hj => hdis j hj (by omega) ⟩
  · exact ⟨ hk, hin, hdis ⟩

/-- `validate_batch` accepts exactly the applicable batches, and every refusal
names a real defect of the input. -/
theorem validate_batch_spec (len : Usize) (edits : Slice (Usize × Usize)) :
    editmap.validate_batch len edits ⦃ r =>
      Verdict len.val edits.val r ∧ (r = .Ok () ↔ BatchOk len.val edits.val) ⦄ := by
  unfold editmap.validate_batch
  apply WP.spec_mono (validate_batch_loop_spec len edits 0#usize (by simp)
    (fun j _ hj => by simp at hj) (fun j _ hj => by simp at hj))
  intro r hr
  refine ⟨ hr, ?_ ⟩
  cases r with
  | Ok u => cases u; simpa [Verdict] using hr
  | Err refusal =>
    simp only [reduceCtorEq, false_iff]
    exact verdict_err_not_ok _ _ refusal hr

/-! ## Composition over a batch -/

/-- One edit with its replacement extent: `(start, old_end, new_end)`. -/
abbrev Extent := Nat × Nat × Nat

/-- `mapSpec` without the platform bound (the bound never binds for anchors
inside a buffer). -/
def mapNat (p : Nat) (e : Extent) : Nat :=
  if p < e.1 then p else if e.2.1 ≤ p then e.2.2 + (p - e.2.1) else e.1

theorem mapSpec_eq_mapNat {p s oe ne : Nat} (h : ne + (p - oe) ≤ Usize.max) :
    mapSpec p s oe ne = mapNat p (s, oe, ne) := by
  unfold mapSpec mapNat
  simp only
  split_ifs <;> omega

/-- The reverse-order fold over a batch sorted by start, in `Nat` (no
platform bound): the proof device `map_through_edits_composition` reduces
the extracted loop to. -/
def batchMap (p : Nat) (es : List Extent) : Nat :=
  es.foldr (fun e acc => mapNat acc e) p

/-- Classify the anchor once against original coordinates: the deltas of the
edits wholly before it accumulate; an anchor inside an edit lands on its
(shifted) start. -/
def directMap (p : Nat) : List Extent → Int
  | [] => p
  | (s, oe, ne) :: rest =>
    if p < s then p else if p < oe then s else ((ne : Int) - oe) + directMap p rest

/-- The batch geometry `validate_batch` accepts, with replacement extents:
`start ≤ old_end`, `start ≤ new_end` (by construction), and each edit ends by
the next one's start. -/
def Chain3 : List Extent → Prop
  | [] => True
  | e :: rest => e.1 ≤ e.2.1 ∧ e.1 ≤ e.2.2 ∧ (∀ f ∈ rest.head?, e.2.1 ≤ f.1) ∧ Chain3 rest

theorem chain3_later {e : Extent} {rest : List Extent} (h : Chain3 (e :: rest)) :
    ∀ x ∈ rest, e.2.1 ≤ x.1 := by
  induction rest generalizing e with
  | nil => simp
  | cons f rest ih =>
    obtain ⟨ _, _, hhead, hrest ⟩ := h
    have hef : e.2.1 ≤ f.1 := hhead f (by simp)
    intro x hx
    rcases List.mem_cons.mp hx with rfl | hx
    · exact hef
    · have hfx := ih hrest x hx
      have := hrest.1
      omega

/-- Anchors strictly before every edit are untouched. -/
theorem batchMap_below {p : Nat} {es : List Extent} (h : ∀ x ∈ es, p < x.1) :
    batchMap p es = p := by
  induction es with
  | nil => rfl
  | cons e rest ih =>
    simp only [batchMap, List.foldr_cons] at *
    rw [ih (fun x hx => h x (by simp [hx]))]
    simp [mapNat, h e (by simp)]

/-- Mapping never moves an anchor below a bound that all edits start at or
after. -/
theorem batchMap_lower {p b : Nat} {es : List Extent} (hchain : Chain3 es)
    (hstarts : ∀ x ∈ es, b ≤ x.1) (hp : b ≤ p) : b ≤ batchMap p es := by
  induction es with
  | nil => simpa [batchMap]
  | cons e rest ih =>
    obtain ⟨ hso, hsn, _, hrest ⟩ := hchain
    have hq := ih hrest (fun x hx => hstarts x (by simp [hx]))
    have hbe := hstarts e (by simp)
    show b ≤ mapNat (batchMap p rest) e
    generalize batchMap p rest = q at hq ⊢
    unfold mapNat
    split_ifs <;> omega

/-- The reverse-order fold equals the direct map. -/
theorem batchMap_eq_directMap (p : Nat) (es : List Extent) (hchain : Chain3 es) :
    (batchMap p es : Int) = directMap p es := by
  induction es with
  | nil => simp [batchMap, directMap]
  | cons e rest ih =>
    obtain ⟨ s, oe, ne ⟩ := e
    have hlater := chain3_later hchain
    obtain ⟨ hso, hsn, _, hrest ⟩ := hchain
    simp only at hso hsn hlater
    show ((mapNat (batchMap p rest) (s, oe, ne) : Nat) : Int) = directMap p ((s, oe, ne) :: rest)
    simp only [directMap]
    split_ifs with hps hpoe
    · rw [batchMap_below (fun x hx => by have := hlater x hx; omega)]
      simp [mapNat, hps]
    · rw [batchMap_below (fun x hx => by have := hlater x hx; omega)]
      simp [mapNat, hps, show ¬ oe ≤ p by omega]
    · have hq : oe ≤ batchMap p rest := batchMap_lower hrest hlater (by omega)
      have hih := ih hrest
      generalize batchMap p rest = q at hq hih ⊢
      unfold mapNat
      simp only
      simp only [show ¬ q < s by omega, hq, ↓reduceIte]
      omega

/-- Bridge: a batch `validate_batch` accepts, with replacement ends no earlier
than their starts, has the composition's geometry. -/
theorem consecutive_of_batchOk (len : Nat) :
    ∀ (es : List (Usize × Usize)) (ends : List Nat), ends.length = es.length →
      (∀ i (h₁ : i < es.length) (h₂ : i < ends.length), es[i].1.val ≤ ends[i]) →
      BatchOk len es →
      Chain3 (List.zipWith (fun e n => (e.1.val, e.2.val, n)) es ends)
  | [], _, _, _, _ => by simp [Chain3]
  | _ :: _, [], hlen, _, _ => by simp at hlen
  | e :: rest, n :: ns, hlen, hends, ⟨ hin, hdis ⟩ => by
    simp only [List.zipWith_cons_cons, Chain3]
    have he := hin e (by simp)
    refine ⟨ he.1, hends 0 (by simp) (by simp), ?_, ?_ ⟩
    · intro f hf
      cases rest with
      | nil => simp at hf
      | cons g rest =>
        cases ns with
        | nil => simp at hlen
        | cons m ns =>
          simp only [List.zipWith_cons_cons, List.head?_cons, Option.mem_def,
            Option.some.injEq] at hf
          subst hf
          exact (hdis 0 (by simp)).2
    · apply consecutive_of_batchOk len rest ns (by simpa using hlen)
      · intro i h₁ h₂
        exact hends (i + 1) (by simpa using h₁) (by simpa using h₂)
      · exact ⟨ fun x hx => hin x (by simp [hx]),
          fun i hi => hdis (i + 1) (by simp; omega) ⟩

/-! ## Mapping through a journal -/

/-- One journaled edit `(start, old_end, new_end)` as an extent. -/
def extent (e : Usize × Usize × Usize) : Extent := (e.1.val, e.2.1.val, e.2.2.val)

/-- The journal fold `map_through_edits` computes: `mapSpec` through each
edit in application order. -/
def journalSpec (p : Nat) (es : List (Usize × Usize × Usize)) : Nat :=
  es.foldl (fun q e => mapSpec q e.1.val e.2.1.val e.2.2.val) p

/-- The loop from position `k` folds the rest of the journal. -/
theorem map_through_edits_loop_spec (edits : Slice (Usize × Usize × Usize)) (mapped k : Usize)
    (hk : k.val ≤ edits.val.length) :
    editmap.map_through_edits_loop edits mapped k ⦃ r =>
      r.val = journalSpec mapped.val (edits.val.drop k.val) ⦄ := by
  unfold editmap.map_through_edits_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize => edits.val.length - s.2.val)
    (fun s : Usize × Usize => s.2.val ≤ edits.val.length ∧
      journalSpec s.1.val (edits.val.drop s.2.val) = journalSpec mapped.val (edits.val.drop k.val))
  · intro ⟨ m, j ⟩ ⟨ hj, hinv ⟩
    simp only at hj hinv
    unfold editmap.map_through_edits_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < edits.val.length := by scalar_tac
      step as ⟨ s, oe, ne, hcur ⟩
      step with map_position_spec as ⟨ m1, hm1 ⟩
      step as ⟨ j1, hj1 ⟩
      and_intros
      all_goals try scalar_tac
      rw [← hinv, hj1, List.drop_eq_getElem_cons hlt']
      simp only [journalSpec, List.foldl_cons, ← hcur, hm1]
    next hge =>
      have hend : j.val = edits.val.length := by scalar_tac
      simp only [WP.spec_ok]
      rw [← hinv, hend, List.drop_length]
      rfl
  · exact ⟨ hk, rfl ⟩

/-- `map_through_edits` folds `map_position` over the journal in application
order, for every input. -/
theorem map_through_edits_spec (p : Usize) (edits : Slice (Usize × Usize × Usize)) :
    editmap.map_through_edits p edits ⦃ r => r.val = journalSpec p.val edits.val ⦄ := by
  unfold editmap.map_through_edits
  apply WP.spec_mono (map_through_edits_loop_spec edits p 0#usize (by simp))
  intro r hr
  simpa using hr

theorem journalSpec_monotone {a b : Nat} (es : List (Usize × Usize × Usize))
    (hsn : ∀ e ∈ es, e.1.val ≤ e.2.2.val) (hab : a ≤ b) :
    journalSpec a es ≤ journalSpec b es := by
  induction es generalizing a b with
  | nil => simpa [journalSpec]
  | cons e rest ih =>
    simp only [journalSpec, List.foldl_cons] at *
    apply ih (fun x hx => hsn x (by simp [hx]))
    exact mapSpec_monotone (hsn e (by simp)) (by scalar_tac) hab

/-- Anchors keep their relative order through a whole journal. -/
theorem map_through_edits_monotone (a b : Usize) (edits : Slice (Usize × Usize × Usize))
    (hsn : ∀ e ∈ edits.val, e.1.val ≤ e.2.2.val) (hab : a.val ≤ b.val) :
    editmap.map_through_edits a edits ⦃ ra =>
      editmap.map_through_edits b edits ⦃ rb => ra.val ≤ rb.val ⦄ ⦄ := by
  apply WP.spec_mono (map_through_edits_spec a edits)
  intro ra hra
  apply WP.spec_mono (map_through_edits_spec b edits)
  intro rb hrb
  rw [hra, hrb]
  exact journalSpec_monotone edits.val hsn hab

/-- Below the platform bound the journal fold is the `Nat` fold. -/
theorem journalSpec_eq_foldl (p : Nat) (es : List (Usize × Usize × Usize))
    (hbound : p + (es.map (fun e => e.2.2.val)).sum ≤ Usize.max) :
    journalSpec p es = (es.map extent).foldl mapNat p := by
  induction es generalizing p with
  | nil => rfl
  | cons e rest ih =>
    obtain ⟨ s, oe, ne ⟩ := e
    simp only [List.map_cons, List.sum_cons] at hbound
    simp only [journalSpec, List.foldl_cons, List.map_cons, extent] at *
    rw [mapSpec_eq_mapNat (by omega)]
    apply ih
    have : mapNat p (s.val, oe.val, ne.val) ≤ p + ne.val := by
      unfold mapNat; simp only; split_ifs <;> omega
    omega

/-- Composition over the extracted fold: through the journal of a batch
applied back to front (`journal` is the batch reversed), an anchor lands
where classifying it once against the original coordinates puts it. The
bound keeps every intermediate offset below the platform maximum, as byte
offsets into a rope always are. -/
theorem map_through_edits_composition (p : Usize) (journal : Slice (Usize × Usize × Usize))
    (hchain : Chain3 (journal.val.reverse.map extent))
    (hbound : p.val + (journal.val.map (fun e => e.2.2.val)).sum ≤ Usize.max) :
    editmap.map_through_edits p journal ⦃ r =>
      (r.val : Int) = directMap p.val (journal.val.reverse.map extent) ⦄ := by
  apply WP.spec_mono (map_through_edits_spec p journal)
  intro r hr
  rw [hr, journalSpec_eq_foldl _ _ hbound, ← batchMap_eq_directMap _ _ hchain]
  simp only [batchMap, List.map_reverse, List.foldr_reverse]

end Strop.Kernel
