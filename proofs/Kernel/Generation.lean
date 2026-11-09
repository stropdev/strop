import Extracted.StropKernel

/-!
# Arena generations (plans/0086 ring 0)

Theorems over the Aeneas translation of `strop_kernel::generation`: a reused
slot's generation advances by exactly one until it would wrap, and then the
slot retires; a fresh slot index exists exactly while the slot count fits the
u32 index space, and equals that count. Together: a stale arena key can never
alias a new occupant.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- `u32::MAX` as a value. -/
theorem u32_MAX_val : (core.num.U32.MAX).val = U32.max := by
  simp [core.num.U32.MAX, U32.rMax, U32.max_eq]

/-- Every `usize` platform can hold every `u32`. -/
theorem u32_max_le_usize_max : U32.max ≤ Usize.max := by
  rcases Usize.bounds_eq with h | h <;> simp [h, U32.max_eq, U64.max_eq]

/-- `next_generation` advances by one, or retires the slot at `u32::MAX`. -/
theorem next_generation_spec (current : U32) :
    generation.next_generation current ⦃ r =>
      (r.isSome ↔ current.val < U32.max) ∧
      (∀ next, r = some next → next.val = current.val + 1) ⦄ := by
  unfold generation.next_generation
  split_ifs with h
  · have : current.val = U32.max := by rw [h, u32_MAX_val]
    simp [this]
  · have hne : current.val ≠ U32.max := by
      intro heq; apply h
      apply UScalar.eq_of_val_eq
      rw [heq, u32_MAX_val]
    have hlt : current.val < U32.max := by scalar_tac
    step as ⟨ next, hnext ⟩
    simp [hlt, hnext]

/-- `index_for_len` is `Some len` exactly while `len` fits a u32. -/
theorem index_for_len_spec (len : Usize) :
    generation.index_for_len len ⦃ r =>
      (r.isSome ↔ len.val ≤ U32.max) ∧
      (∀ index, r = some index → index.val = len.val) ⦄ := by
  unfold generation.index_for_len
  have hfits : (core.num.U32.MAX).val ≤ UScalar.max .Usize := by
    rw [u32_MAX_val, UScalar.max_USize_eq]; exact u32_max_le_usize_max
  apply WP.spec_bind (UScalar.cast_inBounds_spec .Usize core.num.U32.MAX hfits)
  intro bound hbound
  have hb : bound.val = U32.max := by rw [hbound, u32_MAX_val]
  split_ifs with h
  · have : ¬ len.val ≤ U32.max := by scalar_tac
    simp [this]
  · have hle : len.val ≤ U32.max := by scalar_tac
    apply WP.spec_bind (UScalar.cast_inBounds_spec .U32 len (by scalar_tac))
    intro index hindex
    simp [hle, hindex]

end Strop.Kernel
