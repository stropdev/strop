import Extracted.StropKernel

/-!
# The cache core (plans/0099)

Theorems over the Aeneas translation of `strop_kernel::cache`
(`Extracted/StropKernel.lean`). An entry is named by its subject, its producer
and the version of the subject it was computed for.

- `find_spec`: a hit is the first live slot whose subject, producer and
  version all equal the ones asked for; a miss means no live slot matches. An
  entry for another version is never served.
- `retire_spec`, `find_after_retire`: an invalidation kills exactly the slots
  it covers, and a lookup for a retired subject or producer misses, whatever
  the version.
- `place_spec`, `total_cost_le`: a value over the limit is refused and nothing
  changes; an accepted value is in the returned slot under its exact key; every
  other live slot is unchanged and belongs to another pair; the table keeps its
  length and its cost bound.
- `place_evicts_oldest`: `place` overwrites a live entry of another pair only
  when no slot is dead, and then the least recently used one.
- `find_after_place`: the entry just placed is found under its key, and the
  same pair at any other version misses.
- `touch_spec`: a use changes one slot's tick and nothing else.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-! ## Lookup -/

/-- `x` is the live entry for exactly this subject, producer and version. -/
def Holds (x : cache.Slot) (s p v : U64) : Prop :=
  x.live = true ∧ x.subject = s ∧ x.producer = p ∧ x.version = v

@[step]
theorem holds_spec (x : cache.Slot) (s p v : U64) :
    cache.holds x s p v ⦃ b => b = true ↔ Holds x s p v ⦄ := by
  unfold cache.holds Holds
  split_ifs <;> simp_all

/-- What `find` answers: the first slot holding the key, or that none does. -/
def Found (l : List cache.Slot) (s p v : U64) : Option Usize → Prop
  | none => ∀ j (h : j < l.length), ¬ Holds l[j] s p v
  | some i => ∃ h : i.val < l.length, Holds l[i.val] s p v ∧
      ∀ j (hj : j < i.val), ¬ Holds (l[j]'(by omega)) s p v

theorem find_loop_spec (slots : Slice cache.Slot) (s p v : U64) (k : Usize)
    (hk : k.val ≤ slots.val.length)
    (hbefore : ∀ j (h : j < slots.val.length), j < k.val → ¬ Holds slots.val[j] s p v) :
    cache.find_loop slots s p v k ⦃ r => Found slots.val s p v r ⦄ := by
  unfold cache.find_loop
  apply loop.spec_decr_nat (fun k : Usize => slots.val.length - k.val)
    (fun k : Usize => k.val ≤ slots.val.length ∧
      ∀ j (h : j < slots.val.length), j < k.val → ¬ Holds slots.val[j] s p v)
  · intro k ⟨ hk, hbefore ⟩
    unfold cache.find_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : k.val < slots.val.length := by scalar_tac
      step as ⟨ x, hx ⟩
      step as ⟨ b, hb ⟩
      split
      next hyes =>
        simp only [WP.spec_ok]
        refine ⟨ hlt', ?_, fun j hj => hbefore j (by omega) hj ⟩
        rw [← hx]; exact hb.mp hyes
      next hno =>
        step as ⟨ k1, hk1 ⟩
        refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
        intro j hj hjk
        by_cases hjlt : j < k.val
        · exact hbefore j hj hjlt
        · have : j = k.val := by scalar_tac
          subst this
          rw [← hx]; intro h; exact hno (hb.mpr h)
    next hlt =>
      simp only [WP.spec_ok]
      intro j hj
      exact hbefore j hj (by scalar_tac)
  · exact ⟨ hk, hbefore ⟩

/-- `find` returns the first live slot with exactly this subject, producer and
version, and `none` exactly when there is none. -/
theorem find_spec (slots : Slice cache.Slot) (s p v : U64) :
    cache.find slots s p v ⦃ r => Found slots.val s p v r ⦄ := by
  unfold cache.find
  exact find_loop_spec slots s p v 0#usize (by simp) (fun j _ hj => by simp at hj)

/-! ## Invalidation -/

/-- An invalidation by subject and/or producer covers `x`; with neither it
covers every slot. -/
def Selects (x : cache.Slot) (s p : U64) (bs bp : Bool) : Prop :=
  (bs = true → x.subject = s) ∧ (bp = true → x.producer = p)

@[step]
theorem selects_spec (x : cache.Slot) (s p : U64) (bs bp : Bool) :
    cache.selects x s p bs bp ⦃ b => b = true ↔ Selects x s p bs bp ⦄ := by
  unfold cache.selects Selects
  split_ifs <;> simp_all

/-- Slot `j` after an invalidation: dead when covered, untouched otherwise. -/
def RetiredAt (old new : cache.Slot) (s p : U64) (bs bp : Bool) : Prop :=
  (Selects old s p bs bp → new = { old with live := false }) ∧
  (¬ Selects old s p bs bp → new = old)

/-- The table after an invalidation. -/
def Retired (old new : List cache.Slot) (s p : U64) (bs bp : Bool) : Prop :=
  ∃ hlen : new.length = old.length,
    ∀ j (h : j < old.length), RetiredAt old[j] (new[j]'(by omega)) s p bs bp

theorem retire_loop_spec (slots : Slice cache.Slot) (s p : U64) (bs bp : Bool) :
    cache.retire_loop slots s p bs bp 0#usize ⦃ r => Retired slots.val r.val s p bs bp ⦄ := by
  unfold cache.retire_loop
  apply loop.spec_decr_nat
    (fun (st : Slice cache.Slot × Usize) => slots.val.length - st.2.val)
    (fun (st : Slice cache.Slot × Usize) =>
      ∃ hlen : st.1.val.length = slots.val.length, st.2.val ≤ slots.val.length ∧
        ∀ j (h : j < slots.val.length),
          (j < st.2.val → RetiredAt slots.val[j] (st.1.val[j]'(by omega)) s p bs bp) ∧
          (st.2.val ≤ j → st.1.val[j]'(by omega) = slots.val[j]))
  · intro ⟨ cur, k ⟩ ⟨ hlen, hk, hinv ⟩
    simp only at hlen hk hinv
    unfold cache.retire_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : k.val < cur.val.length := by scalar_tac
      have hlto : k.val < slots.val.length := by omega
      step as ⟨ x, hx ⟩
      have hxold : x = slots.val[k.val] := by rw [hx]; exact (hinv k.val hlto).2 (Nat.le_refl _)
      step as ⟨ b, hb ⟩
      split
      next hyes =>
        step as ⟨ nv, hnv ⟩
        step as ⟨ k1, hk1 ⟩
        have hnvlen : nv.val.length = slots.val.length := by simp [hnv, hlen]
        refine ⟨ ⟨ hnvlen, by scalar_tac, ?_ ⟩, by scalar_tac ⟩
        intro j hj
        have hget : nv.val[j]'(by omega) =
            if k.val = j then { x with live := false } else cur.val[j]'(by omega) := by
          simp [hnv, List.getElem_set]
        constructor
        · intro hjk
          by_cases hjlt : j < k.val
          · rw [hget]; simp only [show ¬ k.val = j by omega, ↓reduceIte]; exact (hinv j hj).1 hjlt
          · have hjeq : k.val = j := by scalar_tac
            subst hjeq
            rw [hget]; simp only [↓reduceIte]; rw [hxold]
            exact ⟨ fun _ => rfl, fun hn => absurd (hxold ▸ hb.mp hyes) hn ⟩
        · intro hjk
          rw [hget]; simp only [show ¬ k.val = j by scalar_tac, ↓reduceIte]
          exact (hinv j hj).2 (by scalar_tac)
      next hno =>
        step as ⟨ k1, hk1 ⟩
        refine ⟨ ⟨ hlen, by scalar_tac, ?_ ⟩, by scalar_tac ⟩
        intro j hj
        constructor
        · intro hjk
          by_cases hjlt : j < k.val
          · exact (hinv j hj).1 hjlt
          · have hjeq : j = k.val := by scalar_tac
            subst hjeq
            have hsame := (hinv k.val hj).2 (Nat.le_refl _)
            refine ⟨ fun hsel => absurd (hb.mpr (hxold ▸ hsel)) hno, fun _ => hsame ⟩
        · intro hjk
          exact (hinv j hj).2 (by scalar_tac)
    next hlt =>
      simp only [WP.spec_ok]
      exact ⟨ hlen, fun j hj => (hinv j hj).1 (by scalar_tac) ⟩
  · exact ⟨ rfl, by simp, fun j hj => ⟨ fun h => by simp at h, fun _ => rfl ⟩ ⟩

/-- `retire` kills every slot the invalidation covers and touches no other. -/
@[step]
theorem retire_spec (slots : Slice cache.Slot) (s p : U64) (bs bp : Bool) :
    cache.retire slots s p bs bp ⦃ r => Retired slots.val r.val s p bs bp ⦄ := by
  unfold cache.retire
  exact retire_loop_spec slots s p bs bp

/-- No slot of a retired table holds an entry the invalidation covered. -/
theorem retired_not_holds {old new : List cache.Slot} {s p : U64} {bs bp : Bool}
    (h : Retired old new s p bs bp) (s' p' v : U64)
    (hcover : (bs = true → s' = s) ∧ (bp = true → p' = p)) :
    ∀ j (hj : j < new.length), ¬ Holds new[j] s' p' v := by
  obtain ⟨ hlen, hall ⟩ := h
  intro j hj hholds
  obtain ⟨ hlive, hsub, hprod, _ ⟩ := hholds
  have hat := hall j (by omega)
  by_cases hsel : Selects old[j] s p bs bp
  · rw [hat.1 hsel] at hlive
    simp at hlive
  · apply hsel
    rw [hat.2 hsel] at hsub hprod
    exact ⟨ fun hb => by rw [hsub]; exact hcover.1 hb, fun hb => by rw [hprod]; exact hcover.2 hb ⟩

/-- Invalidate, then look up: a miss, whatever the version. Retiring by
subject covers every producer; retiring by producer covers every subject. -/
theorem find_after_retire (slots : Slice cache.Slot) (s p : U64) (bs bp : Bool)
    (s' p' v : U64) (hcover : (bs = true → s' = s) ∧ (bp = true → p' = p)) :
    (do let retired ← cache.retire slots s p bs bp
        cache.find retired s' p' v) ⦃ r => r = none ⦄ := by
  step as ⟨ retired, hretired ⟩
  apply WP.spec_mono (find_spec retired s' p' v)
  intro r hr
  cases r with
  | none => rfl
  | some i =>
    obtain ⟨ hi, hholds, _ ⟩ := hr
    exact absurd hholds (retired_not_holds hretired s' p' v hcover i.val hi)

/-! ## Use -/

/-- `touch` sets one slot's last-use tick and changes nothing else; an index
past the table changes nothing at all. -/
theorem touch_spec (slots : Slice cache.Slot) (i : Usize) (now : U64) :
    cache.touch slots i now ⦃ r => r.val.length = slots.val.length ∧
      ∀ j (h : j < slots.val.length) (h' : j < r.val.length), r.val[j] =
        if i.val = j then { slots.val[j] with used := now } else slots.val[j] ⦄ := by
  unfold cache.touch
  try dsimp only
  split
  next hlt =>
    have hlt' : i.val < slots.val.length := by scalar_tac
    step as ⟨ x, hx ⟩
    step as ⟨ nv, hnv ⟩
    refine ⟨ by simp [hnv], ?_ ⟩
    intro j hj hj'
    simp only [hnv, Slice.set_val_eq, List.getElem_set, hx]
    split_ifs with he
    · subst he; rfl
    · rfl
  next hlt =>
    simp only [WP.spec_ok]
    refine ⟨ trivial, ?_ ⟩
    intro j hj hj'
    have : ¬ i.val = j := by scalar_tac
    simp [this]

/-! ## Placement -/

/-- What `victim` answers: slot `r` is the first dead slot, or — with no dead
slot — a least recently used one. `r` is the table's length only when the table
is empty. -/
def Victim (l : List cache.Slot) (r : Nat) : Prop :=
  r ≤ l.length ∧ (r = l.length → l.length = 0) ∧
  ∀ h : r < l.length,
    (l[r].live = false ∧ ∀ j (hj : j < r), (l[j]'(by omega)).live = true) ∨
    ((∀ j (hj : j < l.length), l[j].live = true) ∧
      ∀ j (hj : j < l.length), l[r].used.val ≤ l[j].used.val)

theorem victim_loop_spec (slots : Slice cache.Slot) :
    cache.victim_loop slots (Slice.len slots) 0#u64 0#usize ⦃ r => Victim slots.val r.val ⦄ := by
  unfold cache.victim_loop
  apply loop.spec_decr_nat
    (fun (st : Usize × U64 × Usize) => slots.val.length - st.2.2.val)
    (fun (st : Usize × U64 × Usize) =>
      st.2.2.val ≤ slots.val.length ∧ st.1.val ≤ slots.val.length ∧
      (∀ j (hj : j < slots.val.length), j < st.2.2.val → slots.val[j].live = true) ∧
      (st.1.val = slots.val.length → st.2.2.val = 0) ∧
      (∀ hb : st.1.val < slots.val.length, st.1.val < st.2.2.val ∧
        st.2.1 = slots.val[st.1.val].used ∧
        ∀ j (hj : j < slots.val.length), j < st.2.2.val →
          slots.val[st.1.val].used.val ≤ slots.val[j].used.val))
  · intro ⟨ best, oldest, k ⟩ ⟨ hk, hbest, hlive, hnone, hsome ⟩
    simp only at hk hbest hlive hnone hsome
    unfold cache.victim_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : k.val < slots.val.length := by scalar_tac
      step as ⟨ x, hx ⟩
      split
      next hxlive =>
        have hklive : slots.val[k.val].live = true := by rw [← hx]; exact hxlive
        split
        next hfirst =>
          have hbl : best.val = slots.val.length := by scalar_tac
          have hk0 : k.val = 0 := hnone hbl
          step as ⟨ k1, hk1 ⟩
          refine ⟨ by scalar_tac, by scalar_tac, ?_, ?_, ?_, by scalar_tac ⟩
          · intro j hj hjk
            have : j = k.val := by scalar_tac
            subst this; exact hklive
          · intro h; scalar_tac
          · intro hb
            refine ⟨ by scalar_tac, by rw [hx], ?_ ⟩
            intro j hj hjk
            have : j = k.val := by scalar_tac
            subst this; exact Nat.le_refl _
        next hfirst =>
          have hbl : best.val < slots.val.length := by scalar_tac
          obtain ⟨ hbk, hold, hmin ⟩ := hsome hbl
          split
          next holder =>
            step as ⟨ k1, hk1 ⟩
            refine ⟨ by scalar_tac, by scalar_tac, ?_, ?_, ?_, by scalar_tac ⟩
            · intro j hj hjk
              by_cases hjlt : j < k.val
              · exact hlive j hj hjlt
              · have : j = k.val := by scalar_tac
                subst this; exact hklive
            · intro h; scalar_tac
            · intro hb
              refine ⟨ by scalar_tac, by rw [hx], ?_ ⟩
              intro j hj hjk
              have hxu : slots.val[k.val].used.val < slots.val[best.val].used.val := by
                rw [← hx, ← hold]; scalar_tac
              by_cases hjlt : j < k.val
              · have := hmin j hj hjlt; omega
              · have : j = k.val := by scalar_tac
                subst this; exact Nat.le_refl _
          next holder =>
            step as ⟨ k1, hk1 ⟩
            refine ⟨ by scalar_tac, by scalar_tac, ?_, ?_, ?_, by scalar_tac ⟩
            · intro j hj hjk
              by_cases hjlt : j < k.val
              · exact hlive j hj hjlt
              · have : j = k.val := by scalar_tac
                subst this; exact hklive
            · intro h; scalar_tac
            · intro hb
              refine ⟨ by scalar_tac, hold, ?_ ⟩
              intro j hj hjk
              have hxu : slots.val[best.val].used.val ≤ slots.val[k.val].used.val := by
                rw [← hx, ← hold]; scalar_tac
              by_cases hjlt : j < k.val
              · exact hmin j hj hjlt
              · have : j = k.val := by scalar_tac
                subst this; exact hxu
      next hxdead =>
        simp only [WP.spec_ok]
        have hkdead : slots.val[k.val].live = false := by
          rw [← hx]; simpa using hxdead
        refine ⟨ by omega, fun h => by omega, fun _ => Or.inl ⟨ hkdead, ?_ ⟩ ⟩
        intro j hj
        exact hlive j (by omega) hj
    next hlt =>
      have hend : k.val = slots.val.length := by scalar_tac
      simp only [WP.spec_ok]
      refine ⟨ hbest, fun h => by have := hnone h; omega, fun hb => Or.inr ⟨ ?_, ?_ ⟩ ⟩
      · intro j hj; exact hlive j hj (by omega)
      · intro j hj; exact (hsome hb).2.2 j hj (by omega)
  · refine ⟨ by simp, by simp, fun j _ h => by simp at h, fun _ => by simp, fun hb => ?_ ⟩
    simp at hb

/-- `victim` picks the first dead slot, or with none dead the least recently
used one. -/
@[step]
theorem victim_spec (slots : Slice cache.Slot) :
    cache.victim slots ⦃ r => Victim slots.val r.val ⦄ := by
  unfold cache.victim
  exact victim_loop_spec slots

/-- The table after `place` stored an entry in slot `i`: the pair's previous
entry was retired (`mid`), slot `i` was `mid`'s victim, and the new entry was
written there. -/
def Placed (old new : List cache.Slot) (s p v : U64) (cost : Usize) (now : U64) (i : Nat) :
    Prop :=
  ∃ mid, Retired old mid s p true true ∧ Victim mid i ∧ i < mid.length ∧
    new = mid.set i { live := true, subject := s, producer := p, version := v,
                      cost := cost, used := now }

/-- `place` refuses a value over the limit and changes nothing; refuses on an
empty table; otherwise the result is `Placed`. -/
theorem place_spec (slots : Slice cache.Slot) (s p v : U64) (cost limit : Usize) (now : U64) :
    cache.place slots s p v cost limit now ⦃ r =>
      match r.1 with
      | none => (limit.val < cost.val ∧ r.2 = slots) ∨
          (cost.val ≤ limit.val ∧ slots.val.length = 0 ∧ r.2.val.length = 0)
      | some i => cost.val ≤ limit.val ∧ Placed slots.val r.2.val s p v cost now i.val ⦄ := by
  unfold cache.place
  split
  next hover =>
    simp only [WP.spec_ok]
    exact Or.inl ⟨ by scalar_tac, trivial ⟩
  next hfits =>
    step as ⟨ mid, hmid ⟩
    step as ⟨ i, hvictim ⟩
    split
    next hnone =>
      simp only [WP.spec_ok]
      have hlen := hmid.1
      have hi : i.val = mid.val.length := by have := hvictim.1; scalar_tac
      have h0 := hvictim.2.1 hi
      exact Or.inr ⟨ by scalar_tac, by omega, h0 ⟩
    next hsome =>
      have hi : i.val < mid.val.length := by scalar_tac
      step as ⟨ nv, hnv ⟩
      refine ⟨ by scalar_tac, mid.val, hmid, hvictim, hi, ?_ ⟩
      simp [hnv]

/-! ## What a placed table guarantees -/

theorem placed_length {old new : List cache.Slot} {s p v : U64} {cost : Usize} {now : U64}
    {i : Nat} (h : Placed old new s p v cost now i) : new.length = old.length := by
  obtain ⟨ mid, ⟨ hlen, _ ⟩, _, _, rfl ⟩ := h
  simp [hlen]

/-- Every live entry costs at most `limit`. -/
def Bounded (limit : Nat) (l : List cache.Slot) : Prop :=
  ∀ j (h : j < l.length), l[j].live = true → l[j].cost.val ≤ limit

/-- The cost of the live entries. -/
def liveCost : List cache.Slot → Nat
  | [] => 0
  | x :: rest => (if x.live = true then x.cost.val else 0) + liveCost rest

/-- A bounded table costs at most `length × limit`: the table is a slice of
fixed length, so this is the cache's memory bound. -/
theorem total_cost_le (limit : Nat) (l : List cache.Slot) (h : Bounded limit l) :
    liveCost l ≤ l.length * limit := by
  induction l with
  | nil => simp [liveCost]
  | cons x rest ih =>
    have hrest : Bounded limit rest := fun j hj hl => h (j + 1) (by simp; omega) (by simpa using hl)
    have hx : (if x.live = true then x.cost.val else 0) ≤ limit := by
      split_ifs with hl
      · exact h 0 (by simp) (by simpa using hl)
      · omega
    have := ih hrest
    simp only [liveCost, List.length_cons, Nat.add_mul, Nat.one_mul]
    omega

/-- `place` keeps the cost bound. -/
theorem placed_bounded {old new : List cache.Slot} {s p v : U64} {cost : Usize} {now : U64}
    {i limit : Nat} (h : Placed old new s p v cost now i) (hcost : cost.val ≤ limit)
    (hold : Bounded limit old) : Bounded limit new := by
  obtain ⟨ mid, ⟨ hlen, hall ⟩, _, hi, rfl ⟩ := h
  intro j hj hlive
  simp only [List.length_set] at hj
  simp only [List.getElem_set] at hlive ⊢
  split_ifs at hlive ⊢ with he
  · exact hcost
  · have hat := hall j (by omega)
    by_cases hsel : Selects old[j] s p true true
    · rw [hat.1 hsel] at hlive; simp at hlive
    · rw [hat.2 hsel] at hlive ⊢
      exact hold j (by omega) hlive

/-- The new entry is in slot `i` under its exact key; every other live slot is
unchanged and belongs to another (subject, producer) pair. -/
theorem placed_entries {old new : List cache.Slot} {s p v : U64} {cost : Usize} {now : U64}
    {i : Nat} (h : Placed old new s p v cost now i) :
    ∃ hi : i < new.length, Holds new[i] s p v ∧ new[i].cost = cost ∧ new[i].used = now ∧
      ∀ j (hj : j < new.length), j ≠ i → new[j].live = true →
        new[j] = old[j]'(by have := placed_length h; omega) ∧
        ¬ (new[j].subject = s ∧ new[j].producer = p) := by
  obtain ⟨ mid, ⟨ hlen, hall ⟩, _, hi, rfl ⟩ := h
  refine ⟨ by simpa using hi, by simp [Holds], by simp, by simp, ?_ ⟩
  intro j hj hne hlive
  simp only [List.length_set] at hj
  simp only [List.getElem_set, show ¬ i = j from fun h => hne h.symm, ↓reduceIte] at hlive ⊢
  have hat := hall j (by omega)
  by_cases hsel : Selects old[j] s p true true
  · rw [hat.1 hsel] at hlive; simp at hlive
  · rw [hat.2 hsel]
    refine ⟨ rfl, fun hpair => hsel ⟨ fun _ => hpair.1, fun _ => hpair.2 ⟩ ⟩

/-- A refresh replaces: the entry just placed is found under its key, and the
same pair at any other version misses. -/
theorem find_after_place (old new : Slice cache.Slot) (s p v : U64) (cost : Usize) (now : U64)
    (i : Usize) (h : Placed old.val new.val s p v cost now i.val) (v' : U64) :
    cache.find new s p v' ⦃ r => (v' = v → r = some i) ∧ (v' ≠ v → r = none) ⦄ := by
  obtain ⟨ hi, hholds, _, _, hothers ⟩ := placed_entries h
  apply WP.spec_mono (find_spec new s p v')
  intro r hr
  cases r with
  | none =>
    refine ⟨ fun hv => ?_, fun _ => rfl ⟩
    subst hv
    exact absurd hholds (hr i.val hi)
  | some k =>
    obtain ⟨ hk, hkholds, _ ⟩ := hr
    have hki : k.val = i.val := by
      by_contra hne
      exact (hothers k.val hk hne hkholds.1).2 ⟨ hkholds.2.1, hkholds.2.2.1 ⟩
    have hkeq : k = i := by scalar_tac
    subst hkeq
    refine ⟨ fun _ => rfl, fun hv => ?_ ⟩
    exact absurd (hkholds.2.2.2.symm.trans hholds.2.2.2) hv

/-- Eviction is least-recently-used and only under pressure: when `place`
overwrites a live entry of another pair, no slot of the table was dead and no
live entry had been used less recently. -/
theorem place_evicts_oldest {old new : List cache.Slot} {s p v : U64} {cost : Usize} {now : U64}
    {i : Nat} (h : Placed old new s p v cost now i) (hi : i < old.length)
    (hlive : old[i].live = true) (hother : ¬ (old[i].subject = s ∧ old[i].producer = p)) :
    ∀ j (hj : j < old.length), old[j].live = true ∧ old[i].used.val ≤ old[j].used.val := by
  obtain ⟨ mid, ⟨ hlen, hall ⟩, ⟨ _, _, hvictim ⟩, himid, _ ⟩ := h
  have hunsel : ¬ Selects old[i] s p true true := fun hsel => hother ⟨ hsel.1 rfl, hsel.2 rfl ⟩
  have hmi : mid[i] = old[i] := (hall i hi).2 hunsel
  rcases hvictim himid with ⟨ hdead, _ ⟩ | ⟨ hall_live, hmin ⟩
  · rw [hmi, hlive] at hdead; simp at hdead
  · intro j hj
    have hmj_live := hall_live j (by omega)
    have hat := hall j hj
    have hmj : mid[j]'(by omega) = old[j] := by
      by_cases hsel : Selects old[j] s p true true
      · rw [hat.1 hsel] at hmj_live; simp at hmj_live
      · exact hat.2 hsel
    have hle := hmin j (by omega)
    rw [hmi, hmj] at hle
    exact ⟨ by rw [← hmj]; exact hmj_live, hle ⟩

end Strop.Kernel
