import Std

namespace Kv9.CheckpointIntent

/-- Arbitrary canonical bytes for one immutable operation. Atomic metadata
apply, collision-resistant identity binding and exact receipts are premises.
Global enumeration, publisher closure and GC are outside this component. -/
structure State (D : Type) where
  archive : Option D := none
  pinned : Bool := false
  confirmed : Bool := false
  used : Bool := false
  cleared : Bool := false

def Valid (canonical : D) (s : State D) : Prop :=
  (s.archive = none ∨ s.archive = some canonical) ∧
  ((s.confirmed = true ∨ s.used = true ∨ s.cleared = true) →
    s.archive = some canonical ∧ s.pinned = true)

inductive Step (canonical : D) : State D → State D → Prop where
  | protect (s) : Step canonical s {s with pinned := true}
  | record (s) (payload : D) (exact : payload = canonical)
      (covered : s.pinned = true) (empty : s.archive = none) :
      Step canonical s {s with archive := some payload}
  | receipt (s) (stored : s.archive = some canonical) (covered : s.pinned = true) :
      Step canonical s {s with confirmed := true}
  | use (s) (receipt : s.confirmed = true) : Step canonical s {s with used := true}
  | clear (s) (receipt : s.confirmed = true) : Step canonical s {s with cleared := true}
  | restart (s) : Step canonical s {s with confirmed := false}
  | unchanged (s) : Step canonical s s

theorem initial_valid (canonical : D) : Valid canonical ({} : State D) := by
  simp [Valid]

theorem step_preserves (h : Step canonical s t) (good : Valid canonical s) :
    Valid canonical t := by
  cases h <;> simp_all [Valid]

inductive Reachable (canonical : D) : State D → Prop where
  | initial : Reachable canonical {}
  | next (before : Reachable canonical s) (step : Step canonical s t) : Reachable canonical t

theorem reachable_valid (h : Reachable canonical s) : Valid canonical s := by
  induction h with
  | initial => exact initial_valid _
  | next _ step ih => exact step_preserves step ih

theorem clear_keeps_exact_intent (h : Reachable canonical s) (cleared : s.cleared = true) :
    s.archive = some canonical ∧ s.pinned = true := by
  exact (reachable_valid h).2 (Or.inr (Or.inr cleared))

theorem use_requires_archive (h : Reachable canonical s) (used : s.used = true) :
    s.archive = some canonical ∧ s.pinned = true := by
  exact (reachable_valid h).2 (Or.inr (Or.inl used))

theorem step_never_replaces_archive (h : Step canonical s t) (stored : s.archive = some d) :
    t.archive = some d := by
  cases h <;> simp_all

theorem restart_keeps_durable_projection (s : State D) :
    ({s with confirmed := false} : State D).archive = s.archive ∧
    ({s with confirmed := false} : State D).pinned = s.pinned := by
  exact ⟨rfl, rfl⟩

end Kv9.CheckpointIntent
