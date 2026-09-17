import Std

namespace Kv9.StorageRetirement

/-- One removed replica's local fate. The removal decision's own authority
chain (evidence, quorum floor, never-the-destination) is a premise from the
replica-removal model; here the decision that names THIS exact store is the
one key that fences the local replica. Retirement stops the replica and is
permanent and durable; storage is never deleted and nothing ever serves. -/
structure State where
  removalCommitted : Bool := false
  running : Bool := false
  retired : Bool := false
  storageDeleted : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with removalCommitted := true}
  | run {s} (fresh : s.retired = false) : Step s {s with running := true}
  | retire {s} (d : s.removalCommitted = true) :
      Step s {s with retired := true, running := false}
  | restartRetired {s} (r : s.retired = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.retired = true → s.removalCommitted = true) ∧
  ¬(s.retired = true ∧ s.running = true) ∧
  s.storageDeleted = false ∧
  s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hC, hX, hD, hS⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, by simp_all, hD, hS⟩
  | run fresh => exact ⟨by simp_all, by simp_all, hD, hS⟩
  | retire d => exact ⟨by simp_all, by simp_all, hD, hS⟩
  | restartRetired r => exact ⟨hC, hX, hD, hS⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem retirement_requires_the_committed_removal {s : State}
    (run : Reachable s) (retired : s.retired = true) :
    s.removalCommitted = true :=
  (run_safe run).1 retired

theorem a_retired_replica_never_runs {s : State} (run : Reachable s) :
    ¬(s.retired = true ∧ s.running = true) :=
  (run_safe run).2.1

theorem storage_is_never_deleted {s : State} (run : Reachable s) :
    s.storageDeleted = false :=
  (run_safe run).2.2.1

theorem no_serving_capability {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2.2

theorem retirement_is_permanent {s t : State} (step : Step s t)
    (retired : s.retired = true) : t.retired = true := by
  cases step <;> simp_all

theorem restart_preserves_retirement {s : State} (retired : s.retired = true) :
    Step s s :=
  .restartRetired retired

theorem a_removal_alone_retires_nothing :
    ∃ s, Reachable s ∧ s.removalCommitted = true ∧ s.retired = false := by
  refine ⟨⟨true, true, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial (.run rfl)) .commit

theorem the_committed_chain_reaches_retirement :
    ∃ s, Reachable s ∧ s.retired = true ∧ s.running = false ∧
      s.storageDeleted = false := by
  refine ⟨⟨true, false, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial (.run rfl)) .commit) (.retire rfl)

end Kv9.StorageRetirement
