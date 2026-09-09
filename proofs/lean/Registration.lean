import Std

namespace Kv9.Registration

/-- The cursor update used after selecting the first seed of a retry pass. -/
def nextSeed (size cursor : Nat) : Nat :=
  if cursor + 1 = size then 0 else cursor + 1

/-- The branch implementation is cyclic advancement on valid cursors. -/
theorem next_seed_is_modulo (size cursor : Nat) (valid : cursor < size) :
    nextSeed size cursor = (cursor + 1) % size := by
  by_cases last : cursor + 1 = size
  · simp [nextSeed, last]
  · have before : cursor + 1 < size := by omega
    simp [nextSeed, last, Nat.mod_eq_of_lt before]

/-- Advancing a valid cursor cannot leave the seed range. -/
theorem next_seed_stays_in_range (size cursor : Nat) (valid : cursor < size) :
    nextSeed size cursor < size := by
  rw [next_seed_is_modulo size cursor valid]
  exact Nat.mod_lt _ (by omega)

/-- From any cursor, every seed is first within one cycle of retry passes. -/
theorem every_seed_gets_a_first_turn (size start target : Nat)
    (startValid : start < size) (targetValid : target < size) :
    ∃ offset, offset < size ∧ (start + offset) % size = target := by
  by_cases ahead : start ≤ target
  · refine ⟨target - start, by omega, ?_⟩
    have sum : start + (target - start) = target := by omega
    rw [sum, Nat.mod_eq_of_lt targetValid]
  · refine ⟨size - start + target, by omega, ?_⟩
    have sum : start + (size - start + target) = size + target := by omega
    rw [sum, Nat.add_mod]
    simp [Nat.mod_eq_of_lt targetValid]

/-- Actual repeated cursor updates, rather than a closed-form assumption. -/
def cursorAfter (size start : Nat) : Nat → Nat
  | 0 => start
  | passes + 1 => nextSeed size (cursorAfter size start passes)

/-- Induction connects the implementation step to its closed form. -/
theorem cursor_after_is_modulo (size start passes : Nat) (valid : start < size) :
    cursorAfter size start passes = (start + passes) % size := by
  induction passes with
  | zero => simp [cursorAfter, Nat.mod_eq_of_lt valid]
  | succ passes ih =>
    simp only [cursorAfter, ih]
    rw [next_seed_is_modulo size _ (Nat.mod_lt _ (by omega))]
    simp [Nat.add_assoc]

/-- Every seed is selected by the real step function within one cycle. -/
theorem repeated_steps_cover_every_seed (size start target : Nat)
    (startValid : start < size) (targetValid : target < size) :
    ∃ offset, offset < size ∧ cursorAfter size start offset = target := by
  obtain ⟨offset, bound, selected⟩ :=
    every_seed_gets_a_first_turn size start target startValid targetValid
  exact ⟨offset, bound, by rw [cursor_after_is_modulo size start offset startValid, selected]⟩

end Kv9.Registration
