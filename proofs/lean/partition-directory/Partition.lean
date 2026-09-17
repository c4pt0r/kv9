import Std

namespace Kv9.PartitionDirectory

/-- One keyspace's range directory across a split, as the generalized read
model validates it. Catalog atomicity is a premise from the metadata Raft
model; the split intent's authority from the split-intent model. The
directory is either the legacy single unsealed full range or the split
shape — sealed parent with BOTH children covering exactly — and the ONLY
transition between them is one atomic publication. No reachable state is
uncovered, partially published, or routed through a sealed range. -/
structure State where
  legacy : Bool := true
  split : Bool := false
  uncovered : Bool := false
  partialPublish : Bool := false
  routedSealed : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | publish {s} (l : s.legacy = true) :
      Step s {s with legacy := false, split := true}
  | route {s} (covered : s.legacy = true ∨ s.split = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.legacy = true ∨ s.split = true) ∧
  ¬(s.legacy = true ∧ s.split = true) ∧
  s.uncovered = false ∧
  s.partialPublish = false ∧
  s.routedSealed = false ∧
  s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hC, hX, hU, hP, hR, hV⟩ := safe
  cases step with
  | publish l => exact ⟨by simp_all, by simp_all, hU, hP, hR, hV⟩
  | route covered => exact ⟨hC, hX, hU, hP, hR, hV⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem coverage_is_never_lost {s : State} (run : Reachable s) :
    s.legacy = true ∨ s.split = true :=
  (run_safe run).1

theorem the_directory_is_never_in_both_shapes {s : State} (run : Reachable s) :
    ¬(s.legacy = true ∧ s.split = true) :=
  (run_safe run).2.1

theorem no_partial_publication_exists {s : State} (run : Reachable s) :
    s.uncovered = false ∧ s.partialPublish = false :=
  ⟨(run_safe run).2.2.1, (run_safe run).2.2.2.1⟩

theorem sealed_ranges_never_route {s : State} (run : Reachable s) :
    s.routedSealed = false :=
  (run_safe run).2.2.2.2.1

theorem no_serving_capability_is_minted {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2.2.2.2

theorem a_split_directory_is_permanent {s t : State} (step : Step s t)
    (published : s.split = true) : t.split = true := by
  cases step <;> simp_all

theorem routing_is_always_possible {s : State} (run : Reachable s) :
    Step s s :=
  .route (coverage_is_never_lost run)

theorem the_atomic_publication_reaches_the_split_shape :
    ∃ s, Reachable s ∧ s.split = true ∧ s.legacy = false ∧
      s.partialPublish = false := by
  refine ⟨⟨false, true, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial (.publish rfl)

end Kv9.PartitionDirectory
