import Std

namespace Kv9.ReplicaRemoval

/-- One migration source replica's retirement from the voter set. Evidence
settlement, promotion and raft joint-consensus semantics are premises from
their own models. A removal happens ONLY under a committed decision naming
one exact source replica — never the destination — with the promoted
destination voting and the quorum floor of three voters preserved. -/
structure State where
  migration : Bool := false
  evidence : Bool := false
  decided : Bool := false
  destinationVoter : Bool := false
  removed : Bool := false
  destinationRemoved : Bool := false
  serving : Bool := false
  quorum : Nat := 3
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with migration := true}
  | record {s} (m : s.migration = true) : Step s {s with evidence := true}
  | promote {s} (e : s.evidence = true) (fresh : s.destinationVoter = false) :
      Step s {s with destinationVoter := true, quorum := s.quorum + 1}
  | decide {s} (e : s.evidence = true) : Step s {s with decided := true}
  | remove {s} (d : s.decided = true) (v : s.destinationVoter = true)
      (q : s.quorum = 4) (fresh : s.removed = false) :
      Step s {s with removed := true, quorum := s.quorum - 1}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.removed = true → s.decided = true ∧ s.destinationVoter = true) ∧
  (s.decided = true → s.evidence = true) ∧
  (s.destinationVoter = true → s.evidence = true) ∧
  (s.evidence = true → s.migration = true) ∧
  3 ≤ s.quorum ∧
  s.destinationRemoved = false ∧
  s.serving = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hR, hD, hV, hE, hQ, hX, hS⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hQ, hX, hS⟩
  | record m => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hQ, hX, hS⟩
  | promote e fresh =>
      refine ⟨by simp_all, hD, by simp_all, hE, ?_, hX, hS⟩
      show 3 ≤ s.quorum + 1
      omega
  | decide e => exact ⟨by simp_all, by simp_all, hV, hE, hQ, hX, hS⟩
  | remove d v q fresh =>
      refine ⟨by simp_all, hD, hV, hE, ?_, hX, hS⟩
      show 3 ≤ s.quorum - 1
      omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem removal_requires_the_committed_decision_and_voter {s : State}
    (run : Reachable s) (removed : s.removed = true) :
    s.decided = true ∧ s.destinationVoter = true :=
  (run_safe run).1 removed

theorem a_decision_requires_the_committed_evidence {s : State}
    (run : Reachable s) (decided : s.decided = true) : s.evidence = true :=
  (run_safe run).2.1 decided

theorem the_quorum_never_drops_below_three {s : State} (run : Reachable s) :
    3 ≤ s.quorum :=
  (run_safe run).2.2.2.2.1

theorem the_destination_is_never_removed {s : State} (run : Reachable s) :
    s.destinationRemoved = false :=
  (run_safe run).2.2.2.2.2.1

theorem no_serving_capability {s : State} (run : Reachable s) :
    s.serving = false :=
  (run_safe run).2.2.2.2.2.2

theorem removal_is_permanent {s t : State} (step : Step s t)
    (removed : s.removed = true) : t.removed = true := by
  cases step <;> simp_all

theorem a_removed_chain_carries_the_full_authority {s : State}
    (run : Reachable s) (removed : s.removed = true) :
    s.evidence = true ∧ s.migration = true := by
  have chain := (run_safe run).1 removed
  have e := (run_safe run).2.1 chain.1
  exact ⟨e, (run_safe run).2.2.2.1 e⟩

theorem a_decision_alone_removes_nothing :
    ∃ s, Reachable s ∧ s.decided = true ∧ s.removed = false := by
  refine ⟨⟨true, true, true, false, false, false, false, 3⟩, ?_, rfl, rfl⟩
  exact .step (.step (.step .initial .commit) (.record rfl)) (.decide rfl)

theorem the_committed_chain_reaches_removal :
    ∃ s, Reachable s ∧ s.removed = true ∧ s.quorum = 3 ∧
      s.destinationVoter = true := by
  refine ⟨⟨true, true, true, true, true, false, false, 3⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .commit) (.record rfl))
    (.promote rfl rfl)) (.decide rfl)) (.remove rfl rfl rfl rfl)

end Kv9.ReplicaRemoval
