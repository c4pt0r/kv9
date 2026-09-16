import Std

namespace Kv9.GroupControl

/-- One operation's exact root/operation/group/replica binding is abstracted by
a natural number. Equality means equality of the entire immutable encoding. -/
structure State where
  creation : Option Nat := none
  desire : Option Nat := none
  staged : Option Nat := none
  running : Bool := false
  failed : Bool := false
  acknowledged : Bool := false
  deriving DecidableEq, Repr

inductive Step (localBinding : Nat) : State → State → Prop where
  | prepare {s} (binding : Nat) (empty : s.creation = none) :
      Step localBinding s {s with creation := some binding}
  | plan {s} (binding : Nat) :
      Step localBinding s {s with staged := some binding}
  | discard {s} : Step localBinding s {s with staged := none}
  | commit {s} (binding : Nat) (planned : s.staged = some binding)
      (compatible : s.creation = none ∨ s.creation = some binding) :
      Step localBinding s {s with creation := some binding, desire := some binding, staged := none, acknowledged := true}
  | activate {s} (committed : s.desire = some localBinding)
      (healthy : s.failed = false) : Step localBinding s {s with running := true}
  | fail {s} : Step localBinding s {s with running := false, failed := true}
  | restart {s} : Step localBinding s {s with running := false, failed := false, staged := none, acknowledged := false}
  | idle {s} : Step localBinding s s

def Safe (localBinding : Nat) (s : State) : Prop :=
  (∀ binding, s.desire = some binding → s.creation = some binding) ∧
  (s.running = true → s.desire = some localBinding ∧ s.failed = false) ∧
  (s.acknowledged = true → s.desire ≠ none)

theorem initial_safe (localBinding : Nat) : Safe localBinding {} := by simp [Safe]

theorem step_safe (localBinding : Nat) {s t : State}
    (safe : Safe localBinding s) (step : Step localBinding s t) : Safe localBinding t := by
  cases step with
  | commit binding _ compatible =>
    obtain ⟨pair, owner, _⟩ := safe
    refine ⟨by simp, ?_, by simp⟩
    intro running
    obtain ⟨desire, healthy⟩ := owner running
    have creation := pair localBinding desire
    rcases compatible with empty | same <;> simp_all
  | _ => simp_all [Safe]

inductive Reachable (localBinding : Nat) : State → Prop where
  | initial : Reachable localBinding {}
  | step {s t} : Reachable localBinding s → Step localBinding s t → Reachable localBinding t

theorem run_safe (localBinding : Nat) {s : State} (run : Reachable localBinding s) : Safe localBinding s := by
  induction run with
  | initial => exact initial_safe localBinding
  | step _ transition ih => exact step_safe localBinding ih transition

theorem automatic_start_requires_exact_committed_pair (localBinding : Nat) {s : State}
    (run : Reachable localBinding s) (running : s.running = true) :
    s.creation = some localBinding ∧ s.desire = some localBinding ∧ s.failed = false := by
  obtain ⟨pair, owner, _⟩ := run_safe localBinding run
  obtain ⟨desire, healthy⟩ := owner running
  exact ⟨pair localBinding desire, desire, healthy⟩

theorem preparation_alone_cannot_run (localBinding : Nat) {s : State}
    (run : Reachable localBinding s) (noDesire : s.desire = none) : s.running = false := by
  have safe := run_safe localBinding run
  cases h : s.running <;> simp_all [Safe]

theorem staging_cannot_publish_desire (s : State) (binding : Nat) :
    ({s with staged := some binding} : State).desire = s.desire := rfl

theorem commit_preserves_existing_creation (localBinding : Nat) {s t : State}
    (step : Step localBinding s t)
    (binding : Nat) (existing : s.creation = some binding) : t.creation = some binding := by
  cases step <;> simp_all

theorem committed_desire_survives_restart (s : State) :
    ({s with running := false, failed := false, staged := none, acknowledged := false} : State).desire = s.desire := rfl

theorem desire_is_monotone (localBinding : Nat) {s t : State}
    (safe : Safe localBinding s) (step : Step localBinding s t)
    (binding : Nat) (existing : s.desire = some binding) : t.desire = some binding := by
  have creation := safe.1 binding existing
  cases step <;> simp_all

theorem acknowledgment_is_not_readiness :
    ∃ s, Reachable 7 s ∧ s.acknowledged = true ∧ s.running = false := by
  let planned : State := {staged := some 7}
  let committed : State := {creation := some 7, desire := some 7, acknowledged := true}
  refine ⟨committed, ?_, rfl, rfl⟩
  have p : Reachable 7 planned := .step .initial (.plan 7)
  exact .step p (.commit 7 rfl (Or.inl rfl))

/-- One turn selects the first eligible immutable desire. A failed or running
slot is skipped, and the iteration ends after this single attempt. -/
structure Candidate where
  exactLocalStore : Bool
  running : Bool
  failed : Bool
  deriving DecidableEq, Repr

def eligible (c : Candidate) : Bool := c.exactLocalStore && !c.running && !c.failed
def select : List Candidate → List Candidate
  | [] => []
  | c :: cs => if eligible c then [c] else select cs

theorem one_attempt_per_turn (cs : List Candidate) : (select cs).length ≤ 1 := by
  induction cs with
  | nil => simp [select]
  | cons c cs ih => simp only [select]; split <;> simp_all

theorem selected_has_exact_identity_and_is_not_failed (cs : List Candidate)
    (c : Candidate) (chosen : c ∈ select cs) :
    c.exactLocalStore = true ∧ c.running = false ∧ c.failed = false := by
  induction cs with
  | nil => simp [select] at chosen
  | cons head tail ih =>
    simp only [select] at chosen
    split at chosen
    next h =>
      have eq : c = head := by simpa using chosen
      subst c
      simpa [eligible, and_assoc] using h
    next _ => exact ih chosen

end Kv9.GroupControl
