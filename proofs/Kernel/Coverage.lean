import Extracted.StropKernel

/-!
# Project query coverage

The required-child slice is the authoritative scope, not a list of replies.
Likewise the path ledger represents all eligible paths observed by the walk,
not just result rows. Constructing these ledgers, and binding a token to the
scope/policy or bytes actually read, are host obligations. The theorems below
decide directly over the extracted consumers of that evidence.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- Arbitrary declaration IDs and cached candidates may promote work; neither
can exclude a path. Unknown paths must still be examined. -/
theorem candidate_hints_only_prioritize (e : coverage.Evidence) (h : coverage.Hint) :
    coverage.schedule e h ⦃ r =>
      (r = .Skip ↔ e = .Excluded) ∧
      (r = .ExamineFirst ↔ e ≠ .Excluded ∧ h ≠ .None) ⦄ := by
  cases e <;> cases h <;> simp [coverage.schedule, WP.spec_ok]

/-- Indexing the authoritative ledger, not comparing the number of result rows. -/
def PathsVerified (ps : Slice coverage.PathWork) : Prop :=
  ∀ j (hj : j < ps.val.length), ps.val[j] = .Verified

@[step]
theorem all_paths_verified (ps : Slice coverage.PathWork) :
    coverage.all_verified ps ⦃ b => b = true ↔ PathsVerified ps ⦄ := by
  unfold coverage.all_verified coverage.all_verified_loop
  apply loop.spec_decr_nat (fun k : Usize => ps.val.length - k.val)
    (fun k : Usize => k.val ≤ ps.val.length ∧
      ∀ j (hj : j < ps.val.length), j < k.val → ps.val[j] = .Verified)
  · intro k ⟨ hk, hbefore ⟩
    unfold coverage.all_verified_loop.body
    dsimp only
    split
    next hlt =>
      have hlt' : k.val < ps.val.length := by scalar_tac
      step as ⟨ w, hw ⟩
      cases w <;> simp [WP.spec_ok]
      all_goals first
        | intro hall
          have hj := hall k.val hlt'
          rw [← hw] at hj
          contradiction
        | step as ⟨ k1, hk1 ⟩
          refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
          intro j hj hjk
          by_cases hjlt : j < k.val
          · exact hbefore j hj hjlt
          · have : j = k.val := by scalar_tac
            subst j
            exact hw.symm
    next hlt =>
      simp only [WP.spec_ok, true_iff, PathsVerified]
      intro j hj
      exact hbefore j hj (by scalar_tac)
  · exact ⟨ by simp, fun _ _ h => by simp at h ⟩

/-- The only certificate minted bears this scan's identity, and is available
iff enumeration, verification and the terminal have all succeeded. Every
other outcome retains its actual terminal, including unsupported coverage. -/
theorem settle_certifies_verified_walk (q : coverage.Query) (p : Option U64) (exhausted : Bool)
    (terminal : coverage.Terminal) (ps : Slice coverage.PathWork) :
    coverage.settle q p exhausted terminal ps ⦃ r =>
      (r = .Complete ⟨q, p⟩ ↔ exhausted = true ∧ terminal = .Success ∧ PathsVerified ps) ∧
      (r = .Partial terminal ∨ r = .Complete ⟨q, p⟩) ⦄ := by
  cases exhausted <;> cases terminal <;> simp [coverage.settle, WP.spec_ok]
  step as ⟨ b, hb ⟩
  cases b <;> simp_all [WP.spec_ok]

/-- A certificate belongs to the *required* owner, not to a sibling that
happened to deliver first. The loose paths have their own `none` owner. -/
def SettledGroup (q : coverage.Query) (p : Option U64) : coverage.Settlement → Prop
  | .Partial _ => False
  | .Complete s => q.scope = s.query.scope ∧ q.policy = s.query.policy ∧
      q.generation = s.query.generation ∧ p = s.project

theorem project_token_matches (p obs : Option U64) :
    coverage.same_project p obs = ok (decide (p = obs)) := by
  cases p <;> cases obs <;> simp [coverage.same_project]

@[step]
theorem certificate_current (q : coverage.Query) (p : Option U64) (r : coverage.Settlement) :
    coverage.current q p r ⦃ b => b = true ↔ SettledGroup q p r ⦄ := by
  cases r with
  | Partial _ => simp [coverage.current, SettledGroup, WP.spec_ok]
  | Complete s =>
    unfold coverage.current SettledGroup
    dsimp only
    split_ifs <;> simp_all [project_token_matches, WP.spec_ok]

/-- Every child slot is required, including slots that have no report. -/
def ChildrenSettled (q : coverage.Query) (cs : Slice coverage.Child) : Prop :=
  ∀ j (hj : j < cs.val.length),
    SettledGroup q (some cs.val[j].project) cs.val[j].scan

theorem every_required_child (q : coverage.Query) (cs : Slice coverage.Child) :
    coverage.workspace_complete_loop q cs 0#usize ⦃ b => b = true ↔ ChildrenSettled q cs ⦄ := by
  unfold coverage.workspace_complete_loop
  apply loop.spec_decr_nat (fun k : Usize => cs.val.length - k.val)
    (fun k : Usize => k.val ≤ cs.val.length ∧
      ∀ j (hj : j < cs.val.length), j < k.val →
        SettledGroup q (some cs.val[j].project) cs.val[j].scan)
  · intro k ⟨ hk, hbefore ⟩
    unfold coverage.workspace_complete_loop.body
    dsimp only
    split
    next hlt =>
      have hlt' : k.val < cs.val.length := by scalar_tac
      step as ⟨ child, hc ⟩
      step as ⟨ b, hb ⟩
      split
      next hy =>
        step as ⟨ k1, hk1 ⟩
        refine ⟨ by scalar_tac, ?_, by scalar_tac ⟩
        intro j hj hjk
        by_cases hjlt : j < k.val
        · exact hbefore j hj hjlt
        · have : j = k.val := by scalar_tac
          subst j
          rw [← hc]
          exact hb.mp hy
      next hn =>
        simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
        intro hall
        have hj := hall k.val hlt'
        rw [← hc] at hj
        exact hn (hb.mpr hj)
    next hlt =>
      simp only [WP.spec_ok, true_iff, ChildrenSettled]
      intro j hj
      exact hbefore j hj (by scalar_tac)
  · exact ⟨ by simp, fun _ _ h => by simp at h ⟩

/-- Completion iff loose paths and *all* required children have valid
coverage for the requested scope, policy and generation. -/
theorem workspace_all_and_loose (q : coverage.Query) (loose : coverage.Settlement)
    (cs : Slice coverage.Child) :
    coverage.workspace_complete q loose cs ⦃ b =>
      b = true ↔ SettledGroup q none loose ∧ ChildrenSettled q cs ⦄ := by
  unfold coverage.workspace_complete
  step as ⟨ b, hb ⟩
  split
  next hy =>
    apply WP.spec_mono (every_required_child q cs)
    intro r hr
    simp [hb.mp hy, hr]
  next hn =>
    have missing : ¬ SettledGroup q none loose := fun h => hn (hb.mpr h)
    simp [WP.spec_ok, missing]

end Strop.Kernel
