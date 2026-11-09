import Extracted.StropKernel

/-!
# Worker frame admission (plans/0086 ring 0)

Theorems over the Aeneas translation of `strop_kernel::frame`. Each admission
is exactly its bound, and `frame_ready` is total: for every header offset,
body length and buffer size it terminates without overflow, and when it
reports a frame ready the body slice `[header_end + 4, header_end + 4 + length)`
lies inside the buffered bytes — the take in `FrameDecoder::next_frame` cannot
go out of bounds or wrap.
-/

open Aeneas Aeneas.Std Result
open strop_kernel

namespace Strop.Kernel

/-- A header fits iff the terminator ends within 8192 bytes. -/
theorem header_admitted_spec (header_end : Usize) :
    frame.header_admitted header_end ⦃ r => r ↔ header_end.val + 4 ≤ 8192 ⦄ := by
  unfold frame.header_admitted
  simp only [frame.MAX_HEADER_BYTES]
  step as ⟨ bound, hbound ⟩
  simp only [decide_eq_true_eq]
  scalar_tac

/-- A body fits iff it is at most one MiB. -/
theorem body_admitted_spec (length : Usize) :
    frame.body_admitted length ⦃ r => r ↔ length.val ≤ 1048576 ⦄ := by
  unfold frame.body_admitted
  simp only [frame.MAX_BODY_BYTES]
  step as ⟨ bound, hbound ⟩
  simp only [decide_eq_true_eq]
  scalar_tac

/-- A frame is ready iff its header and body are admitted and the buffer holds
the terminator and the whole body — so the body slice is in bounds. -/
theorem frame_ready_spec (header_end length buffered : Usize) :
    frame.frame_ready header_end length buffered ⦃ r =>
      r ↔ (header_end.val + 4 ≤ 8192 ∧ length.val ≤ 1048576 ∧
           header_end.val + 4 + length.val ≤ buffered.val) ⦄ := by
  unfold frame.frame_ready
  step with header_admitted_spec as ⟨ h, hh ⟩
  split_ifs with hhead
  · step with body_admitted_spec as ⟨ b, hb ⟩
    split_ifs with hbody
    · have h1 := hh.mp hhead
      have h2 := hb.mp hbody
      step as ⟨ start, hstart ⟩
      step as ⟨ stop, hstop ⟩
      simp only [decide_eq_true_eq]
      constructor
      · intro hge; exact ⟨ h1, h2, by scalar_tac ⟩
      · intro ⟨ _, _, hle ⟩; scalar_tac
    · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
      intro ⟨ _, h2, _ ⟩; exact hbody (hb.mpr h2)
  · simp only [WP.spec_ok, Bool.false_eq_true, false_iff]
    intro ⟨ h1, _, _ ⟩; exact hhead (hh.mpr h1)

end Strop.Kernel
