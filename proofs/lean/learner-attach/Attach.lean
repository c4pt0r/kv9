import Std

namespace Kv9.LearnerAttach

/-- One destination's attachment to one source group, authorized by one
committed migration operation. Group-consensus atomicity of the
configuration entry and the capture model's cut semantics are premises from
their own models. The destination becomes a LEARNER only; promotion,
replication, installation and serving have no step here. -/
structure State where
  intent : Bool := false
  learner : Bool := false
  voter : Bool := false
  cutAdvanced : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with intent := true}
  | attach {s} (committed : s.intent = true) (fresh : s.voter = false) :
      Step s {s with learner := true, cutAdvanced := true}
  | confirm {s} (attached : s.learner = true) :
      Step s {s with cutAdvanced := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.learner = true → s.intent = true ∧ s.cutAdvanced = true) ∧
  s.voter = false ∧ s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  cases step <;> simp_all [Safe]

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem attach_requires_the_committed_intent {s : State}
    (run : Reachable s) (attached : s.learner = true) :
    s.intent = true ∧ s.cutAdvanced = true :=
  (run_safe run).1 attached

theorem the_destination_is_never_a_voter {s : State} (run : Reachable s) :
    s.voter = false :=
  (run_safe run).2.1

theorem no_serving_capability {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2

theorem the_learner_is_monotone {s t : State} (step : Step s t)
    (attached : s.learner = true) : t.learner = true := by
  cases step <;> simp_all

theorem confirmation_re_advances_the_cut {s : State}
    (attached : s.learner = true) :
    Step s {s with cutAdvanced := true} :=
  .confirm attached

theorem an_intent_alone_attaches_nothing :
    ∃ s, Reachable s ∧ s.intent = true ∧ s.learner = false := by
  exact ⟨{intent := true}, .step .initial .commit, rfl, rfl⟩

end Kv9.LearnerAttach
