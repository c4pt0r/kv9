-- The checker prepends the exact, source-bound Interleave.lean kernel model.
namespace Kv9.FnvWriter

abbrev Body := List FnvInterleave.Octet
def budget : Nat := 65536
def bodyBytes (bodies : List Body) : Nat := (bodies.map List.length).sum

structure WriterState where
  written : List Body
  pending : List Body

def view (s : WriterState) : List Body := s.written ++ s.pending
def flush (s : WriterState) : WriterState := ⟨view s, []⟩
def push (s : WriterState) (b : Body) : WriterState := ⟨s.written, s.pending ++ [b]⟩
def fits (s : WriterState) (b : Body) : Prop :=
  s.pending.length < 4 ∧ bodyBytes s.pending + b.length ≤ budget
instance (s : WriterState) (b : Body) : Decidable (fits s b) :=
  inferInstanceAs (Decidable (_ ∧ _))

def finishGroup (s : WriterState) : WriterState :=
  if s.pending.length = 4 then flush s else s

def stage (s : WriterState) (b : Body) : WriterState :=
  if b.length > budget then ⟨view s ++ [b], []⟩
  else
    let ready := if fits s b then s else flush s
    finishGroup (push ready b)

def bounded (s : WriterState) : Prop :=
  s.pending.length < 4 ∧ bodyBytes s.pending ≤ budget

theorem flush_view (s : WriterState) : view (flush s) = view s := by
  simp [view, flush]

theorem push_view (s : WriterState) (b : Body) : view (push s b) = view s ++ [b] := by
  simp [view, push, List.append_assoc]

theorem finishGroup_view (s : WriterState) : view (finishGroup s) = view s := by
  unfold finishGroup
  split
  · exact flush_view s
  · rfl

theorem stage_view (s : WriterState) (b : Body) : view (stage s b) = view s ++ [b] := by
  unfold stage
  split
  · simp [view]
  · rw [finishGroup_view, push_view]
    congr 1
    split
    · rfl
    · exact flush_view s

theorem fold_view (bodies : List Body) (s : WriterState) :
    view (bodies.foldl stage s) = view s ++ bodies := by
  induction bodies generalizing s with
  | nil => simp
  | cons b bs ih => simp only [List.foldl_cons, ih, stage_view]; simp [List.append_assoc]

theorem complete_stream (bodies : List Body) :
    (flush (bodies.foldl stage ⟨[], []⟩)).written = bodies := by
  simpa [flush, view] using fold_view bodies ⟨[], []⟩

theorem flush_bound (s : WriterState) : bounded (flush s) := by
  simp [bounded, flush, bodyBytes, budget]

theorem push_bound (s : WriterState) (b : Body) (h : fits s b) :
    (push s b).pending.length ≤ 4 ∧ bodyBytes (push s b).pending ≤ budget := by
  constructor
  · simp only [push, List.length_append, List.length_singleton]
    have := h.1
    omega
  · simpa [push, bodyBytes, List.map_append, List.sum_append] using h.2

theorem finishGroup_bound (s : WriterState)
    (h : s.pending.length ≤ 4 ∧ bodyBytes s.pending ≤ budget) : bounded (finishGroup s) := by
  unfold finishGroup
  split
  · exact flush_bound s
  · simp only [bounded]; constructor <;> omega

theorem stage_bound (s : WriterState) (b : Body) : bounded (stage s b) := by
  unfold stage
  split
  · simp [bounded, bodyBytes, budget]
  · rename_i small
    have valid : fits (if fits s b then s else flush s) b := by
      split
      · assumption
      · simp [fits, flush, bodyBytes]; omega
    exact finishGroup_bound _ (push_bound _ b valid)

theorem fold_bound (bodies : List Body) (s : WriterState) (h : bounded s) :
    bounded (bodies.foldl stage s) := by
  induction bodies generalizing s with
  | nil => exact h
  | cons b bs ih => exact ih (stage s b) (stage_bound s b)

/-- Matches Rust's subtraction guard without overflowing payload + 1. -/
theorem payload_guard (used payload : Nat) (h : used ≤ budget) :
    payload < budget - used ↔ used + (payload + 1) ≤ budget := by
  omega

/-- Failed encoding can leave pending bodies unwritten, never reordered. -/
theorem written_prefix (s : WriterState) : s.written.IsPrefix (view s) := by
  exact ⟨s.pending, rfl⟩

/-- A short write after complete frames leaves a prefix of the canonical bytes. -/
theorem partial_frame_prefix (done rest : List Body) (current : Body) (n : Nat) :
    (done.flatten ++ current.take n).IsPrefix ((done ++ current :: rest).flatten) := by
  refine ⟨current.drop n ++ rest.flatten, ?_⟩
  simp only [List.flatten_append, List.flatten_cons, List.append_assoc]
  rw [← List.append_assoc (current.take n) (current.drop n), List.take_append_drop]

def scalarHash (b : Body) := FnvInterleave.scalar FnvInterleave.fnvStep b FnvInterleave.initial

/-- Any deterministic existing length/checksum/body encoding is preserved. -/
theorem four_frames (encode : Body → FnvInterleave.Word → Body) (a b c d : Body) :
    let sums := FnvInterleave.checksum4 a b c d
    [encode a sums.1, encode b sums.2.1, encode c sums.2.2.1, encode d sums.2.2.2] =
      [encode a (scalarHash a), encode b (scalarHash b),
       encode c (scalarHash c), encode d (scalarHash d)] := by
  simp only [FnvInterleave.checksum_equivalence, scalarHash]

end Kv9.FnvWriter
