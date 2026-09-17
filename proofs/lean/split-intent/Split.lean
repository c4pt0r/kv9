import Std

namespace Kv9.SplitIntent

/-- One parent range's committed split intent. Catalog row immutability and
the single-range routing model are premises from the data-range model. An
intent requires a BOUND, unsealed parent and two activated, unbound child
groups on the parent's exact replica set; it is one per parent and
permanent; and it alone seals, populates, republishes and reroutes
nothing — every one of those is a later increment's step, absent here. -/
structure State where
  parentBound : Bool := false
  parentSealed : Bool := false
  childrenActivated : Bool := false
  childrenBound : Bool := false
  intent : Bool := false
  republished : Bool := false
  rerouted : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | bindParent {s} : Step s {s with parentBound := true}
  | activateChildren {s} : Step s {s with childrenActivated := true}
  | record {s} (p : s.parentBound = true) (u : s.parentSealed = false)
      (a : s.childrenActivated = true) (f : s.childrenBound = false)
      (once : s.intent = false) :
      Step s {s with intent := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.intent = true →
    s.parentBound = true ∧ s.childrenActivated = true ∧ s.childrenBound = false) ∧
  s.parentSealed = false ∧
  s.republished = false ∧
  s.rerouted = false ∧
  s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hI, hS, hP, hR, hV⟩ := safe
  cases step with
  | bindParent => exact ⟨by simp_all, hS, hP, hR, hV⟩
  | activateChildren => exact ⟨by simp_all, hS, hP, hR, hV⟩
  | record p u a f once => exact ⟨by simp_all, hS, hP, hR, hV⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem an_intent_requires_the_bound_parent_and_ready_children {s : State}
    (run : Reachable s) (recorded : s.intent = true) :
    s.parentBound = true ∧ s.childrenActivated = true ∧
      s.childrenBound = false :=
  (run_safe run).1 recorded

theorem nothing_seals_here {s : State} (run : Reachable s) :
    s.parentSealed = false :=
  (run_safe run).2.1

theorem nothing_republishes_or_reroutes_here {s : State} (run : Reachable s) :
    s.republished = false ∧ s.rerouted = false :=
  ⟨(run_safe run).2.2.1, (run_safe run).2.2.2.1⟩

theorem no_serving_capability {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2.2.2

theorem an_intent_is_permanent {s t : State} (step : Step s t)
    (recorded : s.intent = true) : t.intent = true := by
  cases step <;> simp_all

theorem one_intent_per_parent {s t : State} (step : Step s t)
    (recorded : s.intent = true) : t = s ∨ t.intent = true := by
  cases step <;> simp_all

theorem readiness_alone_records_nothing :
    ∃ s, Reachable s ∧ s.parentBound = true ∧ s.childrenActivated = true ∧
      s.intent = false := by
  refine ⟨⟨true, false, true, false, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step .initial .bindParent) .activateChildren

theorem the_committed_chain_reaches_the_intent :
    ∃ s, Reachable s ∧ s.intent = true ∧ s.parentSealed = false ∧
      s.rerouted = false := by
  refine ⟨⟨true, false, true, false, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .bindParent) .activateChildren)
    (.record rfl rfl rfl rfl rfl)

end Kv9.SplitIntent
