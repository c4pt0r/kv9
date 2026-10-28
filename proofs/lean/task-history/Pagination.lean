import Std

namespace Kv9.TaskHistory

/-- The ordered rows from one immutable snapshot plus a fixed overlay.
The Rust key cursor must refine `remaining`: it is not an offset API. -/
structure Scan (α : Type) where
  emitted : List α
  remaining : List α

def advance (width : Nat) (s : Scan α) : Scan α :=
  { emitted := s.emitted ++ s.remaining.take width
    remaining := s.remaining.drop width }

def run (width : Nat) : Nat → Scan α → Scan α
  | 0, s => s
  | n + 1, s => advance width (run width n s)

def CompletePrefix (snapshot : List α) (s : Scan α) : Prop :=
  s.emitted ++ s.remaining = snapshot

theorem initial_prefix (snapshot : List α) :
    CompletePrefix snapshot ⟨[], snapshot⟩ := by
  simp [CompletePrefix]

theorem advance_preserves_prefix (snapshot : List α) (width : Nat) (s : Scan α)
    (h : CompletePrefix snapshot s) : CompletePrefix snapshot (advance width s) := by
  simpa [CompletePrefix, advance, List.append_assoc] using h

theorem run_preserves_prefix (snapshot : List α) (width n : Nat) (s : Scan α)
    (h : CompletePrefix snapshot s) : CompletePrefix snapshot (run width n s) := by
  induction n with
  | zero => exact h
  | succ n ih => exact advance_preserves_prefix snapshot width (run width n s) ih

theorem page_bounded (width : Nat) (s : Scan α) :
    (s.remaining.take width).length ≤ width := by
  simp only [List.length_take]
  omega

theorem nonempty_advance_progress (width : Nat) (s : Scan α)
    (positive : 0 < width) (pending : 0 < s.remaining.length) :
    (advance width s).remaining.length < s.remaining.length := by
  simp only [advance, List.length_drop]
  omega

theorem run_suffix (width n : Nat) (s : Scan α) :
    (run width n s).remaining = s.remaining.drop (n * width) := by
  induction n with
  | zero => simp [run]
  | succ n ih => simp [run, advance, ih, List.drop_drop, Nat.add_mul]

theorem finite_exhaustion (width : Nat) (s : Scan α) (positive : 0 < width) :
    (run width s.remaining.length s).remaining = [] := by
  rw [run_suffix]
  apply List.drop_eq_nil_iff.mpr
  exact Nat.le_mul_of_pos_right s.remaining.length positive

theorem complete_history (snapshot : List α) (width : Nat) (positive : 0 < width) :
    (run width snapshot.length ⟨[], snapshot⟩).emitted = snapshot := by
  have h := run_preserves_prefix snapshot width snapshot.length ⟨[], snapshot⟩ (initial_prefix snapshot)
  have exhausted := finite_exhaustion width (Scan.mk [] snapshot) positive
  simpa [CompletePrefix, exhausted] using h

theorem lookup_preserved (snapshot : List α) (width : Nat) (positive : 0 < width)
    (predicate : α → Bool) :
    (run width snapshot.length ⟨[], snapshot⟩).emitted.find? predicate = snapshot.find? predicate := by
  rw [complete_history snapshot width positive]

theorem full_final_page_exhausts (snapshot : List α) (width : Nat)
    (full : snapshot.length = width) :
    (advance width ⟨[], snapshot⟩).remaining = [] := by
  simp [advance, ← full]

end Kv9.TaskHistory
