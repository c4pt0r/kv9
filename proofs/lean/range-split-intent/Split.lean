import Std

namespace Kv9.RangeSplitIntent

/-- One parent range and its split intent as the catalog sees it (docs/RANGE-SPLIT.md §2).
`bound` says the parent has a committed, unsealed binding; `keyInside` that the split key
falls strictly inside it; `childrenOnMembers` that both children live on the parent's
committed members; `intents` counts committed intents for this parent; `sealed`,
`published` and `rerouted` are the later parts, never touched here; `serving` says the
parent serves. -/
structure State where
  bound : Bool := true
  keyInside : Bool := true
  childrenOnMembers : Bool := true
  intents : Nat := 0
  childrenReady : Bool := false
  sealed : Bool := false
  published : Bool := false
  rerouted : Bool := false
  serving : Bool := true
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | unbind {s} : Step s {s with bound := false}
  | keyOutside {s} : Step s {s with keyInside := false}
  | keyIn {s} : Step s {s with keyInside := true}
  | childrenElsewhere {s} : Step s {s with childrenOnMembers := false}
  /-- The intent: only on a bound, unsealed parent, only with a key inside, only with
  the children on the parent's members, at most once per parent (a repeat confirms).
  It creates and activates the children; it seals, publishes and reroutes nothing. -/
  | intend {s} (b : s.bound = true) (u : s.sealed = false) (k : s.keyInside = true)
      (m : s.childrenOnMembers = true) :
      Step s {s with intents := if s.intents = 0 then 1 else s.intents, childrenReady := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.intents ≤ 1 ∧ s.sealed = false ∧ s.published = false ∧ s.rerouted = false ∧ s.serving = true

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hI, hS, hP, hR, hV⟩ := safe
  cases step with
  | unbind => exact ⟨hI, hS, hP, hR, hV⟩
  | keyOutside => exact ⟨hI, hS, hP, hR, hV⟩
  | keyIn => exact ⟨hI, hS, hP, hR, hV⟩
  | childrenElsewhere => exact ⟨hI, hS, hP, hR, hV⟩
  | intend b u k m =>
      refine ⟨?_, hS, hP, hR, hV⟩
      show (if s.intents = 0 then 1 else s.intents) ≤ 1
      split <;> omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- S1: one intent per parent, permanent.
theorem one_intent_per_parent {s : State} (run : Reachable s) : s.intents ≤ 1 :=
  (run_safe run).1

theorem a_repeated_intent_confirms {s t : State} (step : Step s t) (had : s.intents = 1) :
    t.intents = 1 := by
  cases step <;> simp_all

theorem an_intent_is_permanent {s t : State} (step : Step s t) (had : s.intents = 1) :
    t.intents = 1 :=
  a_repeated_intent_confirms step had

-- S2 and the planner's guards: an intent needs a bound unsealed parent, a key inside
-- and children on the parent's members.
theorem an_intent_needs_a_bound_parent_a_key_inside_and_the_members {s t : State}
    (step : Step s t) (fresh : s.intents = 0) (intended : t.intents = 1) :
    s.bound = true ∧ s.sealed = false ∧ s.keyInside = true ∧ s.childrenOnMembers = true := by
  cases step <;> simp_all

-- S3: part 1 seals, publishes and reroutes nothing; the parent keeps serving.
theorem nothing_seals_publishes_or_reroutes_here {s : State} (run : Reachable s) :
    s.sealed = false ∧ s.published = false ∧ s.rerouted = false ∧ s.serving = true :=
  (run_safe run).2

-- An intent leaves the children created and activated.
theorem an_intent_readies_the_children {s t : State} (step : Step s t)
    (intended : s.intents < t.intents) : t.childrenReady = true := by
  cases step <;> simp_all

-- The committed chain reaches the intent.
theorem the_chain_reaches_an_intent :
    ∃ s, Reachable s ∧ s.intents = 1 ∧ s.childrenReady = true ∧ s.serving = true := by
  refine ⟨⟨true, true, true, 1, true, false, false, false, true⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial (.intend rfl rfl rfl rfl)

end Kv9.RangeSplitIntent
