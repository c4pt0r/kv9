import Std

namespace Kv9.ParentRetirement

/-- The sealed parent's fate after its split publishes. Publication
atomicity and the storage-retirement machinery are premises from their own
models. The PUBLISHED directory — sealed parent with covering children —
is the one authority that retires the local parent replica on every
hosting node; retirement is permanent and durable, storage is never
deleted, and the children's serving is never disturbed. -/
structure State where
  published : Bool := false
  parentRunning : Bool := true
  parentRetired : Bool := false
  storageDeleted : Bool := false
  childrenDisturbed : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | publish {s} : Step s {s with published := true}
  | retire {s} (p : s.published = true) :
      Step s {s with parentRetired := true, parentRunning := false}
  | restartRetired {s} (r : s.parentRetired = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.parentRetired = true → s.published = true) ∧
  ¬(s.parentRetired = true ∧ s.parentRunning = true) ∧
  s.storageDeleted = false ∧
  s.childrenDisturbed = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hX, hD, hC⟩ := safe
  cases step with
  | publish => exact ⟨by simp_all, by simp_all, hD, hC⟩
  | retire p => exact ⟨by simp_all, by simp_all, hD, hC⟩
  | restartRetired r => exact ⟨hP, hX, hD, hC⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem retirement_requires_the_published_split {s : State}
    (run : Reachable s) (retired : s.parentRetired = true) :
    s.published = true :=
  (run_safe run).1 retired

theorem a_retired_parent_never_runs {s : State} (run : Reachable s) :
    ¬(s.parentRetired = true ∧ s.parentRunning = true) :=
  (run_safe run).2.1

theorem storage_is_never_deleted {s : State} (run : Reachable s) :
    s.storageDeleted = false :=
  (run_safe run).2.2.1

theorem the_children_are_never_disturbed {s : State} (run : Reachable s) :
    s.childrenDisturbed = false :=
  (run_safe run).2.2.2

theorem retirement_is_permanent {s t : State} (step : Step s t)
    (retired : s.parentRetired = true) : t.parentRetired = true := by
  cases step <;> simp_all

theorem restart_preserves_retirement {s : State}
    (retired : s.parentRetired = true) : Step s s :=
  .restartRetired retired

theorem a_publication_alone_retires_nothing :
    ∃ s, Reachable s ∧ s.published = true ∧ s.parentRetired = false := by
  refine ⟨⟨true, true, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .publish

theorem the_published_chain_reaches_retirement :
    ∃ s, Reachable s ∧ s.parentRetired = true ∧ s.parentRunning = false ∧
      s.childrenDisturbed = false := by
  refine ⟨⟨true, false, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step .initial .publish) (.retire rfl)

end Kv9.ParentRetirement
