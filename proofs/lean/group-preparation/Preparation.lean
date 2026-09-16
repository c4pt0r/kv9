import Std

namespace Kv9.GroupPreparation

/-- Durable local phases. Neither phase grants voting or routing authority. -/
inductive Phase where
  | absent | intent | ready
  deriving DecidableEq, Repr

structure State where
  phase : Phase := .absent
  binding : Option Nat := none
  raftDurable : Bool := false
  engineDurable : Bool := false
  failed : Bool := false
  reported : Bool := false
  deriving DecidableEq, Repr

/-- `authority` abstracts the exact committed root/task/group/store binding.
Transitions occur at successful durable operations. A failed operation has no
successful return; recovery must synchronize and validate surviving state.
The filesystem atomic-publication contract and committed metadata prefix are
explicit premises of the implementation refinement, not proved here. -/
inductive Step (authority : Nat) : State → State → Prop where
  | intent {s} (absent : s.phase = .absent) (healthy : s.failed = false) :
      Step authority s {s with phase := .intent, binding := some authority}
  | raft {s} (prepared : s.phase = .intent) (healthy : s.failed = false) :
      Step authority s {s with raftDurable := true}
  | engine {s} (prepared : s.phase = .intent) (healthy : s.failed = false) :
      Step authority s {s with engineDurable := true}
  | ready {s} (prepared : s.phase = .intent) (healthy : s.failed = false)
      (raft : s.raftDurable = true) (engine : s.engineDurable = true) :
      Step authority s {s with phase := .ready}
  | report {s} (ready : s.phase = .ready) (healthy : s.failed = false) :
      Step authority s {s with reported := true}
  | fail {s} : Step authority s {s with failed := true, reported := false}
  | restart {s} : Step authority s {s with failed := false, reported := false}
  | idle {s} : Step authority s s

def Safe (authority : Nat) (s : State) : Prop :=
  (s.phase ≠ .absent → s.binding = some authority) ∧
  (s.raftDurable = true → s.phase ≠ .absent) ∧
  (s.engineDurable = true → s.phase ≠ .absent) ∧
  (s.phase = .ready → s.raftDurable = true ∧ s.engineDurable = true) ∧
  (s.reported = true → s.phase = .ready ∧ s.failed = false)

theorem initial_safe (authority : Nat) : Safe authority {} := by
  simp [Safe]

theorem step_preserves (authority : Nat) {before after : State}
    (safe : Safe authority before) (step : Step authority before after) :
    Safe authority after := by
  cases step <;> simp_all [Safe]

inductive Reachable (authority : Nat) : State → Prop where
  | initial : Reachable authority {}
  | step {before after} : Reachable authority before → Step authority before after → Reachable authority after

theorem reachable_safe (authority : Nat) {s : State} (run : Reachable authority s) :
    Safe authority s := by
  induction run with
  | initial => exact initial_safe authority
  | step _ transition ih => exact step_preserves authority ih transition

/-- Every returned preparation has the exact binding and both durable stores. -/
theorem report_requires_durable_binding (authority : Nat) {s : State}
    (run : Reachable authority s) (reported : s.reported = true) :
    s.binding = some authority ∧ s.raftDurable = true ∧
      s.engineDurable = true ∧ s.failed = false := by
  obtain ⟨binding, _, _, stores, reporting⟩ := reachable_safe authority run
  obtain ⟨phase, healthy⟩ := reporting reported
  have nonempty : s.phase ≠ .absent := by simp [phase]
  exact ⟨binding nonempty, (stores phase).1, (stores phase).2, healthy⟩

/-- Initialization can run only before StorageReady, never during recovery of it. -/
def mayInitialize (s : State) : Bool := s.phase == .intent && !s.failed

theorem ready_never_initializes (s : State) (ready : s.phase = .ready) :
    mayInitialize s = false := by simp [mayInitialize, ready]

theorem failed_never_initializes (s : State) (failed : s.failed = true) :
    mayInitialize s = false := by simp [mayInitialize, failed]

theorem failed_has_no_report (authority : Nat) {s : State}
    (run : Reachable authority s) (failed : s.failed = true) : s.reported = false := by
  have safe := reachable_safe authority run
  cases h : s.reported <;> simp_all [Safe]

/-- Durable readiness never moves back to a phase that may initialize a log. -/
theorem ready_is_monotone (authority : Nat) {before after : State}
    (step : Step authority before after) (ready : before.phase = .ready) :
    after.phase = .ready := by
  cases step <;> simp_all

/-- A restart discards the old in-process observation; it must be revalidated. -/
theorem restart_discards_report (s : State) :
    ({s with failed := false, reported := false} : State).reported = false := rfl

end Kv9.GroupPreparation
