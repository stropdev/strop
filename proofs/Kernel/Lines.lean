import Kernel.EditMap

/-!
# Line geometry across a diff (line-geometry kernel RFC, #287)

Theorems over the Aeneas translation of `strop_kernel::lines`
(`Extracted/StropKernel.lean`). A region is a quadruple of plain numbers
`(old, old_len, new, new_len)`: old lines `old..old + old_len` became new
lines `new..new + new_len`.

- `coherent_spec`: `coherent` accepts exactly the lists `Coherent` describes:
  every region starts at or after the previous one's end on both sides, at
  the same distance on both, is non-empty, and ends inside `usize`.
- `runs_spec`: `runs` appends exactly `RunsOf` of the body: one region per
  maximal stretch of non-context lines, its lengths counting that stretch's
  deletions and additions, its starts the lines before it on each side.
- `runs_coherent`: what `runs` produces for one hunk is always a diff.
- `span_spec`: `span` orders a selection's two ends.
- `old_line_spec`, `new_line_spec`: over a coherent list, the line maps answer
  exactly the kept lines (`Kept`): new line `n` was old line `o` when `n` lies
  in a gap between regions and `o` at the same offset into the matching old
  gap. `old_span_spec`: a range is answered exactly when every line of it is
  kept and they stay consecutive on the old side. None of them fails on any
  input (the coherence premise sits inside the postcondition).
- `kept_respects_regions`: a kept line is never a deleted or an added line,
  and the pairing keeps order.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-! ## The model -/

/-- A region as plain numbers: (old, old_len, new, new_len). -/
abbrev Quad := Nat × Nat × Nat × Nat

def quad (r : lines.Region) : Quad := (r.old.val, r.old_len.val, r.new.val, r.new_len.val)

/-- The diffs `coherent` accepts; `ob`/`nb` are the first lines after the
regions already passed. -/
def Coherent (ob nb : Nat) : List Quad → Prop
  | [] => True
  | (ro, rol, rn, rnl) :: rest =>
      ob ≤ ro ∧ nb ≤ rn ∧ ro - ob = rn - nb ∧ 0 < rol + rnl ∧
      ro + rol ≤ Usize.max ∧ rn + rnl ≤ Usize.max ∧
      Coherent (ro + rol) (rn + rnl) rest

/-- A pending run, appended when it changed anything. -/
def closeRun (q : Quad) : List Quad :=
  if q.2.1 = 0 ∧ q.2.2.2 = 0 then [] else [q]

/-- The regions of a body from the pending run `q` on: a context line closes
the run and opens an empty one on the next line of both sides; a deletion or
an addition lengthens the run on its side. -/
def runsFrom : Quad → List lines.Origin → List Quad
  | q, [] => closeRun q
  | (o, ol, n, nl), .Context :: rest =>
      closeRun (o, ol, n, nl) ++ runsFrom (o + ol + 1, 0, n + nl + 1, 0) rest
  | (o, ol, n, nl), .Deletion :: rest => runsFrom (o, ol + 1, n, nl) rest
  | (o, ol, n, nl), .Addition :: rest => runsFrom (o, ol, n, nl + 1) rest

/-- The regions a hunk body denotes when its first line is old line `os` and
new line `ns`. -/
def RunsOf (os ns : Nat) (body : List lines.Origin) : List Quad :=
  runsFrom (os, 0, ns, 0) body

/-! ## `coherent` -/

theorem coherent_loop_spec (regions : Slice lines.Region) (ob nb k : Usize)
    (hk : k.val ≤ regions.val.length)
    (hinv : Coherent 0 0 (regions.val.map quad) ↔
      Coherent ob.val nb.val ((regions.val.map quad).drop k.val)) :
    lines.coherent_loop regions ob nb k ⦃ b =>
      b = true ↔ Coherent 0 0 (regions.val.map quad) ⦄ := by
  unfold lines.coherent_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize × Usize => regions.val.length - s.2.2.val)
    (fun s : Usize × Usize × Usize => s.2.2.val ≤ regions.val.length ∧
      (Coherent 0 0 (regions.val.map quad) ↔
        Coherent s.1.val s.2.1.val ((regions.val.map quad).drop s.2.2.val)))
  · intro ⟨ ob, nb, j ⟩ ⟨ hj, hinv ⟩
    simp only at hj hinv
    unfold lines.coherent_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < regions.val.length := by scalar_tac
      have hdrop : (regions.val.map quad).drop j.val =
          quad regions.val[j.val] :: (regions.val.map quad).drop (j.val + 1) := by
        rw [List.drop_eq_getElem_cons (by simp [hlt'])]; simp
      rw [hdrop] at hinv
      step as ⟨ r, hr ⟩
      have hr' : regions.val[j.val] = r := by rw [hr]
      rw [hr'] at hinv
      simp only [quad, Coherent] at hinv
      split_ifs <;> step*
    next hge =>
      have hend : j.val = regions.val.length := by scalar_tac
      simp only [WP.spec_ok, true_iff]
      rw [hinv, hend]; simp [Coherent]
  · exact ⟨ hk, hinv ⟩

/-- `coherent` accepts exactly the region lists that describe a diff, for
every input. -/
theorem coherent_spec (regions : Slice lines.Region) :
    lines.coherent regions ⦃ b => b = true ↔ Coherent 0 0 (regions.val.map quad) ⦄ := by
  unfold lines.coherent
  exact coherent_loop_spec regions 0#usize 0#usize 0#usize (by simp) (by simp)

/-! ## `runs` -/

theorem lines_zero_val : (0#usize : Usize).val = 0 := by simp
theorem lines_one_val : (1#usize : Usize).val = 1 := by simp

theorem close_spec (run : lines.Region) (out : alloc.vec.Vec lines.Region)
    (hroom : out.val.length < Usize.max) :
    lines.close run out ⦃ r =>
      r.val.map quad = out.val.map quad ++ closeRun (quad run) ∧
      r.val.length ≤ out.val.length + 1 ⦄ := by
  unfold lines.close
  split_ifs with h1 h2
  · step as ⟨ o, ho ⟩
    have h : ¬ (run.old_len.val = 0 ∧ run.new_len.val = 0) := by
      simp only [bne_iff_ne, ne_eq] at h1; scalar_tac
    simp only [ho, closeRun, quad, List.map_append, List.map_cons, List.map_nil]
    simp_all
  · step as ⟨ o, ho ⟩
    have h : ¬ (run.old_len.val = 0 ∧ run.new_len.val = 0) := by
      simp only [bne_iff_ne, ne_eq] at h2; scalar_tac
    simp only [ho, closeRun, quad, List.map_append, List.map_cons, List.map_nil]
    simp_all
  · have h : run.old_len.val = 0 ∧ run.new_len.val = 0 := by
      simp only [bne_iff_ne, ne_eq, Decidable.not_not] at h1 h2; scalar_tac
    simp only [WP.spec_ok, closeRun, quad]
    simp_all

theorem runsFrom_context (o ol n nl : Nat) (rest : List lines.Origin) :
    runsFrom (o, ol, n, nl) (.Context :: rest) =
      closeRun (o, ol, n, nl) ++ runsFrom (o + ol + 1, 0, n + nl + 1, 0) rest := rfl

theorem runs_loop_spec (body : Slice lines.Origin) (out : alloc.vec.Vec lines.Region)
    (o ol n nl k : Usize) (hk : k.val ≤ body.val.length)
    (ho : o.val + ol.val + (body.val.length - k.val) ≤ Usize.max)
    (hn : n.val + nl.val + (body.val.length - k.val) ≤ Usize.max)
    (hroom : out.val.length + (body.val.length - k.val) < Usize.max) :
    lines.runs_loop body out o ol n nl k ⦃ r =>
      r.1.val.map quad ++ closeRun (r.2.1.val, r.2.2.1.val, r.2.2.2.1.val, r.2.2.2.2.val) =
        out.val.map quad ++ runsFrom (o.val, ol.val, n.val, nl.val) (body.val.drop k.val) ∧
      r.1.val.length < Usize.max ⦄ := by
  unfold lines.runs_loop
  apply loop.spec_decr_nat
    (fun s : alloc.vec.Vec lines.Region × Usize × Usize × Usize × Usize × Usize =>
      body.val.length - s.2.2.2.2.2.val)
    (fun s => s.2.2.2.2.2.val ≤ body.val.length ∧
      s.2.1.val + s.2.2.1.val + (body.val.length - s.2.2.2.2.2.val) ≤ Usize.max ∧
      s.2.2.2.1.val + s.2.2.2.2.1.val + (body.val.length - s.2.2.2.2.2.val) ≤ Usize.max ∧
      s.1.val.length + (body.val.length - s.2.2.2.2.2.val) < Usize.max ∧
      s.1.val.map quad ++ runsFrom (s.2.1.val, s.2.2.1.val, s.2.2.2.1.val, s.2.2.2.2.1.val)
          (body.val.drop s.2.2.2.2.2.val) =
        out.val.map quad ++ runsFrom (o.val, ol.val, n.val, nl.val) (body.val.drop k.val))
  · intro ⟨ cur, o1, ol1, n1, nl1, j ⟩ ⟨ hj, ho1, hn1, hr1, hinv ⟩
    simp only at hj ho1 hn1 hr1 hinv
    unfold lines.runs_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < body.val.length := by scalar_tac
      have hdrop : body.val.drop j.val = body.val[j.val] :: body.val.drop (j.val + 1) :=
        List.drop_eq_getElem_cons hlt'
      rw [hdrop] at hinv
      step as ⟨ x, hx ⟩
      have hx' : body.val[j.val] = x := by rw [hx]
      rw [hx'] at hinv
      cases x
      · -- a context line closes the run
        simp only
        step with close_spec as ⟨ c, hc, hcl ⟩
        simp only [lift]
        step as ⟨ j1, hj1 ⟩
        have hs1 : (core.num.Usize.saturating_add (core.num.Usize.saturating_add o1 ol1)
            1#usize).val = o1.val + ol1.val + 1 := by
          simp only [saturating_add_val, lines_one_val]; omega
        have hs2 : (core.num.Usize.saturating_add (core.num.Usize.saturating_add n1 nl1)
            1#usize).val = n1.val + nl1.val + 1 := by
          simp only [saturating_add_val, lines_one_val]; omega
        simp only [hs1, hs2, hj1, lines_zero_val]
        refine ⟨ by omega, by omega, by omega, by omega, ?_, by omega ⟩
        rw [hc, List.append_assoc, ← hinv, runsFrom_context]
        simp [quad]
      · -- an addition lengthens the run on the new side
        simp only [lift]
        step as ⟨ j1, hj1 ⟩
        have hs : (core.num.Usize.saturating_add nl1 1#usize).val = nl1.val + 1 := by
          simp only [saturating_add_val, lines_one_val]; omega
        simp only [hs, hj1]
        refine ⟨ by omega, by omega, by omega, by omega, ?_, by omega ⟩
        rw [← hinv]; rfl
      · -- a deletion lengthens the run on the old side
        simp only [lift]
        step as ⟨ j1, hj1 ⟩
        have hs : (core.num.Usize.saturating_add ol1 1#usize).val = ol1.val + 1 := by
          simp only [saturating_add_val, lines_one_val]; omega
        simp only [hs, hj1]
        refine ⟨ by omega, by omega, by omega, by omega, ?_, by omega ⟩
        rw [← hinv]; rfl
    next hge =>
      have hend : j.val = body.val.length := by scalar_tac
      simp only [WP.spec_ok]
      rw [hend, List.drop_length] at hinv
      refine ⟨ ?_, by scalar_tac ⟩
      rw [← hinv]; simp [runsFrom]
  · exact ⟨ hk, ho, hn, hroom, rfl ⟩

/-- `runs` appends exactly the regions the body denotes, whenever no line
number of the hunk passes `usize` and `out` has room for the pushes. -/
theorem runs_spec (os ns : Usize) (body : Slice lines.Origin) (out : alloc.vec.Vec lines.Region)
    (hos : os.val + body.val.length ≤ Usize.max) (hns : ns.val + body.val.length ≤ Usize.max)
    (hroom : out.val.length + body.val.length < Usize.max) :
    lines.runs os ns body out ⦃ r =>
      r.val.map quad = out.val.map quad ++ RunsOf os.val ns.val body.val ⦄ := by
  unfold lines.runs
  step with runs_loop_spec as ⟨ c, o, ol, n, nl, hc, hcl ⟩
  step with close_spec as ⟨ d, hd, _ ⟩
  rw [hd]
  simp only [List.drop_zero] at hc
  simp only [quad, RunsOf]
  exact hc

/-! ## A hunk's regions are a diff -/

theorem coherent_closeRun_append (ob nb o ol n nl : Nat) (rest : List Quad)
    (hob : ob ≤ o) (hnb : nb ≤ n) (hd : o - ob = n - nb)
    (ho : o + ol ≤ Usize.max) (hn : n + nl ≤ Usize.max)
    (hrest : ol = 0 ∧ nl = 0 → Coherent ob nb rest)
    (hnext : Coherent (o + ol) (n + nl) rest) :
    Coherent ob nb (closeRun (o, ol, n, nl) ++ rest) := by
  unfold closeRun
  split_ifs with h
  · exact hrest h
  · simp only [List.singleton_append, Coherent]
    simp only at h
    exact ⟨ hob, hnb, hd, by omega, ho, hn, hnext ⟩

theorem runsFrom_coherent (body : List lines.Origin) :
    ∀ (ob nb o ol n nl : Nat), ob ≤ o → nb ≤ n → o - ob = n - nb →
      o + ol + body.length ≤ Usize.max → n + nl + body.length ≤ Usize.max →
      Coherent ob nb (runsFrom (o, ol, n, nl) body) := by
  induction body with
  | nil =>
    intro ob nb o ol n nl hob hnb hd ho hn
    simp only [runsFrom]
    have := coherent_closeRun_append ob nb o ol n nl [] hob hnb hd (by simp at ho; omega)
      (by simp at hn; omega) (fun _ => trivial) trivial
    simpa using this
  | cons x rest ih =>
    intro ob nb o ol n nl hob hnb hd ho hn
    simp only [List.length_cons] at ho hn
    cases x with
    | Context =>
      rw [runsFrom_context]
      apply coherent_closeRun_append ob nb o ol n nl _ hob hnb hd (by omega) (by omega)
      · intro h
        exact ih ob nb _ _ _ _ (by omega) (by omega) (by omega) (by omega) (by omega)
      · exact ih _ _ _ _ _ _ (by omega) (by omega) (by omega) (by omega)
          (by omega)
    | Addition =>
      exact ih ob nb o ol n (nl + 1) hob hnb hd (by omega) (by omega)
    | Deletion =>
      exact ih ob nb o (ol + 1) n nl hob hnb hd (by omega) (by omega)

/-- What `runs` produces for one hunk is always a diff: its regions are
ordered, at the same distance on both sides, non-empty, and inside `usize`. -/
theorem runs_coherent (os ns : Nat) (body : List lines.Origin)
    (hos : os + body.length ≤ Usize.max) (hns : ns + body.length ≤ Usize.max) :
    Coherent os ns (RunsOf os ns body) :=
  runsFrom_coherent body os ns os 0 ns 0 le_rfl le_rfl (by simp) (by omega) (by omega)

/-! ## Selections -/

/-- `span` orders a selection's two ends, for every input. -/
theorem span_spec (a h : Usize) :
    lines.span a h ⦃ r => r.1.val = min a.val h.val ∧ r.2.val = max a.val h.val ⦄ := by
  unfold lines.span
  split_ifs with hle
  · simp only [WP.spec_ok]; constructor <;> scalar_tac
  · simp only [WP.spec_ok]; constructor <;> scalar_tac

/-! ## Line maps -/

/-- New line `n` is a kept line and was old line `o`: `n` lies in a gap
between regions and `o` sits at the same offset in the matching old gap.
`ob`/`nb` are the first lines after the regions already passed. -/
def Kept (ob nb : Nat) : List Quad → Nat → Nat → Prop
  | [], o, n => nb ≤ n ∧ o = ob + (n - nb)
  | (ro, rol, rn, rnl) :: rest, o, n =>
      (nb ≤ n ∧ n < rn ∧ o = ob + (n - nb)) ∨ Kept (ro + rol) (rn + rnl) rest o n

theorem kept_new_ge : ∀ (qs : List Quad) (ob nb o n : Nat),
    Coherent ob nb qs → Kept ob nb qs o n → nb ≤ n
  | [], _, _, _, _, _, h => h.1
  | (ro, rol, rn, rnl) :: rest, ob, nb, o, n, hc, h => by
    simp only [Coherent] at hc
    simp only [Kept] at h
    rcases h with h | h
    · exact h.1
    · have := kept_new_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega

theorem kept_old_ge : ∀ (qs : List Quad) (ob nb o n : Nat),
    Coherent ob nb qs → Kept ob nb qs o n → ob ≤ o
  | [], _, _, _, _, _, h => by simp only [Kept] at h; omega
  | (ro, rol, rn, rnl) :: rest, ob, nb, o, n, hc, h => by
    simp only [Coherent] at hc
    simp only [Kept] at h
    rcases h with h | h
    · omega
    · have := kept_old_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega

/-- Every region of a coherent list starts at or after the bases. -/
theorem coherent_starts : ∀ (qs : List Quad) (ob nb : Nat), Coherent ob nb qs →
    ∀ q ∈ qs, ob ≤ q.1 ∧ nb ≤ q.2.2.1
  | [], _, _, _, q, hq => by simp at hq
  | (ro, rol, rn, rnl) :: rest, ob, nb, hc, q, hq => by
    simp only [Coherent] at hc
    rcases List.mem_cons.mp hq with rfl | hq
    · exact ⟨ hc.1, hc.2.1 ⟩
    · have := coherent_starts rest _ _ hc.2.2.2.2.2.2 q hq; omega

/-- A new line before the next region is kept, at its offset into the gap. -/
theorem kept_stop_new (ob nb ro rol rn rnl o n : Nat) (rest : List Quad)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (hn : nb ≤ n) (hlt : n < rn) :
    Kept ob nb ((ro, rol, rn, rnl) :: rest) o n ↔ o = ob + (n - nb) := by
  simp only [Coherent] at hc
  simp only [Kept]
  constructor
  · rintro (h | h)
    · exact h.2.2
    · have := kept_new_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega
  · intro h; exact Or.inl ⟨ hn, hlt, h ⟩

/-- A new line a region added is not kept. -/
theorem kept_inside_new (ob nb ro rol rn rnl o n : Nat) (rest : List Quad)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (h1 : rn ≤ n) (h2 : n < rn + rnl) :
    ¬ Kept ob nb ((ro, rol, rn, rnl) :: rest) o n := by
  simp only [Coherent] at hc
  simp only [Kept]
  rintro (h | h)
  · omega
  · have := kept_new_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega

/-- Past a region on the new side, only the rest of the list can keep `n`. -/
theorem kept_skip_new (ob nb ro rol rn rnl o n : Nat) (rest : List Quad) (h : rn ≤ n) :
    Kept ob nb ((ro, rol, rn, rnl) :: rest) o n ↔ Kept (ro + rol) (rn + rnl) rest o n := by
  simp only [Kept]
  constructor
  · rintro (h' | h')
    · omega
    · exact h'
  · exact Or.inr

/-- An old line before the next region is kept, at its offset into the gap. -/
theorem kept_stop_old (ob nb ro rol rn rnl o n : Nat) (rest : List Quad)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (ho : ob ≤ o) (hlt : o < ro) :
    Kept ob nb ((ro, rol, rn, rnl) :: rest) o n ↔ n = nb + (o - ob) := by
  simp only [Coherent] at hc
  simp only [Kept]
  constructor
  · rintro (h | h)
    · omega
    · have := kept_old_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega
  · intro h; exact Or.inl ⟨ by omega, by omega, by omega ⟩

/-- An old line a region deleted is not kept. -/
theorem kept_inside_old (ob nb ro rol rn rnl o n : Nat) (rest : List Quad)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (h1 : ro ≤ o) (h2 : o < ro + rol) :
    ¬ Kept ob nb ((ro, rol, rn, rnl) :: rest) o n := by
  simp only [Coherent] at hc
  simp only [Kept]
  rintro (h | h)
  · omega
  · have := kept_old_ge rest _ _ o n hc.2.2.2.2.2.2 h; omega

/-- Past a region on the old side, only the rest of the list can keep `o`. -/
theorem kept_skip_old (ob nb ro rol rn rnl o n : Nat) (rest : List Quad)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (h : ro ≤ o) :
    Kept ob nb ((ro, rol, rn, rnl) :: rest) o n ↔ Kept (ro + rol) (rn + rnl) rest o n := by
  simp only [Coherent] at hc
  simp only [Kept]
  constructor
  · rintro (h' | h')
    · omega
    · exact h'
  · exact Or.inr

theorem shift_spec (line a b : Usize) (h : a.val ≤ line.val) :
    lines.shift line a b ⦃ r =>
      (∀ u, r = some u → u.val = b.val + (line.val - a.val)) ∧
      (r = none ↔ Usize.max < b.val + (line.val - a.val)) ⦄ := by
  unfold lines.shift
  step as ⟨ d, hd ⟩
  step as ⟨ room, hroom ⟩
  split_ifs with hgt
  · simp only [WP.spec_ok, reduceCtorEq, false_implies, implies_true, true_iff, true_and]
    scalar_tac
  · step as ⟨ x, hx ⟩
    simp only [Option.some.injEq, reduceCtorEq, false_iff, not_lt]
    exact ⟨ fun u hu => by rw [← hu, hx, hd], by scalar_tac ⟩

theorem drop_quad_cons (rs : Slice lines.Region) (j : Nat) (h : j < rs.val.length) :
    (rs.val.map quad).drop j = quad rs.val[j] :: (rs.val.map quad).drop (j + 1) := by
  rw [List.drop_eq_getElem_cons (by simp [h])]; simp

/-- What `old_line` answers for new line `n`, over a coherent list. -/
def OldLineOk (qs : List Quad) (n : Nat) (r : Option Usize) : Prop :=
  Coherent 0 0 qs → ∀ o, (∃ u, r = some u ∧ u.val = o) ↔ Kept 0 0 qs o n ∧ o ≤ Usize.max

theorem shift_answers (line a b : Usize) (r : Option Usize)
    (h1 : ∀ u, r = some u → u.val = b.val + (line.val - a.val))
    (h2 : r = none ↔ Usize.max < b.val + (line.val - a.val)) (x : Nat) :
    (∃ u, r = some u ∧ u.val = x) ↔ x = b.val + (line.val - a.val) ∧ x ≤ Usize.max := by
  constructor
  · rintro ⟨ u, rfl, rfl ⟩
    have := h1 u rfl
    have : u.val ≤ Usize.max := by scalar_tac
    omega
  · rintro ⟨ hx, hle ⟩
    cases r with
    | none => have := h2.mp rfl; omega
    | some u => exact ⟨ u, rfl, by rw [h1 u rfl, hx] ⟩

theorem old_line_loop_spec (n : Usize) (rs : Slice lines.Region) (ob nb k : Usize)
    (hk : k.val ≤ rs.val.length) (hn : nb.val ≤ n.val)
    (hinv : Coherent 0 0 (rs.val.map quad) →
      Coherent ob.val nb.val ((rs.val.map quad).drop k.val) ∧
      ∀ o, (Kept 0 0 (rs.val.map quad) o n.val ↔
        Kept ob.val nb.val ((rs.val.map quad).drop k.val) o n.val)) :
    lines.old_line_loop n rs ob nb k ⦃ r => OldLineOk (rs.val.map quad) n.val r ⦄ := by
  unfold lines.old_line_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize × Usize => rs.val.length - s.2.2.val)
    (fun s : Usize × Usize × Usize => s.2.2.val ≤ rs.val.length ∧ s.2.1.val ≤ n.val ∧
      (Coherent 0 0 (rs.val.map quad) →
        Coherent s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) ∧
        ∀ o, (Kept 0 0 (rs.val.map quad) o n.val ↔
          Kept s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) o n.val)))
  · intro ⟨ ob, nb, j ⟩ ⟨ hj, hn, hinv ⟩
    simp only at hj hn hinv
    unfold lines.old_line_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < rs.val.length := by scalar_tac
      rw [drop_quad_cons rs j.val hlt'] at hinv
      step as ⟨ r, hr ⟩
      have hr' : rs.val[j.val] = r := by rw [hr]
      rw [hr'] at hinv
      simp only [quad] at hinv
      split
      next hstop =>
        step with shift_spec as ⟨ x, hx1, hx2 ⟩
        intro hc o
        obtain ⟨ hc', hk ⟩ := hinv hc
        rw [shift_answers _ _ _ _ hx1 hx2, hk o,
          kept_stop_new _ _ _ _ _ _ _ _ _ hc' hn (by scalar_tac)]
      next hpast =>
        simp only [lift, bind_ok]
        split
        next hin =>
          have hin' : n.val < min (r.new.val + r.new_len.val) Usize.max := by
            rw [← saturating_add_val]; scalar_tac
          simp only [WP.spec_ok]
          intro hc o
          obtain ⟨ hc', hk ⟩ := hinv hc
          have hfit := hc'.2.2.2.2.2.1
          constructor
          · rintro ⟨ u, hu, _ ⟩; cases hu
          · rintro ⟨ hkept, _ ⟩
            exact absurd ((hk o).mp hkept)
              (kept_inside_new _ _ _ _ _ _ _ _ _ hc' (by scalar_tac) (by omega))
        next hout =>
          have hout' : min (r.new.val + r.new_len.val) Usize.max ≤ n.val := by
            rw [← saturating_add_val]; scalar_tac
          step as ⟨ j1, hj1 ⟩
          refine ⟨ by scalar_tac, by rw [saturating_add_val]; omega, ?_, by scalar_tac ⟩
          intro hc
          obtain ⟨ hc', hk ⟩ := hinv hc
          simp only [Coherent] at hc'
          simp only [saturating_add_val, hj1]
          have e1 : min (r.old.val + r.old_len.val) Usize.max = r.old.val + r.old_len.val := by
            omega
          have e2 : min (r.new.val + r.new_len.val) Usize.max = r.new.val + r.new_len.val := by
            omega
          rw [e1, e2]
          refine ⟨ hc'.2.2.2.2.2.2, fun o => ?_ ⟩
          rw [hk o, kept_skip_new _ _ _ _ _ _ _ _ _ (by scalar_tac)]
    next hge =>
      have hend : j.val = rs.val.length := by scalar_tac
      step with shift_spec as ⟨ x, hx1, hx2 ⟩
      intro hc o
      obtain ⟨ _, hk ⟩ := hinv hc
      rw [shift_answers _ _ _ _ hx1 hx2, hk o, hend]
      rw [List.drop_eq_nil_of_le (by simp)]
      simp only [Kept]
      omega
  · exact ⟨ hk, hn, hinv ⟩

/-- `old_line` never fails, and over a coherent region list it answers a new
line's old line exactly when the line is kept (and the answer fits a
`usize`). -/
theorem old_line_spec (n : Usize) (rs : Slice lines.Region) :
    lines.old_line n rs ⦃ r => Coherent 0 0 (rs.val.map quad) →
      ∀ o : Nat, (∃ u, r = some u ∧ u.val = o) ↔
        Kept 0 0 (rs.val.map quad) o n.val ∧ o ≤ Usize.max ⦄ := by
  unfold lines.old_line
  exact old_line_loop_spec n rs 0#usize 0#usize 0#usize (by simp) (by simp)
    (fun hc => ⟨ by simpa using hc, fun o => by simp ⟩)

/-- What `new_line` answers for old line `o`, over a coherent list. -/
def NewLineOk (qs : List Quad) (o : Nat) (r : Option Usize) : Prop :=
  Coherent 0 0 qs → ∀ n, (∃ u, r = some u ∧ u.val = n) ↔ Kept 0 0 qs o n ∧ n ≤ Usize.max

theorem new_line_loop_spec (o : Usize) (rs : Slice lines.Region) (ob nb k : Usize)
    (hk : k.val ≤ rs.val.length) (ho : ob.val ≤ o.val)
    (hinv : Coherent 0 0 (rs.val.map quad) →
      Coherent ob.val nb.val ((rs.val.map quad).drop k.val) ∧
      ∀ n, (Kept 0 0 (rs.val.map quad) o.val n ↔
        Kept ob.val nb.val ((rs.val.map quad).drop k.val) o.val n)) :
    lines.new_line_loop o rs ob nb k ⦃ r => NewLineOk (rs.val.map quad) o.val r ⦄ := by
  unfold lines.new_line_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize × Usize => rs.val.length - s.2.2.val)
    (fun s : Usize × Usize × Usize => s.2.2.val ≤ rs.val.length ∧ s.1.val ≤ o.val ∧
      (Coherent 0 0 (rs.val.map quad) →
        Coherent s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) ∧
        ∀ n, (Kept 0 0 (rs.val.map quad) o.val n ↔
          Kept s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) o.val n)))
  · intro ⟨ ob, nb, j ⟩ ⟨ hj, ho, hinv ⟩
    simp only at hj ho hinv
    unfold lines.new_line_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < rs.val.length := by scalar_tac
      rw [drop_quad_cons rs j.val hlt'] at hinv
      step as ⟨ r, hr ⟩
      have hr' : rs.val[j.val] = r := by rw [hr]
      rw [hr'] at hinv
      simp only [quad] at hinv
      split
      next hstop =>
        step with shift_spec as ⟨ x, hx1, hx2 ⟩
        intro hc n
        obtain ⟨ hc', hk ⟩ := hinv hc
        rw [shift_answers _ _ _ _ hx1 hx2, hk n,
          kept_stop_old _ _ _ _ _ _ _ _ _ hc' ho (by scalar_tac)]
      next hpast =>
        simp only [lift, bind_ok]
        split
        next hin =>
          have hin' : o.val < min (r.old.val + r.old_len.val) Usize.max := by
            rw [← saturating_add_val]; scalar_tac
          simp only [WP.spec_ok]
          intro hc n
          obtain ⟨ hc', hk ⟩ := hinv hc
          have hfit := hc'.2.2.2.2.1
          constructor
          · rintro ⟨ u, hu, _ ⟩; cases hu
          · rintro ⟨ hkept, _ ⟩
            exact absurd ((hk n).mp hkept)
              (kept_inside_old _ _ _ _ _ _ _ _ _ hc' (by scalar_tac) (by omega))
        next hout =>
          have hout' : min (r.old.val + r.old_len.val) Usize.max ≤ o.val := by
            rw [← saturating_add_val]; scalar_tac
          step as ⟨ j1, hj1 ⟩
          refine ⟨ by scalar_tac, by rw [saturating_add_val]; omega, ?_, by scalar_tac ⟩
          intro hc
          obtain ⟨ hc', hk ⟩ := hinv hc
          have hc'' := hc'
          simp only [Coherent] at hc'
          simp only [saturating_add_val, hj1]
          have e1 : min (r.old.val + r.old_len.val) Usize.max = r.old.val + r.old_len.val := by
            omega
          have e2 : min (r.new.val + r.new_len.val) Usize.max = r.new.val + r.new_len.val := by
            omega
          rw [e1, e2]
          refine ⟨ hc'.2.2.2.2.2.2, fun n => ?_ ⟩
          rw [hk n, kept_skip_old _ _ _ _ _ _ _ _ _ hc'' (by scalar_tac)]
    next hge =>
      have hend : j.val = rs.val.length := by scalar_tac
      step with shift_spec as ⟨ x, hx1, hx2 ⟩
      intro hc n
      obtain ⟨ _, hk ⟩ := hinv hc
      rw [shift_answers _ _ _ _ hx1 hx2, hk n, hend]
      rw [List.drop_eq_nil_of_le (by simp)]
      simp only [Kept]
      omega
  · exact ⟨ hk, ho, hinv ⟩

/-- `new_line` never fails, and over a coherent region list it answers an old
line's new line exactly when the line is kept (and the answer fits a
`usize`). -/
theorem new_line_spec (o : Usize) (rs : Slice lines.Region) :
    lines.new_line o rs ⦃ r => Coherent 0 0 (rs.val.map quad) →
      ∀ n : Nat, (∃ u, r = some u ∧ u.val = n) ↔
        Kept 0 0 (rs.val.map quad) o.val n ∧ n ≤ Usize.max ⦄ := by
  unfold lines.new_line
  exact new_line_loop_spec o rs 0#usize 0#usize 0#usize (by simp) (by simp)
    (fun hc => ⟨ by simpa using hc, fun n => by simp ⟩)

/-- New lines `f..=l` are kept and consecutive on the old side from `a`. -/
def KeptRun (qs : List Quad) (f l a : Nat) : Prop :=
  ∀ i, f ≤ i → i ≤ l → Kept 0 0 qs (a + (i - f)) i

/-- What `old_span` answers for new lines `f..=l`, over a coherent list: a
range only when every line is kept and consecutive on the old side, and
`none` only when no such old range exists (inside `usize`). -/
def OldSpanOk (qs : List Quad) (f l : Nat) (r : Option (Usize × Usize)) : Prop :=
  Coherent 0 0 qs →
    (∀ a b, r = some (a, b) → b.val - a.val = l - f ∧ a.val ≤ b.val ∧ KeptRun qs f l a.val) ∧
    (r = none → l < f ∨ ¬ ∃ a, a + (l - f) ≤ Usize.max ∧ KeptRun qs f l a)

/-- Once the walk stops (`l` before the next region, or no region left), the
range maps at its offset into the gap. -/
theorem old_span_some (qs rest : List Quad) (ob nb f l : Nat) (a b : Usize)
    (hfl : f ≤ l) (hnb : nb ≤ f)
    (hk : ∀ o i, nb ≤ i → (Kept 0 0 qs o i ↔ Kept ob nb rest o i))
    (hgap : ∀ o i, nb ≤ i → i ≤ l → (Kept ob nb rest o i ↔ o = ob + (i - nb)))
    (hb : b.val = ob + (l - nb)) (ha : a.val = b.val - (l - f)) :
    OldSpanOk qs f l (some (a, b)) := by
  intro _
  constructor
  · intro a' b' h
    simp only [Option.some.injEq, Prod.mk.injEq] at h
    obtain ⟨ rfl, rfl ⟩ := h
    refine ⟨ by omega, by omega, fun i hfi hil => ?_ ⟩
    rw [hk _ i (by omega), hgap _ i (by omega) hil]
    omega
  · intro h; simp at h

/-- ... and when the old range would pass `usize`, no old range exists. -/
theorem old_span_overflow (qs rest : List Quad) (ob nb f l : Nat)
    (hfl : f ≤ l) (hnb : nb ≤ f)
    (hk : ∀ o i, nb ≤ i → (Kept 0 0 qs o i ↔ Kept ob nb rest o i))
    (hgap : ∀ o i, nb ≤ i → i ≤ l → (Kept ob nb rest o i ↔ o = ob + (i - nb)))
    (hover : Usize.max < ob + (l - nb)) :
    OldSpanOk qs f l none := by
  intro _
  refine ⟨ fun a b h => by simp at h, fun _ => Or.inr ?_ ⟩
  rintro ⟨ a, hfit, hrun ⟩
  have := (hgap _ f hnb hfl).mp ((hk _ f hnb).mp (hrun f le_rfl hfl))
  omega

/-- The walk refuses at a region the range touches: no old range exists. -/
theorem old_span_refused (qs rest : List Quad) (ob nb ro rol rn rnl f l : Nat)
    (hc : Coherent ob nb ((ro, rol, rn, rnl) :: rest)) (hnb : nb ≤ f) (hfl : f ≤ l)
    (hk : ∀ o i, nb ≤ i → (Kept 0 0 qs o i ↔ Kept ob nb ((ro, rol, rn, rnl) :: rest) o i))
    (hl : rn ≤ l) (hf : f < rn + rnl) :
    ¬ ∃ a, a + (l - f) ≤ Usize.max ∧ KeptRun qs f l a := by
  rintro ⟨ a, _, hrun ⟩
  by_cases hadd : 0 < rnl
  · -- the range holds a line the region added
    by_cases hfr : f ≤ rn
    · exact kept_inside_new _ _ _ _ _ _ _ _ _ hc le_rfl (by omega)
        ((hk _ _ (by omega)).mp (hrun rn hfr hl))
    · exact kept_inside_new _ _ _ _ _ _ _ _ _ hc (by omega) hf
        ((hk _ _ hnb).mp (hrun f le_rfl hfl))
  · -- a pure deletion falls between two lines of the range
    have hc' := hc
    simp only [Coherent] at hc'
    have h1 := (kept_stop_new _ _ _ _ _ _ _ _ _ hc (by omega) (by omega)).mp
      ((hk _ (rn - 1) (by omega)).mp (hrun (rn - 1) (by omega) (by omega)))
    have h2 := (kept_skip_new _ _ _ _ _ _ _ _ _ (le_refl rn)).mp
      ((hk _ rn (by omega)).mp (hrun rn (by omega) hl))
    have := kept_old_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h2
    omega

theorem old_span_loop_spec (f l : Usize) (rs : Slice lines.Region) (ob nb k : Usize)
    (hfl : f.val ≤ l.val) (hk : k.val ≤ rs.val.length) (hnb : nb.val ≤ f.val)
    (hinv : Coherent 0 0 (rs.val.map quad) →
      Coherent ob.val nb.val ((rs.val.map quad).drop k.val) ∧
      ∀ o i, nb.val ≤ i → (Kept 0 0 (rs.val.map quad) o i ↔
        Kept ob.val nb.val ((rs.val.map quad).drop k.val) o i)) :
    lines.old_span_loop f l rs ob nb k ⦃ r => OldSpanOk (rs.val.map quad) f.val l.val r ⦄ := by
  unfold lines.old_span_loop
  apply loop.spec_decr_nat (fun s : Usize × Usize × Usize => rs.val.length - s.2.2.val)
    (fun s : Usize × Usize × Usize => s.2.2.val ≤ rs.val.length ∧ s.2.1.val ≤ f.val ∧
      (Coherent 0 0 (rs.val.map quad) →
        Coherent s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) ∧
        ∀ o i, s.2.1.val ≤ i → (Kept 0 0 (rs.val.map quad) o i ↔
          Kept s.1.val s.2.1.val ((rs.val.map quad).drop s.2.2.val) o i)))
  · intro ⟨ ob, nb, j ⟩ ⟨ hj, hnb, hinv ⟩
    simp only at hj hnb hinv
    unfold lines.old_span_loop.body
    try dsimp only
    split
    next hlt =>
      have hlt' : j.val < rs.val.length := by scalar_tac
      rw [drop_quad_cons rs j.val hlt'] at hinv
      step as ⟨ r, hr ⟩
      have hr' : rs.val[j.val] = r := by rw [hr]
      rw [hr'] at hinv
      simp only [quad] at hinv
      split
      next hstop =>
        have hk' := fun hc => (hinv hc).2
        have hgap : Coherent 0 0 (rs.val.map quad) → ∀ o i, nb.val ≤ i → i ≤ l.val →
            (Kept ob.val nb.val ((r.old.val, r.old_len.val, r.new.val, r.new_len.val) :: (rs.val.map quad).drop (j.val + 1)) o i ↔ o = ob.val + (i - nb.val)) := fun hc o i hi hil =>
              kept_stop_new _ _ _ _ _ _ _ _ _ (hinv hc).1 hi (by scalar_tac)
        step as ⟨ d, hd ⟩
        step as ⟨ room, hroom ⟩
        split_ifs with hover
        · simp only [WP.spec_ok]
          intro hc
          exact old_span_overflow _ _ ob.val nb.val f.val l.val hfl hnb (hk' hc) (hgap hc)
            (by scalar_tac) hc
        · step as ⟨ e, he ⟩
          step as ⟨ i1, hi1 ⟩
          step as ⟨ i2, hi2 ⟩
          intro hc
          exact old_span_some _ _ ob.val nb.val f.val l.val i2 e hfl hnb (hk' hc) (hgap hc)
            (by rw [he, hd]) (by rw [hi2, hi1]) hc
      next hpast =>
        simp only [lift, bind_ok]
        split
        next hin =>
          have hin' : f.val < min (r.new.val + r.new_len.val) Usize.max := by
            rw [← saturating_add_val]; scalar_tac
          simp only [WP.spec_ok]
          intro hc
          obtain ⟨ hc', hk ⟩ := hinv hc
          have hfit := hc'.2.2.2.2.2.1
          refine ⟨ fun a b h => by simp at h, fun _ => Or.inr ?_ ⟩
          exact old_span_refused _ _ _ _ _ _ _ _ f.val l.val hc' hnb hfl hk (by scalar_tac)
            (by omega)
        next hout =>
          have hout' : min (r.new.val + r.new_len.val) Usize.max ≤ f.val := by
            rw [← saturating_add_val]; scalar_tac
          step as ⟨ j1, hj1 ⟩
          refine ⟨ by scalar_tac, by rw [saturating_add_val]; omega, ?_, by scalar_tac ⟩
          intro hc
          obtain ⟨ hc', hk ⟩ := hinv hc
          simp only [Coherent] at hc'
          simp only [saturating_add_val, hj1]
          have e1 : min (r.old.val + r.old_len.val) Usize.max = r.old.val + r.old_len.val := by
            omega
          have e2 : min (r.new.val + r.new_len.val) Usize.max = r.new.val + r.new_len.val := by
            omega
          rw [e1, e2]
          refine ⟨ hc'.2.2.2.2.2.2, fun o i hi => ?_ ⟩
          rw [hk o i (by omega), kept_skip_new _ _ _ _ _ _ _ _ _ (by omega)]
    next hge =>
      have hend : j.val = rs.val.length := by scalar_tac
      have hnil : (rs.val.map quad).drop j.val = [] := List.drop_eq_nil_of_le (by simp; omega)
      rw [hnil] at hinv
      have hk' := fun hc => (hinv hc).2
      have hgap : Coherent 0 0 (rs.val.map quad) → ∀ o i, nb.val ≤ i → i ≤ l.val →
          (Kept ob.val nb.val [] o i ↔ o = ob.val + (i - nb.val)) := fun _ o i hi _ => by simp only [Kept]; omega
      step as ⟨ d, hd ⟩
      step as ⟨ room, hroom ⟩
      split_ifs with hover
      · simp only [WP.spec_ok]
        intro hc
        exact old_span_overflow _ _ ob.val nb.val f.val l.val hfl hnb (hk' hc) (hgap hc)
          (by scalar_tac) hc
      · step as ⟨ e, he ⟩
        step as ⟨ i1, hi1 ⟩
        step as ⟨ i2, hi2 ⟩
        intro hc
        exact old_span_some _ _ ob.val nb.val f.val l.val i2 e hfl hnb (hk' hc) (hgap hc)
          (by rw [he, hd]) (by rw [hi2, hi1]) hc
  · exact ⟨ hk, hnb, hinv ⟩

/-- `old_span` never fails, and over a coherent region list it answers new
lines `f..=l` with an old range exactly when every line of it is kept and
they are consecutive on the old side (inside `usize`). -/
theorem old_span_spec (f l : Usize) (rs : Slice lines.Region) :
    lines.old_span f l rs ⦃ r => Coherent 0 0 (rs.val.map quad) →
      (∀ a b, r = some (a, b) → b.val - a.val = l.val - f.val ∧ a.val ≤ b.val ∧
        ∀ i, f.val ≤ i → i ≤ l.val → Kept 0 0 (rs.val.map quad) (a.val + (i - f.val)) i) ∧
      (r = none → l.val < f.val ∨ ¬ ∃ a, a + (l.val - f.val) ≤ Usize.max ∧
        ∀ i, f.val ≤ i → i ≤ l.val → Kept 0 0 (rs.val.map quad) (a + (i - f.val)) i) ⦄ := by
  unfold lines.old_span
  split_ifs with hlf
  · simp only [WP.spec_ok]
    intro _
    exact ⟨ fun a b h => by simp at h, fun _ => Or.inl (by scalar_tac) ⟩
  · exact old_span_loop_spec f l rs 0#usize 0#usize 0#usize (by scalar_tac) (by simp)
      (by simp) (fun hc => ⟨ by simpa using hc, fun o i _ => by simp ⟩)

/-! ## Kept lines -/

theorem kept_outside : ∀ (qs : List Quad) (ob nb o n : Nat), Coherent ob nb qs →
    Kept ob nb qs o n → ∀ q ∈ qs,
      ¬ (q.1 ≤ o ∧ o < q.1 + q.2.1) ∧ ¬ (q.2.2.1 ≤ n ∧ n < q.2.2.1 + q.2.2.2)
  | [], _, _, _, _, _, _, q, hq => by simp at hq
  | (ro, rol, rn, rnl) :: rest, ob, nb, o, n, hc, h, q, hq => by
    have hc' := hc
    simp only [Coherent] at hc'
    have hstarts := coherent_starts rest _ _ hc'.2.2.2.2.2.2
    simp only [Kept] at h
    rcases h with h | h
    · -- in the gap before this region: below it, and below every later one
      rcases List.mem_cons.mp hq with rfl | hq
      · simp only; omega
      · have := hstarts q hq; omega
    · rcases List.mem_cons.mp hq with rfl | hq
      · have := kept_old_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h
        have := kept_new_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h
        simp only; omega
      · exact kept_outside rest _ _ o n hc'.2.2.2.2.2.2 h q hq

theorem kept_order : ∀ (qs : List Quad) (ob nb o n o' n' : Nat), Coherent ob nb qs →
    Kept ob nb qs o n → Kept ob nb qs o' n' → (n < n' ↔ o < o')
  | [], _, _, _, _, _, _, _, h, h' => by simp only [Kept] at h h'; omega
  | (ro, rol, rn, rnl) :: rest, ob, nb, o, n, o', n', hc, h, h' => by
    have hc' := hc
    simp only [Coherent] at hc'
    simp only [Kept] at h h'
    rcases h with h | h <;> rcases h' with h' | h'
    · omega
    · have := kept_old_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h'
      have := kept_new_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h'
      omega
    · have := kept_old_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h
      have := kept_new_ge rest _ _ _ _ hc'.2.2.2.2.2.2 h
      omega
    · exact kept_order rest _ _ o n o' n' hc'.2.2.2.2.2.2 h h'

/-- Over a coherent region list a kept line is never a deleted or an added
line, and the pairing keeps order on both sides. -/
theorem kept_respects_regions (ob nb : Nat) (qs : List Quad) (hc : Coherent ob nb qs) :
    (∀ o n, Kept ob nb qs o n → ∀ q ∈ qs,
      ¬ (q.1 ≤ o ∧ o < q.1 + q.2.1) ∧ ¬ (q.2.2.1 ≤ n ∧ n < q.2.2.1 + q.2.2.2)) ∧
    (∀ o n o' n', Kept ob nb qs o n → Kept ob nb qs o' n' → (n < n' ↔ o < o')) :=
  ⟨ fun o n h => kept_outside qs ob nb o n hc h,
    fun o n o' n' h h' => kept_order qs ob nb o n o' n' hc h h' ⟩

end Strop.Kernel
