import Extracted.StropKernel

/-!
# Window layout along one axis (plans/0104 §9)

Theorems over the Aeneas translation of `strop_kernel::layout`: how one
split's cells are divided among its children. Child `k` holds `tracks[k]`
windows side by side; each window needs `unit` cells and neighbours a `gap`.

- `minimum_spec`, `total_spec`, `share_spec`: the small functions compute
  `minSpec`, the sum, and `tracks * room / weight` (0 when `weight` is 0).
- `tile_spec`: `tile` writes the pure model `tileModel` into `out`, for every
  child `out` has room for, and leaves the rest of `out` alone. The model is
  the loop itself over a list; everything after it is induction over that
  list, with no Aeneas monad in sight.
- Under `TileBounded` (no product or sum can overflow even a 32-bit `usize`;
  production passes `u16` cells and `(unit, gap)` of `(1, 1)` or `(2, 0)`):
  `tile_in_bounds` (T1) — at any size, no span passes the frame and spans
  are ordered and disjoint.
- Under `TileFits` as well (the minimum fits, every child has a window):
  `tile_exact` (T2) — the spans and gaps cover the frame exactly;
  `tile_minimum` (T3) — every child gets its minimum;
  `tile_fair` (T4) — a child of `t` windows gets between `t·q` and
  `t·(q+1)` of the spare cells, `q` the spare cells per window
  (`quota`); `tile_fair_leaves` — in a flat split any two windows differ by
  at most one cell.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- Every `usize` platform holds a 32-bit value. -/
theorem layout_usize_max_ge : 4294967295 ≤ Usize.max := by
  rcases Usize.bounds_eq with h | h <;> simp [h, U32.max_eq, U64.max_eq]

/-! ## The small functions -/

/-- The fewest cells `t` windows side by side need. -/
def minSpec (t unit gap : Nat) : Nat := if t = 0 then 0 else t * unit + (t - 1) * gap

/-- `minimum` computes `minSpec` whenever the result fits a `usize`. -/
@[step]
theorem minimum_spec (t unit gap : Usize) (h : minSpec t.val unit.val gap.val ≤ Usize.max) :
    layout.minimum t unit gap ⦃ r => r.val = minSpec t.val unit.val gap.val ⦄ := by
  unfold layout.minimum
  unfold minSpec at h
  split_ifs with h0
  · simp [minSpec, h0]
  · have h0' : t.val ≠ 0 := by scalar_tac
    simp only [h0', ↓reduceIte] at h
    step as ⟨ i, hi ⟩
    step as ⟨ i1, hi1 ⟩
    step as ⟨ i2, hi2 ⟩
    step as ⟨ r, hr ⟩
    simp only [minSpec, h0', ↓reduceIte]
    scalar_tac

/-- The values of a slice of `usize`. -/
def vals (s : Slice Usize) : List Nat := s.val.map (·.val)

theorem total_loop_spec (tracks : Slice Usize) (sum k : Usize)
    (hk : k.val ≤ tracks.val.length) (hsum : sum.val = ((vals tracks).take k.val).sum)
    (hfit : (vals tracks).sum ≤ Usize.max) :
    layout.total_loop tracks sum k ⦃ r => r.val = (vals tracks).sum ⦄ := by
  unfold layout.total_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize => tracks.val.length - s.2.val)
    (fun s : Usize × Usize => s.2.val ≤ tracks.val.length ∧
      s.1.val = ((vals tracks).take s.2.val).sum)
  · intro ⟨ s, j ⟩ ⟨ hj, hinv ⟩
    simp only at hj hinv
    unfold layout.total_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < tracks.val.length := by scalar_tac
      have hlen : j.val < (vals tracks).length := by simp [vals, hlt']
      have hsplit := List.sum_take_add_sum_drop (vals tracks) j.val
      rw [List.drop_eq_getElem_cons hlen, List.sum_cons] at hsplit
      have hget : (vals tracks)[j.val] = tracks.val[j.val].val := by simp [vals]
      step as ⟨ x, hx ⟩
      step as ⟨ s1, hs1 ⟩
      step as ⟨ j1, hj1 ⟩
      refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
      rw [hj1, List.sum_take_succ _ _ hlen, hs1, hinv, hget, hx]
    next hge =>
      have hend : j.val = tracks.val.length := by scalar_tac
      simp only [WP.spec_ok]
      rw [hinv, hend, show (↑tracks : List Usize).length = (vals tracks).length by simp [vals],
        List.take_length]
  · exact ⟨ hk, hsum ⟩

/-- `total` is the sum of the slice whenever the sum fits a `usize`. -/
@[step]
theorem total_spec (tracks : Slice Usize) (hfit : (vals tracks).sum ≤ Usize.max) :
    layout.total tracks ⦃ r => r.val = (vals tracks).sum ⦄ := by
  unfold layout.total
  exact total_loop_spec tracks 0#usize 0#usize (by simp) (by simp) hfit

/-- `share` is `tracks * room / weight` (0 when `weight` is 0) whenever the
product fits a `usize`. -/
@[step]
theorem share_spec (room t weight : Usize) (h : t.val * room.val ≤ Usize.max) :
    layout.share room t weight ⦃ r => r.val = t.val * room.val / weight.val ⦄ := by
  unfold layout.share
  step as ⟨ i, hi ⟩
  have hdiv := Usize.checked_div_bv_spec i weight
  simp only [lift]
  split at hdiv
  next z hz =>
    rw [hz]
    simp [core.option.Option.unwrap_or, hdiv.2.1, hi]
  next hz =>
    rw [hz]
    simp [core.option.Option.unwrap_or, hdiv]

/-! ## The pure model of `tile` -/

/-- `tile`'s loop from a child on: `(at, len)` per child, starting at cell
`a` with `room` spare cells and `weight` windows still to serve. -/
def tileFrom (cells unit gap : Nat) : Nat → Nat → Nat → List Nat → List (Nat × Nat)
  | _, _, _, [] => []
  | a, _, _, [_] => [(a, cells - a)]
  | a, room, weight, t :: u :: rest =>
      let extra := t * room / weight
      let len := min (minSpec t unit gap + extra) (cells - a)
      (a, len) :: tileFrom cells unit gap (min (a + len + gap) cells) (room - extra) (weight - t)
        (u :: rest)

/-- What `tile` computes for the children `ts`. -/
def tileModel (cells unit gap : Nat) (ts : List Nat) : List (Nat × Nat) :=
  tileFrom cells unit gap 0 (cells - minSpec ts.sum unit gap) ts.sum ts

theorem tileFrom_length (c u g a r w : Nat) (ts : List Nat) :
    (tileFrom c u g a r w ts).length = ts.length := by
  fun_induction tileFrom c u g a r w ts <;> simp_all

/-- One step of the model: the child at the head and the loop's next state. -/
theorem tileFrom_step (c u g a r w t : Nat) (rest : List Nat) :
    tileFrom c u g a r w (t :: rest) =
      if rest = [] then [(a, c - a)] else
        (a, min (minSpec t u g + t * r / w) (c - a)) ::
          tileFrom c u g (min (a + min (minSpec t u g + t * r / w) (c - a) + g) c)
            (r - t * r / w) (w - t) rest := by
  cases rest <;> simp [tileFrom]

/-- The `TileBounded` hypotheses: no product or sum in `tile` can overflow even a
32-bit `usize`. -/
def TileBounded (cells unit gap : Nat) (ts : List Nat) : Prop :=
  cells ≤ 65535 ∧ ts.sum ≤ 65535 ∧ unit ≤ 2 ∧ gap ≤ 1

/-- `out` after `tile`: child `j` (for every child `out` has room for) holds
the model's span; every other slot is untouched. -/
def Tiled (out r : List layout.Span) (spec : List (Nat × Nat)) : Prop :=
  r.length = out.length ∧
  (∀ j (hs : j < spec.length) (hr : j < r.length), r[j].at.val = spec[j].1 ∧ r[j].len.val = spec[j].2) ∧
  (∀ j (ho : j < out.length) (hr : j < r.length), spec.length ≤ j → r[j] = out[j])

theorem least_spec (a b : Usize) :
    layout.least a b ⦃ r => r.val = min a.val b.val ⦄ := by
  unfold layout.least
  split_ifs with h
  · simp only [WP.spec_ok]; scalar_tac
  · simp only [WP.spec_ok]; scalar_tac

theorem saturating_sub_val (x y : Usize) :
    (core.num.Usize.saturating_sub x y).val = x.val - y.val := by
  simp only [core.num.Usize.saturating_sub, UScalar.saturating_sub, UScalar.val,
    BitVec.toNat_ofNat, Nat.zero_max]
  have := x.hBounds
  rw [Nat.mod_eq_of_lt (by simp only [UScalar.val] at this; omega)]

theorem minSpec_le (t unit gap : Nat) (hu : unit ≤ 2) (hg : gap ≤ 1) :
    minSpec t unit gap ≤ 3 * t := by
  unfold minSpec
  split_ifs
  · omega
  · have h1 : t * unit ≤ t * 2 := Nat.mul_le_mul_left t hu
    have h2 : (t - 1) * gap ≤ (t - 1) * 1 := Nat.mul_le_mul_left (t - 1) hg
    omega

/-- `tile`'s loop state `(out, weight, room, at, k)` before child `k`: the
children before `k` hold the model `M`'s spans, the rest of `out` is
untouched, and the state is the model's at child `k`. -/
def TileInv (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (M : List (Nat × Nat)) (st : Slice layout.Span × Usize × Usize × Usize × Usize) : Prop :=
  st.1.val.length = out.val.length ∧
  st.2.2.2.2.val ≤ tracks.val.length ∧ st.2.2.2.2.val ≤ out.val.length ∧
  st.2.2.2.1.val ≤ cells.val ∧ st.2.2.1.val ≤ cells.val ∧
  (st.2.2.2.2.val < tracks.val.length → st.2.1.val = ((vals tracks).drop st.2.2.2.2.val).sum) ∧
  M.drop st.2.2.2.2.val = tileFrom cells.val unit.val gap.val st.2.2.2.1.val st.2.2.1.val
    st.2.1.val ((vals tracks).drop st.2.2.2.2.val) ∧
  (∀ j (hm : j < M.length) (ho : j < st.1.val.length), j < st.2.2.2.2.val →
    st.1.val[j].at.val = M[j].1 ∧ st.1.val[j].len.val = M[j].2) ∧
  (∀ j (h : j < out.val.length) (ho : j < st.1.val.length), st.2.2.2.2.val ≤ j →
    st.1.val[j] = out.val[j])

/-- One child: the loop writes the model's span and moves to the model's next
state, or stops with `out` tiled. -/
theorem tile_loop_body_spec (cells unit gap : Usize) (tracks : Slice Usize)
    (out : Slice layout.Span) (M : List (Nat × Nat))
    (hb : TileBounded cells.val unit.val gap.val (vals tracks)) (hM : M.length = tracks.val.length)
    (o : Slice layout.Span) (w r a k : Usize) (hinv : TileInv cells unit gap tracks out M (o, w, r, a, k)) :
    layout.tile_loop.body cells unit gap tracks o w r a k ⦃ res =>
      match res with
      | .done y => Tiled out.val y.val M
      | .cont st => TileInv cells unit gap tracks out M st ∧
          tracks.val.length - st.2.2.2.2.val < tracks.val.length - k.val ⦄ := by
  obtain ⟨ hcells, hsum, hunit, hgap ⟩ := hb
  have hmax := layout_usize_max_ge
  obtain ⟨ hlen, hk, hko, ha, hr, hw, hdrop, hdone, hrest ⟩ := hinv
  simp only at hlen hk hko ha hr hw hdrop hdone hrest
  unfold layout.tile_loop.body
  try dsimp only
  split
  next hlt =>
    split
    next hlto =>
      have hkt : k.val < tracks.val.length := by scalar_tac
      have hko' : k.val < out.val.length := by scalar_tac
      step as ⟨ left, hleft ⟩
      step as ⟨ k1, hk1 ⟩
      have hts : k.val < (vals tracks).length := by simp [vals]; omega
      have hMk : k.val < M.length := by omega
      have hget : (vals tracks)[k.val] = tracks.val[k.val].val := by simp [vals]
      rw [List.drop_eq_getElem_cons hts, tileFrom_step, List.drop_eq_getElem_cons hMk] at hdrop
      have hw := hw hkt
      rw [List.drop_eq_getElem_cons hts, List.sum_cons] at hw
      split
      next hlast =>
        have hnil : (vals tracks).drop (k.val + 1) = [] := by
          simp [vals]; scalar_tac
        simp only [hnil, ↓reduceIte, List.cons.injEq] at hdrop
        step as ⟨ s, hs ⟩
        step as ⟨ i4, hi4 ⟩
        step as ⟨ i5, hi5 ⟩
        step with least_spec as ⟨ at1, hat1 ⟩
        refine ⟨ ⟨ by simp [hs, hlen], by scalar_tac, by scalar_tac, by scalar_tac, hr,
          fun h => absurd h (by scalar_tac), ?_, ?_, ?_ ⟩, by scalar_tac ⟩
        · rw [hk1, hdrop.2, hnil]; simp [tileFrom]
        · intro j hm ho hj
          simp only [hs, Slice.set_val_eq, List.getElem_set]
          split_ifs with hkj
          · subst hkj; rw [hdrop.1]; simp [hleft]
          · exact hdone j hm (by simpa [hs] using ho) (by scalar_tac)
        · intro j h ho hj
          simp only [hs, Slice.set_val_eq, List.getElem_set]
          rw [ite_eq_right (fun h => absurd h (by scalar_tac))]
          exact hrest j h (by simpa [hs] using ho) (by scalar_tac)
      next hmid =>
        have hcons : (vals tracks).drop (k.val + 1) ≠ [] := by
          simp [vals]; scalar_tac
        simp only [hcons, ↓reduceIte, List.cons.injEq] at hdrop
        have hdropk := List.sum_take_add_sum_drop (vals tracks) k.val
        have hsumk := List.sum_take_add_sum_drop (vals tracks) 0
        step as ⟨ i4, hi4 ⟩
        have ht : (vals tracks)[k.val] = i4.val := by rw [hget, hi4]
        rw [ht] at hdrop hw
        have hprod : i4.val * r.val ≤ 65535 * 65535 := Nat.mul_le_mul (by simp at hsumk; omega) (by omega)
        step with share_spec as ⟨ extra, hextra ⟩
        have hextra_le : extra.val ≤ r.val := by
          rw [hextra]
          apply Nat.div_le_of_le_mul
          exact Nat.mul_le_mul_right _ (by omega)
        step as ⟨ room2, hroom2 ⟩
        step as ⟨ weight2, hweight2 ⟩
        have hmin := minSpec_le i4.val unit.val gap.val hunit hgap
        step with minimum_spec as ⟨ i5, hi5 ⟩
        step as ⟨ i6, hi6 ⟩
        step with least_spec as ⟨ len1, hlen1 ⟩
        step as ⟨ s, hs ⟩
        step as ⟨ i7, hi7 ⟩
        step as ⟨ i8, hi8 ⟩
        step with least_spec as ⟨ at1, hat1 ⟩
        have hlenv : len1.val = min (minSpec i4.val unit.val gap.val + i4.val * r.val / w.val)
            (cells.val - a.val) := by rw [hlen1, hi6, hi5, hextra, hleft]
        refine ⟨ ⟨ by simp [hs, hlen], by scalar_tac, by scalar_tac, by scalar_tac, by scalar_tac,
          fun _ => by scalar_tac, ?_, ?_, ?_ ⟩, by scalar_tac ⟩
        · rw [hk1, hdrop.2, hat1, hi8, hi7, hroom2, hweight2, hextra, hlenv]
        · intro j hm ho hj
          simp only [hs, Slice.set_val_eq, List.getElem_set]
          split_ifs with hkj
          · subst hkj; rw [hdrop.1]; simp [hlenv]
          · exact hdone j hm (by simpa [hs] using ho) (by scalar_tac)
        · intro j h ho hj
          simp only [hs, Slice.set_val_eq, List.getElem_set]
          rw [ite_eq_right (fun h => absurd h (by scalar_tac))]
          exact hrest j h (by simpa [hs] using ho) (by scalar_tac)
    next hge =>
      have hend : out.val.length ≤ k.val := by scalar_tac
      simp only [WP.spec_ok]
      refine ⟨ hlen, fun j hm ho => hdone j hm ho (by omega), fun j h ho hj => hrest j h ho (by omega) ⟩
  next hge =>
    have hend : tracks.val.length ≤ k.val := by scalar_tac
    simp only [WP.spec_ok]
    refine ⟨ hlen, fun j hm ho => hdone j hm ho (by omega), fun j h ho hj => hrest j h ho (by omega) ⟩

theorem tile_loop_spec (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks))
    (weight room : Usize) (hw : weight.val = (vals tracks).sum) (hr : room.val ≤ cells.val) :
    layout.tile_loop cells unit gap tracks out weight room 0#usize 0#usize ⦃ r =>
      Tiled out.val r.val (tileFrom cells.val unit.val gap.val 0 room.val weight.val (vals tracks)) ⦄ := by
  have hM : (tileFrom cells.val unit.val gap.val 0 room.val weight.val (vals tracks)).length =
      tracks.val.length := by simp [tileFrom_length, vals]
  generalize hMdef : tileFrom cells.val unit.val gap.val 0 room.val weight.val (vals tracks) = M
    at hM ⊢
  unfold layout.tile_loop
  apply loop.spec_decr_nat
    (fun (st : Slice layout.Span × Usize × Usize × Usize × Usize) =>
      tracks.val.length - st.2.2.2.2.val)
    (TileInv cells unit gap tracks out M)
  · intro ⟨ o, w, r, a, k ⟩ hinv
    apply WP.spec_mono (tile_loop_body_spec cells unit gap tracks out M hb hM o w r a k hinv)
    intro res hres
    cases res <;> exact hres
  · refine ⟨ rfl, by simp, by simp, by simp, hr, fun _ => by simpa using hw, by simp [← hMdef], ?_, ?_ ⟩
    · intro j _ _ hj; simp at hj
    · intro j _ _ _; rfl

/-- `tile` writes the model's span for every child `out` has room for, and
leaves the rest of `out` untouched. Any change of `tile`'s behaviour breaks
this. -/
theorem tile_spec (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks)) :
    layout.tile cells unit gap tracks out ⦃ r =>
      Tiled out.val r.val (tileModel cells.val unit.val gap.val (vals tracks)) ⦄ := by
  have hmax := layout_usize_max_ge
  have hb' := hb
  obtain ⟨ hcells, hsum, hunit, hgap ⟩ := hb'
  unfold layout.tile
  step with total_spec as ⟨ weight, hweight ⟩
  have := minSpec_le weight.val unit.val gap.val hunit hgap
  step with minimum_spec as ⟨ need, hneed ⟩
  simp only [lift, bind_ok]
  generalize hroom : core.num.Usize.saturating_sub cells need = room
  have hroomv : room.val = cells.val - need.val := by rw [← hroom, saturating_sub_val]
  apply WP.spec_mono (tile_loop_spec cells unit gap tracks out hb weight room hweight (by omega))
  intro r hr
  rw [hroomv, hneed, hweight] at hr
  exact hr

/-! ## What the model guarantees -/

theorem tileFrom_head (c u g a r w : Nat) (ts : List Nat)
    (h : 0 < (tileFrom c u g a r w ts).length) : (tileFrom c u g a r w ts)[0].1 = a := by
  match ts with
  | [] => simp [tileFrom] at h
  | [_] => simp [tileFrom]
  | _ :: _ :: _ => simp [tileFrom]

/-- At any size, every span ends inside the frame and before the next one
starts. -/
theorem tileFrom_in_bounds (c u g a r w : Nat) (ts : List Nat) (ha : a ≤ c) :
    ∀ i (h : i < (tileFrom c u g a r w ts).length),
      (tileFrom c u g a r w ts)[i].1 + (tileFrom c u g a r w ts)[i].2 ≤ c ∧
      ∀ h' : i + 1 < (tileFrom c u g a r w ts).length,
        (tileFrom c u g a r w ts)[i].1 + (tileFrom c u g a r w ts)[i].2 ≤
          (tileFrom c u g a r w ts)[i + 1].1 := by
  fun_induction tileFrom c u g a r w ts with
  | case1 => intro i h; simp at h
  | case2 a r w t =>
    intro i h
    simp only [List.length_cons, List.length_nil] at h
    have hi : i = 0 := by omega
    subst hi
    simp only [List.getElem_cons_zero]
    exact ⟨ by omega, fun h' => by simp at h' ⟩
  | case3 a r w t v rest extra len ih =>
    intro i h
    have ih := ih (by omega)
    cases i with
    | zero =>
      simp only [List.getElem_cons_zero]
      refine ⟨ by omega, fun h' => ?_ ⟩
      simp only [List.getElem_cons_succ]
      rw [tileFrom_head]
      omega
    | succ i =>
      simp only [List.length_cons] at h
      simp only [List.getElem_cons_succ]
      exact ⟨ (ih i (by omega)).1, fun h' => (ih i (by omega)).2 (by simpa using h') ⟩

/-- A child's minimum and its neighbours': the gap between them is the only
cell the parts do not share. -/
theorem minSpec_add (t v u g : Nat) (ht : 1 ≤ t) (hv : 1 ≤ v) :
    minSpec (t + v) u g = minSpec t u g + g + minSpec v u g := by
  obtain ⟨ t, rfl ⟩ : ∃ t', t = t' + 1 := ⟨ t - 1, by omega ⟩
  obtain ⟨ v, rfl ⟩ : ∃ v', v = v' + 1 := ⟨ v - 1, by omega ⟩
  simp only [minSpec, show t + 1 + (v + 1) ≠ 0 by omega, show t + 1 ≠ 0 by omega,
    show v + 1 ≠ 0 by omega, ↓reduceIte, show t + 1 + (v + 1) - 1 = t + v + 1 by omega,
    Nat.add_sub_cancel]
  ring

/-- One child's share keeps the fairness invariant: with `R` spare cells and
`t + v` windows to serve, `q·(t+v) ≤ R ≤ (q+1)·(t+v)`, the child's share is
between `t·q` and `t·(q+1)` and what is left keeps the invariant for `v`. -/
theorem fair_step (t v r q : Nat) (ht : 1 ≤ t) (hlo : q * (t + v) ≤ r)
    (hhi : r ≤ (q + 1) * (t + v)) :
    t * q ≤ t * r / (t + v) ∧ t * r / (t + v) ≤ t * (q + 1) ∧ t * r / (t + v) ≤ r ∧
    q * v ≤ r - t * r / (t + v) ∧ r - t * r / (t + v) ≤ (q + 1) * v := by
  have hW : 0 < t + v := by omega
  generalize he : t * r / (t + v) = e
  have h1 : e * (t + v) ≤ t * r := he ▸ Nat.div_mul_le_self (t * r) (t + v)
  have h2 : t * r < e * (t + v) + (t + v) := he ▸ Nat.lt_div_mul_add hW
  have hle : e ≤ r := by
    refine Nat.le_of_mul_le_mul_right (c := t + v) ?_ hW
    nlinarith
  refine ⟨ ?_, ?_, hle, ?_, ?_ ⟩
  · by_contra hlt
    have : (e + 1) * (t + v) ≤ t * q * (t + v) := Nat.mul_le_mul_right _ (by omega)
    nlinarith
  · refine Nat.le_of_mul_le_mul_right (c := t + v) ?_ hW
    nlinarith
  · have : (q * v + e) * (t + v) ≤ r * (t + v) := by nlinarith
    have := Nat.le_of_mul_le_mul_right this hW
    omega
  · have : r * (t + v) < (e + 1 + (q + 1) * v) * (t + v) := by nlinarith
    have := Nat.lt_of_mul_lt_mul_right this
    omega

/-- When the minimum fits: each child's length is its minimum plus between
`t·q` and `t·(q+1)` spare cells, the next child starts one gap after it, and
the last child ends on the frame's last cell. -/
theorem tileFrom_fits (c u g q a r w : Nat) (ts : List Nat) (hne : ts ≠ [])
    (hpos : ∀ t ∈ ts, 1 ≤ t) (hw : w = ts.sum) (hc : a + minSpec w u g + r = c)
    (hlo : q * w ≤ r) (hhi : r ≤ (q + 1) * w) :
    (∀ i (h : i < (tileFrom c u g a r w ts).length) (ht : i < ts.length),
      minSpec ts[i] u g + ts[i] * q ≤ (tileFrom c u g a r w ts)[i].2 ∧
      (tileFrom c u g a r w ts)[i].2 ≤ minSpec ts[i] u g + ts[i] * (q + 1)) ∧
    (∀ i (h : i + 1 < (tileFrom c u g a r w ts).length),
      (tileFrom c u g a r w ts)[i + 1].1 =
        (tileFrom c u g a r w ts)[i].1 + (tileFrom c u g a r w ts)[i].2 + g) ∧
    (∀ i (h : i < (tileFrom c u g a r w ts).length), i + 1 = (tileFrom c u g a r w ts).length →
      (tileFrom c u g a r w ts)[i].1 + (tileFrom c u g a r w ts)[i].2 = c) := by
  fun_induction tileFrom c u g a r w ts with
  | case1 => exact absurd rfl hne
  | case2 a r w t =>
    simp only [List.sum_singleton] at hw
    subst hw
    simp only [tileFrom]
    refine ⟨ ?_, ?_, ?_ ⟩
    · intro i h ht
      simp only [List.length_cons, List.length_nil] at h
      have hi : i = 0 := by omega
      subst hi
      simp only [List.getElem_cons_zero]
      rw [Nat.mul_comm] at hlo hhi
      omega
    · intro i h; simp at h
    · intro i h _
      simp only [List.length_cons, List.length_nil] at h
      have hi : i = 0 := by omega
      subst hi
      simp only [List.getElem_cons_zero]
      omega
  | case3 a r w t v rest extra len ih =>
    have hL : tileFrom c u g a r w (t :: v :: rest) =
        (a, len) :: tileFrom c u g (min (a + len + g) c) (r - extra) (w - t) (v :: rest) := by
      rw [tileFrom]
    simp only [hL]
    have hv : 1 ≤ (v :: rest).sum := by
      have := hpos v (by simp); simp only [List.sum_cons]; omega
    have ht : 1 ≤ t := hpos t (by simp)
    have hwv : w = t + (v :: rest).sum := by rw [hw]; simp
    have hw' : w - t = (v :: rest).sum := by omega
    have hext : extra = t * r / (t + (v :: rest).sum) := by simp only [extra, hwv]
    rw [hwv] at hlo hhi
    obtain ⟨ hql, hqh, hle, hlo', hhi' ⟩ := fair_step t (v :: rest).sum r q ht hlo hhi
    rw [← hext] at hql hqh hle hlo' hhi'
    have hmin := minSpec_add t (v :: rest).sum u g ht hv
    rw [← hwv] at hmin
    have hlen : len = minSpec t u g + extra := by
      simp only [len]; omega
    have hnext : min (a + len + g) c = a + len + g := by omega
    have ih := ih (by simp) (fun x hx => hpos x (by simp [hx])) hw'
      (by rw [hnext, hw']; omega) (by rw [hw']; exact hlo') (by rw [hw']; exact hhi')
    obtain ⟨ ih1, ih2, ih3 ⟩ := ih
    refine ⟨ ?_, ?_, ?_ ⟩
    · intro i h hts
      cases i with
      | zero =>
        simp only [List.getElem_cons_zero]
        rw [hlen]; simp only [extra]; omega
      | succ i =>
        simp only [List.getElem_cons_succ]
        exact ih1 i _ (by simpa using hts)
    · intro i h
      cases i with
      | zero =>
        simp only [List.getElem_cons_zero, List.getElem_cons_succ]
        rw [tileFrom_head, hnext]
      | succ i =>
        simp only [List.getElem_cons_succ]
        exact ih2 i _
    · intro i h hlast
      cases i with
      | zero => simp [tileFrom_length] at hlast
      | succ i =>
        simp only [List.getElem_cons_succ]
        exact ih3 i _ (by simpa using hlast)

/-- The minimum fits the frame, every child holds at least one window, and
there is at least one child. -/
def TileFits (cells unit gap : Nat) (ts : List Nat) : Prop :=
  minSpec ts.sum unit gap ≤ cells ∧ (∀ t ∈ ts, 1 ≤ t) ∧ ts ≠ []

/-- The whole frame's fairness quotient: spare cells per window. -/
def quota (cells unit gap : Nat) (ts : List Nat) : Nat :=
  (cells - minSpec ts.sum unit gap) / ts.sum

theorem tileModel_length (c u g : Nat) (ts : List Nat) : (tileModel c u g ts).length = ts.length :=
  tileFrom_length _ _ _ _ _ _ _

theorem tileModel_fits (c u g : Nat) (ts : List Nat) (hf : TileFits c u g ts) :
    let L := tileModel c u g ts
    (∀ i (h : i < L.length) (ht : i < ts.length),
      minSpec ts[i] u g + ts[i] * quota c u g ts ≤ L[i].2 ∧
      L[i].2 ≤ minSpec ts[i] u g + ts[i] * (quota c u g ts + 1)) ∧
    (∀ i (h : i + 1 < L.length), L[i + 1].1 = L[i].1 + L[i].2 + g) ∧
    (∀ i (h : i < L.length), i + 1 = L.length → L[i].1 + L[i].2 = c) := by
  obtain ⟨ hmin, hpos, hne ⟩ := hf
  have hW : 0 < ts.sum := by
    obtain ⟨ t, rest, rfl ⟩ := List.exists_cons_of_ne_nil hne
    have := hpos t (by simp); simp only [List.sum_cons]; omega
  have hlo := Nat.div_mul_le_self (c - minSpec ts.sum u g) ts.sum
  have hhi := Nat.lt_div_mul_add (a := c - minSpec ts.sum u g) hW
  exact tileFrom_fits c u g (quota c u g ts) 0 _ _ ts hne hpos rfl (by omega)
    (by rw [quota]; exact hlo) (by rw [quota, Nat.add_mul, Nat.one_mul]; omega)

/-! ## The headline theorems, over the shipped `tile` -/

/-- T1: at any frame size, no span passes the frame and every span ends
before the next one starts: panes never overlap and nothing reaches the
modeline. -/
theorem tile_in_bounds (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks)) :
    layout.tile cells unit gap tracks out ⦃ r =>
      ∀ k (_ : k < tracks.val.length) (hr : k < r.val.length),
        r.val[k].at.val + r.val[k].len.val ≤ cells.val ∧
        ∀ (_ : k + 1 < tracks.val.length) (hr' : k + 1 < r.val.length),
          r.val[k].at.val + r.val[k].len.val ≤ r.val[k + 1].at.val ⦄ := by
  apply WP.spec_mono (tile_spec cells unit gap tracks out hb)
  intro r ⟨ _, hget, _ ⟩ k hk hr
  have hML := tileModel_length cells.val unit.val gap.val (vals tracks)
  have hvl : (vals tracks).length = tracks.val.length := by simp [vals]
  have hb := tileFrom_in_bounds cells.val unit.val gap.val 0
    (cells.val - minSpec (vals tracks).sum unit.val gap.val) (vals tracks).sum (vals tracks)
    (Nat.zero_le _) k (by unfold tileModel at hML; omega)
  obtain ⟨ hat, hlen ⟩ := hget k (by omega) hr
  unfold tileModel at hat hlen
  refine ⟨ by rw [hat, hlen]; exact hb.1, fun hk' hr' => ?_ ⟩
  obtain ⟨ hat', _ ⟩ := hget (k + 1) (by omega) hr'
  unfold tileModel at hat'
  rw [hat, hlen, hat']
  exact hb.2 (by unfold tileModel at hML; omega)

/-- T2: when the minimum fits, the spans and the gaps between them cover the
frame exactly: the first starts at 0, each next one a gap after the previous
one's end, and the last ends on the frame's last cell. -/
theorem tile_exact (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks))
    (hf : TileFits cells.val unit.val gap.val (vals tracks)) :
    layout.tile cells unit gap tracks out ⦃ r =>
      (∀ h0 : 0 < r.val.length, r.val[0].at.val = 0) ∧
      (∀ k (_ : k + 1 < tracks.val.length) (hr' : k + 1 < r.val.length),
        r.val[k + 1].at.val = r.val[k].at.val + r.val[k].len.val + gap.val) ∧
      (∀ k (_ : k + 1 = tracks.val.length) (hr : k < r.val.length),
        r.val[k].at.val + r.val[k].len.val = cells.val) ⦄ := by
  apply WP.spec_mono (tile_spec cells unit gap tracks out hb)
  intro r ⟨ _, hget, _ ⟩
  have hML := tileModel_length cells.val unit.val gap.val (vals tracks)
  have hvl : (vals tracks).length = tracks.val.length := by simp [vals]
  have hne : 0 < tracks.val.length := by
    have := hf.2.2; rw [← hvl]; exact List.length_pos_iff.mpr this
  obtain ⟨ _, hnext, hlast ⟩ := tileModel_fits _ _ _ _ hf
  refine ⟨ fun h0 => ?_, fun k hk' hr' => ?_, fun k hk hr => ?_ ⟩
  · rw [(hget 0 (by omega) h0).1]
    unfold tileModel
    exact tileFrom_head _ _ _ _ _ _ _ _
  · rw [(hget (k + 1) (by omega) hr').1, (hget k (by omega) (by omega)).1,
      (hget k (by omega) (by omega)).2]
    exact hnext k (by omega)
  · rw [(hget k (by omega) hr).1, (hget k (by omega) hr).2]
    exact hlast k (by omega) (by omega)

/-- T3: when the minimum fits, every child gets at least its minimum: a
column per window side by side, a title and a row per window stacked. -/
theorem tile_minimum (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks))
    (hf : TileFits cells.val unit.val gap.val (vals tracks)) :
    layout.tile cells unit gap tracks out ⦃ r =>
      ∀ k (hk : k < tracks.val.length) (hr : k < r.val.length),
        minSpec tracks.val[k].val unit.val gap.val ≤ r.val[k].len.val ⦄ := by
  apply WP.spec_mono (tile_spec cells unit gap tracks out hb)
  intro r ⟨ _, hget, _ ⟩ k hk hr
  have hML := tileModel_length cells.val unit.val gap.val (vals tracks)
  have hvl : (vals tracks).length = tracks.val.length := by simp [vals]
  obtain ⟨ hfair, _, _ ⟩ := tileModel_fits _ _ _ _ hf
  have := (hfair k (by omega) (by omega)).1
  rw [(hget k (by omega) hr).2]
  simp only [vals, List.getElem_map] at this ⊢
  omega

/-- T4: when the minimum fits, a child of `t` windows gets its minimum plus
between `t·q` and `t·(q+1)` of the spare cells, `q` being the spare cells per
window over the whole frame: `ctrl-w =` by construction. -/
theorem tile_fair (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks))
    (hf : TileFits cells.val unit.val gap.val (vals tracks)) :
    layout.tile cells unit gap tracks out ⦃ r =>
      ∀ k (hk : k < tracks.val.length) (hr : k < r.val.length),
        minSpec tracks.val[k].val unit.val gap.val +
            tracks.val[k].val * quota cells.val unit.val gap.val (vals tracks) ≤ r.val[k].len.val ∧
        r.val[k].len.val ≤ minSpec tracks.val[k].val unit.val gap.val +
            tracks.val[k].val * (quota cells.val unit.val gap.val (vals tracks) + 1) ⦄ := by
  apply WP.spec_mono (tile_spec cells unit gap tracks out hb)
  intro r ⟨ _, hget, _ ⟩ k hk hr
  have hML := tileModel_length cells.val unit.val gap.val (vals tracks)
  have hvl : (vals tracks).length = tracks.val.length := by simp [vals]
  obtain ⟨ hfair, _, _ ⟩ := tileModel_fits _ _ _ _ hf
  have := hfair k (by omega) (by omega)
  rw [(hget k (by omega) hr).2]
  simp only [vals, List.getElem_map] at this ⊢
  exact this

/-- T4 for one flat split: when every child is a single window, any two
lengths differ by at most one cell. -/
theorem tile_fair_leaves (cells unit gap : Usize) (tracks : Slice Usize) (out : Slice layout.Span)
    (hb : TileBounded cells.val unit.val gap.val (vals tracks))
    (hf : TileFits cells.val unit.val gap.val (vals tracks)) (hone : ∀ t ∈ vals tracks, t = 1) :
    layout.tile cells unit gap tracks out ⦃ r =>
      ∀ i j (_ : i < tracks.val.length) (_ : j < tracks.val.length)
        (hri : i < r.val.length) (hrj : j < r.val.length),
        r.val[i].len.val ≤ r.val[j].len.val + 1 ⦄ := by
  apply WP.spec_mono (tile_fair cells unit gap tracks out hb hf)
  intro r hfair i j hi hj hri hrj
  have h1 : ∀ k (hk : k < tracks.val.length), tracks.val[k].val = 1 := fun k hk =>
    hone _ (by simp only [vals]; exact List.mem_map.mpr ⟨ _, List.getElem_mem hk, rfl ⟩)
  have := hfair i hi hri
  have := hfair j hj hrj
  simp only [h1 i hi, h1 j hj, Nat.one_mul] at *
  omega

end Strop.Kernel
