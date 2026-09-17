import Std

namespace Kv9.VoterPromotion

/-- One migration destination's promotion from attached learner to voter.
Attach atomicity, adoption/evidence settlement and quorum semantics are
premises from their own models. Promotion happens ONLY through the group's
own committed log, only for a destination whose committed install evidence
exists, and it is one-way: the voter never demotes here, the learner never
returns, and no step grants serving. -/
structure State where
  migration : Bool := false
  learner : Bool := false
  evidence : Bool := false
  voter : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with migration := true}
  | attach {s} (m : s.migration = true) (fresh : s.voter = false) :
      Step s {s with learner := true}
  | record {s} (l : s.learner = true) :
      Step s {s with evidence := true}
  | promote {s} (e : s.evidence = true) (l : s.learner = true) :
      Step s {s with voter := true, learner := false}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.voter = true → s.evidence = true ∧ s.migration = true) ∧
  (s.evidence = true → s.migration = true ∧ (s.learner = true ∨ s.voter = true)) ∧
  (s.learner = true → s.migration = true) ∧
  ¬(s.learner = true ∧ s.voter = true) ∧
  s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hV, hE, hL, hX, hS⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hS⟩
  | attach m fresh => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hS⟩
  | record l => exact ⟨by simp_all, by simp_all, hL, hX, hS⟩
  | promote e l => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hS⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem promotion_requires_the_committed_evidence {s : State}
    (run : Reachable s) (promoted : s.voter = true) :
    s.evidence = true ∧ s.migration = true :=
  (run_safe run).1 promoted

theorem evidence_implies_membership {s : State}
    (run : Reachable s) (recorded : s.evidence = true) :
    s.learner = true ∨ s.voter = true :=
  ((run_safe run).2.1 recorded).2

theorem never_learner_and_voter {s : State} (run : Reachable s) :
    ¬(s.learner = true ∧ s.voter = true) :=
  (run_safe run).2.2.2.1

theorem no_serving_capability {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2.2.2

theorem a_voter_is_permanent {s t : State} (step : Step s t)
    (promoted : s.voter = true) : t.voter = true := by
  cases step <;> simp_all

theorem the_learner_never_returns_after_promotion {s t : State}
    (step : Step s t) (promoted : s.voter = true) (gone : s.learner = false) :
    t.learner = false := by
  cases step <;> simp_all

theorem an_evidence_row_alone_promotes_nothing :
    ∃ s, Reachable s ∧ s.evidence = true ∧ s.voter = false := by
  refine ⟨⟨true, true, true, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step (.step .initial .commit) (.attach rfl rfl)) (.record rfl)

theorem the_committed_chain_reaches_the_voter :
    ∃ s, Reachable s ∧ s.voter = true ∧ s.learner = false ∧
      s.evidence = true := by
  refine ⟨⟨true, false, true, true, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .commit) (.attach rfl rfl))
    (.record rfl)) (.promote rfl rfl)

end Kv9.VoterPromotion
