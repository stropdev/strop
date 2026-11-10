import Extracted.StropKernel
import Kernel.EditMap

/-!
# Excerpt geometry (plans/0094)

Theorems over the Aeneas translation of `strop_kernel::excerpt`. A
collection's excerpt shows the source bytes `ss..se` as the view bytes
`vs..ve`; `editor/collections/write.rs` asks `source_edit` before a view edit
is made and again to place the same edit in the source.

- `source_edit_spec`: the edit is admitted exactly when it lies inside the
  body and leaves the body's terminator in place (`Admitted`), so no edit
  reaches a card border, a gap row or another file's excerpt. An admitted
  edit lands at `toSource` of both ends: the same offset in the source,
  clamped onto the source's end where the view added a terminator, and
  therefore inside `ss..se` (`toSource_in_span`).
- `source_edit_commutes`: follow an anchor through the edit in the view
  (`mapSpec`, which `map_position_spec` proves is the shipped
  `map_position`) and translate it, or translate it and follow it through the
  source edit — the same source byte either way. Carets, marks and the
  excerpts after the edit agree in both buffers; `map_through_edits_spec`
  extends it over a journal one edit at a time.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- A view edit `s..e` lies inside the body `vs..ve`, the excerpt names a
source span, and an edit reaching the body's end keeps its terminator. -/
def Admitted (vs ve ss se s e : Nat) (keeps : Bool) : Prop :=
  ss ≤ se ∧ vs ≤ s ∧ s ≤ e ∧ e ≤ ve ∧ (e = ve → keeps = true)

/-- A view offset translated to the source, clamped onto the source's end. -/
def toSource (vs ss se p : Nat) : Nat := ss + min (p - vs) (se - ss)

/-- A translated offset never leaves the excerpt's source span. -/
theorem toSource_in_span {vs ss se p : Nat} (h : ss ≤ se) :
    ss ≤ toSource vs ss se p ∧ toSource vs ss se p ≤ se := by
  unfold toSource
  omega

theorem clamp_spec (value bound : Usize) :
    excerpt.clamp value bound ⦃ r => r.val = min value.val bound.val ⦄ := by
  unfold excerpt.clamp
  split_ifs with h
  · simp only [WP.spec_ok]
    scalar_tac
  · simp only [WP.spec_ok]
    scalar_tac

theorem admits_spec (vs ve s e : Usize) (keeps : Bool) :
    excerpt.admits vs ve s e keeps ⦃ r =>
      r = true ↔ (vs.val ≤ s.val ∧ s.val ≤ e.val ∧ e.val ≤ ve.val ∧
        (e.val = ve.val → keeps = true)) ⦄ := by
  unfold excerpt.admits
  split_ifs with h1 h2 h3 h4
  · simp only [WP.spec_ok]
    have : ¬ vs.val ≤ s.val := by scalar_tac
    simp [this]
  · simp only [WP.spec_ok]
    have : ¬ s.val ≤ e.val := by scalar_tac
    simp [this]
  · simp only [WP.spec_ok]
    have : ¬ e.val ≤ ve.val := by scalar_tac
    simp [this]
  · simp only [WP.spec_ok]
    have h1' : vs.val ≤ s.val := by scalar_tac
    have h2' : s.val ≤ e.val := by scalar_tac
    have h3' : e.val ≤ ve.val := by scalar_tac
    have h4' : e.val ≠ ve.val := by scalar_tac
    exact ⟨ fun _ => ⟨ h1', h2', h3', fun h => absurd h h4' ⟩, fun _ => trivial ⟩
  · simp only [WP.spec_ok]
    have h1' : vs.val ≤ s.val := by scalar_tac
    have h2' : s.val ≤ e.val := by scalar_tac
    have h3' : e.val ≤ ve.val := by scalar_tac
    have h4' : e.val = ve.val := by scalar_tac
    exact ⟨ fun hk => ⟨ h1', h2', h3', fun _ => hk ⟩, fun h => h.2.2.2 h4' ⟩

/-- `source_edit` admits exactly the edits inside the body and places each
at the translated offsets. -/
theorem source_edit_spec (vs ve ss se s e : Usize) (keeps : Bool) :
    excerpt.source_edit vs ve ss se s e keeps ⦃ r =>
      (r.isSome ↔ Admitted vs.val ve.val ss.val se.val s.val e.val keeps) ∧
      (∀ a b, r = some (a, b) →
        a.val = toSource vs.val ss.val se.val s.val ∧
        b.val = toSource vs.val ss.val se.val e.val) ⦄ := by
  unfold excerpt.source_edit
  split_ifs with hsrc
  · simp only [WP.spec_ok]
    have : ¬ ss.val ≤ se.val := by scalar_tac
    simp [Admitted, this]
  · have hss : ss.val ≤ se.val := by scalar_tac
    apply WP.spec_bind (admits_spec vs ve s e keeps)
    intro ok hok
    split_ifs with hadm
    · obtain ⟨ h1, h2, h3, h4 ⟩ := hok.mp hadm
      step as ⟨ shown, hshown ⟩
      step as ⟨ i, hi ⟩
      apply WP.spec_bind (clamp_spec i shown)
      intro from' hfrom
      step as ⟨ i1, hi1 ⟩
      apply WP.spec_bind (clamp_spec i1 shown)
      intro to' hto
      step as ⟨ a, ha ⟩
      step as ⟨ b, hb ⟩
      simp only [Option.isSome_some, true_iff, Option.some.injEq, Prod.mk.injEq]
      refine ⟨ ⟨ hss, h1, h2, h3, h4 ⟩, ?_ ⟩
      rintro a' b' ⟨ rfl, rfl ⟩
      simp only [toSource, ha, hb, hfrom, hto, hi, hi1, hshown, and_self]
    · simp only [WP.spec_ok, Option.isSome_none, Bool.false_eq_true, false_iff, Admitted]
      refine ⟨ fun h => hadm (hok.mpr h.2), ?_ ⟩
      intro a b h
      cases h

/-- An admitted edit that stays inside the shown source, followed in the
view and then translated, is the translated anchor followed through the
source edit. `n` is the replacement's length; the bound says neither buffer
outgrows the platform, which the shipped buffers guarantee. -/
theorem source_edit_commutes {vs ve ss se s e n p : Nat}
    (hadm : Admitted vs ve ss se s e true)
    (hshown : e - vs ≤ se - ss) (hp : vs ≤ p) (hpe : p - vs ≤ se - ss)
    (hview : s + n + (p - e) ≤ Usize.max)
    (hsource : ss + (s - vs) + n + (p - e) ≤ Usize.max) :
    ss + (mapSpec p s e (s + n) - vs) =
      mapSpec (ss + (p - vs)) (toSource vs ss se s) (toSource vs ss se e)
        (toSource vs ss se s + n) := by
  obtain ⟨ h0, h1, h2, h3, _ ⟩ := hadm
  unfold mapSpec toSource
  split_ifs <;> omega

end Strop.Kernel
