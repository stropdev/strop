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

end Strop.Kernel
