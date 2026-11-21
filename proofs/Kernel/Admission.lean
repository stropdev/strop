import Extracted.StropKernel

/-!
# Project-index publication and refresh

Contracts over the translated decisions, not over a cache or a second index.
The host owns the observations: non-reused epochs, native path binding, the
exact bytes/format, and the live resolver's trust and capability verdicts.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- Root and native path tokens include *both* incarnations: a name surviving
retirement is not evidence that the resource survived. -/
theorem admission_resource_exact (a b : admission.Resource) :
    admission.same_resource a b ⦃ yes => yes = true ↔ a = b ⦄ := by
  cases a
  cases b
  unfold admission.same_resource
  split_ifs <;> simp_all

/-- Equality of entire stamps includes content, child and language, format,
live request, manifest generation and scan generation. -/
theorem admission_stamp_exact (a b : admission.AnswerStamp) :
    admission.same_answer a b ⦃ yes => yes = true ↔ a = b ⦄ := by
  cases a
  cases b
  unfold admission.same_answer
  apply WP.spec_bind (admission_resource_exact _ _)
  intro same hs
  split_ifs <;> simp_all

/-- Only a disk-owned surface can use disk. A pending, empty or nonempty
buffer-owned surface admits only an actual buffer delivery. -/
def OwnsOrigin (surface : admission.Surface) (origin : admission.Origin) : Prop :=
  (surface = .Disk ∧ origin = .Disk) ∨
  ((surface = .BufferPending ∨ surface = .BufferEmpty ∨ surface = .BufferRows) ∧ origin = .Buffer)

theorem admission_origin_exact (surface : admission.Surface) (origin : admission.Origin) :
    admission.admits_origin surface origin ⦃ yes => yes = true ↔ OwnsOrigin surface origin ⦄ := by
  cases surface <;> cases origin <;> simp [admission.admits_origin, OwnsOrigin]

/-- The candidate was produced by exactly the requested tier and instance.
In the semantic case a real, trusted and capable attachment must exist for
the *target's* live root, child and language; a declaration is not a grant. -/
def AuthorizedProducer (t : admission.AnswerStamp) (want : admission.Designation)
    (used : admission.Producer) : Prop :=
  (∃ id, want = .Text id ∧ used = .Text id) ∨
  (∃ id, want = .Syntax id ∧ used = .Syntax id) ∨
  ∃ live : admission.Attachment, want = .Semantic (.Live live) ∧
    live.trusted = true ∧ live.capable = true ∧ live.root = t.resource.root ∧
    live.root_incarnation = t.resource.root_incarnation ∧ live.child = t.child ∧
    live.language = t.language ∧ used = .Semantic live.server live.attachment

theorem admission_authorized_producer (t : admission.AnswerStamp) (want : admission.Designation)
    (used : admission.Producer) :
    admission.admits_producer t want used ⦃ yes => yes = true ↔ AuthorizedProducer t want used ⦄ := by
  cases want with
  | Text id =>
    cases used <;> simp [admission.admits_producer, AuthorizedProducer,
      core.cmp.impls.PartialEqU64.eq, lift]
    grind
  | Syntax id =>
    cases used <;> simp [admission.admits_producer, AuthorizedProducer,
      core.cmp.impls.PartialEqU64.eq, lift]
    grind
  | Semantic binding =>
    cases binding <;> cases used <;> simp [admission.admits_producer, AuthorizedProducer]
    split_ifs <;> simp_all
    grind

/-- The complete publication decision. Acceptance provides a *current* target.
Cancellation leaves none; supersession cannot republish even a perfectly
matching content snapshot under the successor's query or declaration epoch. -/
theorem admit_live_answer (current : Option admission.Target) (answer : admission.Answer) :
    admission.admit current answer ⦃ yes => yes = true ↔
      ∃ t, current = some t ∧ t.stamp = answer.stamp ∧ OwnsOrigin t.surface answer.origin ∧
        AuthorizedProducer t.stamp t.designated answer.producer ⦄ := by
  cases current with
  | none => simp [admission.admit]
  | some t =>
    unfold admission.admit
    apply WP.spec_bind (admission_stamp_exact _ _)
    intro same hsame
    split
    · next h =>
      apply WP.spec_bind (admission_origin_exact _ _)
      intro owned howned
      split
      · next ho =>
        apply WP.spec_mono (admission_authorized_producer _ _ _)
        intro yes hy
        simp [hsame.mp h, howned.mp ho, hy]
      · next ho => simp [show ¬ OwnsOrigin t.surface answer.origin from fun p => ho (howned.mpr p)]
    · next h => simp [show t.stamp ≠ answer.stamp from fun p => h (hsame.mpr p)]

/-- After begin, any retirement, watch loss, delete/recreate, new invalidation
or new scan leaves a different obligation. Settling such a delivery is the
identity on the *current* state; success discharges its own pending scan. -/
theorem refresh_settles_only_captured (current : admission.Refresh) (started : admission.Scan)
    (success : Bool) :
    admission.settle current started success ⦃ next =>
      (current = .Pending started ∧ success = true → next = .Clean started) ∧
      (next = current ∨
        (current = .Pending started ∧ success = true ∧ next = .Clean started)) ⦄ := by
  cases current with
  | Retired => simp [admission.settle]
  | Unwatched => simp [admission.settle]
  | Clean now => simp [admission.settle]
  | Pending now =>
    cases now
    cases started
    cases success with
    | false => simp [admission.settle]
    | true =>
      unfold admission.settle
      apply WP.spec_bind (admission_resource_exact _ _)
      intro same hs
      split_ifs <;> simp_all

end Strop.Kernel
