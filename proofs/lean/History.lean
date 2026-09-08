import Std

namespace Kv9.History

/-- A certificate expands both effects and observed-response checks into
steps. An unsuccessful model transition rejects the certificate. -/
def verify (transition : State → Action → Option State)
    (allowed : Nat → Action → Bool) : Nat → State → List (Nat × Action) → Bool
  | _, _, [] => true
  | previous, state, (slot, action) :: rest =>
    if previous ≤ slot && allowed slot action then
      match transition state action with
      | none => false
      | some next => verify transition allowed slot next rest
    else false

/-- Logical semantics independent of the Boolean certificate evaluator.
`allowed` represents invocation bounds and operation-specific eligibility;
response actions check the corresponding completed observation in `transition`. -/
inductive Execution (transition : State → Action → Option State)
    (allowed : Nat → Action → Bool) : Nat → State → List (Nat × Action) → Prop where
  | empty : Execution transition allowed previous state []
  | next (ordered : previous ≤ slot) (eligible : allowed slot action = true)
      (effect : transition state action = some successor)
      (tail : Execution transition allowed slot successor rest) :
      Execution transition allowed previous state ((slot, action) :: rest)

theorem accepted_certificate_is_execution
    (accepted : verify transition allowed previous state certificate = true) :
    Execution transition allowed previous state certificate := by
  induction certificate generalizing previous state with
  | nil => exact Execution.empty
  | cons point rest ih =>
    obtain ⟨slot, action⟩ := point
    simp only [verify] at accepted
    split at accepted
    next guard =>
      have guards : previous ≤ slot ∧ allowed slot action = true := by simpa using guard
      have ordered : previous ≤ slot := guards.1
      split at accepted
      next none => contradiction
      next successor effect =>
        exact Execution.next ordered guards.2 effect (ih accepted)
    next rejected => contradiction

/-- Slot bounds imply the real-time constraint for every pair of completed
atomic operations, without assuming a bound on history length or concurrency.
Unknown writes have no response upper bound and are not included as `left`. -/
theorem completed_before_invoked {leftSlot leftResponse rightInvocation rightSlot : Nat} (leftBound : leftSlot ≤ leftResponse)
    (before : leftResponse < rightInvocation) (rightBound : rightInvocation ≤ rightSlot) :
    leftSlot < rightSlot := by
  omega

/-- A whole accepted certificate contains a legal model execution. The model
relation and its response predicates remain explicit refinement obligations. -/
theorem execution_is_accepted
    (execution : Execution transition allowed previous state certificate) :
    verify transition allowed previous state certificate = true := by
  induction execution with
  | empty => rfl
  | next ordered eligible effect tail ih =>
    simp [verify, ordered, eligible, effect, ih]

end Kv9.History
