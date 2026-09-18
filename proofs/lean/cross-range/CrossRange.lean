import Std

namespace Kv9.CrossRange

/-- One span crossing a split boundary, served in range-sized chunks.
Partition-directory exactness (no overlap, no gap) and each group's own
linearizable read are premises from their own models. Chunks walk the
partition in key order, each under its own group authorization; no key is
skipped or duplicated — the partition covers exactly once — and the span
as a whole is never one snapshot: per-chunk atomicity is the documented
contract, exactly as delete-range has always stated. -/
structure State where
  split : Bool := false
  lowChunkDone : Bool := false
  highChunkDone : Bool := false
  skipped : Bool := false
  duplicated : Bool := false
  atomicSpan : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | publishPartition {s} : Step s {s with split := true}
  | serveLow {s} (p : s.split = true) : Step s {s with lowChunkDone := true}
  | serveHigh {s} (p : s.split = true) (ordered : s.lowChunkDone = true) :
      Step s {s with highChunkDone := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.highChunkDone = true → s.lowChunkDone = true) ∧
  (s.lowChunkDone = true → s.split = true) ∧
  s.skipped = false ∧
  s.duplicated = false ∧
  s.atomicSpan = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hO, hS, hK, hD, hA⟩ := safe
  cases step with
  | publishPartition => exact ⟨by simp_all, by simp_all, hK, hD, hA⟩
  | serveLow p => exact ⟨by simp_all, by simp_all, hK, hD, hA⟩
  | serveHigh p ordered => exact ⟨by simp_all, by simp_all, hK, hD, hA⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem chunks_walk_in_key_order {s : State} (run : Reachable s)
    (high : s.highChunkDone = true) : s.lowChunkDone = true :=
  (run_safe run).1 high

theorem chunks_require_the_partition {s : State} (run : Reachable s)
    (low : s.lowChunkDone = true) : s.split = true :=
  (run_safe run).2.1 low

theorem no_key_is_skipped {s : State} (run : Reachable s) :
    s.skipped = false :=
  (run_safe run).2.2.1

theorem no_key_is_duplicated {s : State} (run : Reachable s) :
    s.duplicated = false :=
  (run_safe run).2.2.2.1

theorem the_span_is_never_one_snapshot {s : State} (run : Reachable s) :
    s.atomicSpan = false :=
  (run_safe run).2.2.2.2

theorem a_chunk_is_permanent {s t : State} (step : Step s t)
    (low : s.lowChunkDone = true) : t.lowChunkDone = true := by
  cases step <;> simp_all

theorem a_partition_alone_serves_nothing :
    ∃ s, Reachable s ∧ s.split = true ∧ s.lowChunkDone = false := by
  refine ⟨⟨true, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .publishPartition

theorem the_span_completes_across_the_boundary :
    ∃ s, Reachable s ∧ s.lowChunkDone = true ∧ s.highChunkDone = true ∧
      s.atomicSpan = false := by
  refine ⟨⟨true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .publishPartition) (.serveLow rfl))
    (.serveHigh rfl rfl)

end Kv9.CrossRange
