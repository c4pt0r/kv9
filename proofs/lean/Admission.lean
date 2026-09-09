import Std

namespace Kv9.Admission

/-- Each list entry is an owned reservation's encoded request weight. Entries
remain present while reserved, queued or executing, including after RPC cancel. -/
def Within (requests bytes : Nat) (live : List Nat) : Prop :=
  live.length ≤ requests ∧ live.sum ≤ bytes

/-- The runtime's checked subtraction is safe under the pre-state invariant. -/
def Allowed (requests bytes : Nat) (live : List Nat) (weight : Nat) : Prop :=
  weight ≤ bytes ∧ live.length < requests ∧ weight ≤ bytes - live.sum

theorem empty_within (requests bytes : Nat) : Within requests bytes [] := by
  simp [Within]

/-- The machine additions cannot exceed their configured representable limits. -/
theorem reserve_preserves_bounds (requests bytes weight : Nat) (live : List Nat)
    (before : Within requests bytes live) (allowed : Allowed requests bytes live weight) :
    Within requests bytes (weight :: live) := by
  simp only [Within, List.length_cons, List.sum_cons] at *
  simp only [Allowed] at allowed
  omega

/-- Count and weight accounting is exact, independent of saturating telemetry. -/
theorem reserve_conserves (weight : Nat) (live : List Nat) :
    (weight :: live).length = live.length + 1 ∧
    (weight :: live).sum = live.sum + weight := by
  simp [Nat.add_comm]

/-- Release removes exactly one owned entry; equal weights are still distinct
list positions. There is no operation that releases a non-existent owner. -/
theorem release_conserves (weight : Nat) (left right : List Nat) :
    (left ++ weight :: right).length = (left ++ right).length + 1 ∧
    (left ++ weight :: right).sum = (left ++ right).sum + weight := by
  simp [List.sum_append, Nat.add_assoc, Nat.add_left_comm, Nat.add_comm]

theorem release_preserves_bounds (requests bytes weight : Nat) (left right : List Nat)
    (before : Within requests bytes (left ++ weight :: right)) :
    Within requests bytes (left ++ right) := by
  obtain ⟨count, payload⟩ := release_conserves weight left right
  simp only [Within] at *
  omega

/-- Cancel after submission, queue/start transitions, and refusals are stutters.
Only dropping an actual owner (completion, unwind or pre-submission cancellation)
may use release. This is a Rust ownership refinement obligation, not a scheduler
or whole-process-memory proof. -/
inductive Step (requests bytes : Nat) : List Nat → List Nat → Prop
  | reserve (live : List Nat) (weight : Nat) (allowed : Allowed requests bytes live weight) :
      Step requests bytes live (weight :: live)
  | release (left right : List Nat) (weight : Nat) :
      Step requests bytes (left ++ weight :: right) (left ++ right)
  | stutter (live : List Nat) : Step requests bytes live live

theorem step_preserves_bounds (requests bytes : Nat) (before after : List Nat)
    (bounded : Within requests bytes before) (step : Step requests bytes before after) :
    Within requests bytes after := by
  cases step with
  | reserve live weight allowed => exact reserve_preserves_bounds requests bytes weight _ bounded allowed
  | release left right weight => exact release_preserves_bounds requests bytes weight left right bounded
  | stutter live => exact bounded

inductive Reachable (requests bytes : Nat) : List Nat → Prop
  | initial : Reachable requests bytes []
  | next (before after : List Nat) (reachable : Reachable requests bytes before)
      (step : Step requests bytes before after) : Reachable requests bytes after

/-- An unbounded induction over arbitrary finite resource traces. -/
theorem reachable_within (requests bytes : Nat) (live : List Nat)
    (reachable : Reachable requests bytes live) : Within requests bytes live := by
  induction reachable with
  | initial => exact empty_within requests bytes
  | next before after _ step ih => exact step_preserves_bounds requests bytes before after ih step

end Kv9.Admission
