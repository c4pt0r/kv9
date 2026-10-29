import Std

namespace Kv9.ReplicaRepair

/-- The group leader's view when it decides a repair (docs/REPLICA-REPAIR.md §1–§2).
`silent` is how long the suspect voter has been silent, `window` the configured loss
window; `suspectVoter` says the suspect is a committed voter of the group;
`candidateMember` says the chosen destination is already a member and
`candidateIsSuspect` that it is the lost node itself; `plans` counts committed plans
for this loss; `planFor` records which (source, destination) the plan named. -/
structure State where
  leader : Bool := true
  silent : Nat := 0
  window : Nat := 10
  suspectVoter : Bool := true
  candidateActive : Bool := true
  candidateMember : Bool := false
  candidateIsSuspect : Bool := false
  plans : Nat := 0
  planSourceIsSuspect : Bool := true
  planDestinationOutside : Bool := true
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | tick {s} : Step s {s with silent := s.silent + 1}
  | heard {s} : Step s {s with silent := 0}
  | lose_leadership {s} : Step s {s with leader := false, silent := 0}
  | lead {s} : Step s {s with leader := true, silent := 0}
  /-- The decision: only a leader, only after the full window, only a committed voter
  as source, only an active non-member that is not the suspect as destination, and at
  most one plan per loss (a repeat confirms the same row). -/
  | decide {s} (l : s.leader = true) (w : s.window ≤ s.silent) (v : s.suspectVoter = true)
      (a : s.candidateActive = true) (m : s.candidateMember = false)
      (n : s.candidateIsSuspect = false) :
      Step s {s with plans := if s.plans = 0 then 1 else s.plans,
                     planSourceIsSuspect := true, planDestinationOutside := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.plans ≤ 1 ∧ s.planSourceIsSuspect = true ∧ s.planDestinationOutside = true

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hS, hD⟩ := safe
  cases step with
  | tick => exact ⟨hP, hS, hD⟩
  | heard => exact ⟨hP, hS, hD⟩
  | lose_leadership => exact ⟨hP, hS, hD⟩
  | lead => exact ⟨hP, hS, hD⟩
  | decide l w v a m n =>
      refine ⟨?_, rfl, rfl⟩
      show (if s.plans = 0 then 1 else s.plans) ≤ 1
      split <;> omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- R2: one plan per loss, however many leaders decide.
theorem one_plan_per_loss {s : State} (run : Reachable s) : s.plans ≤ 1 :=
  (run_safe run).1

theorem a_repeated_decision_confirms {s t : State} (step : Step s t) (had : s.plans = 1) :
    t.plans = 1 := by
  cases step <;> simp_all

-- R1: a decision names the silent voter and an active outsider, and only a leader
-- that waited the whole window takes it.
theorem a_decision_requires_the_window_and_leadership {s t : State} (step : Step s t)
    (fresh : s.plans = 0) (planned : t.plans = 1) :
    s.leader = true ∧ s.window ≤ s.silent ∧ s.suspectVoter = true ∧
      s.candidateActive = true ∧ s.candidateMember = false ∧ s.candidateIsSuspect = false := by
  cases step <;> simp_all

theorem a_plan_names_the_suspect_and_an_outsider {s : State} (run : Reachable s) :
    s.planSourceIsSuspect = true ∧ s.planDestinationOutside = true :=
  (run_safe run).2

-- R4: hearing from the voter resets the window, so a decision needs continuous silence.
theorem hearing_from_the_voter_resets_the_window {s t : State} (step : Step s t)
    (reset : t.silent = 0) (was : 0 < s.silent) : t.plans = s.plans := by
  cases step <;> simp_all

-- A lost voter is repaired: the window elapses and a plan is committed.
theorem a_lost_voter_gets_a_plan :
    ∃ s, Reachable s ∧ s.plans = 1 ∧ s.window ≤ s.silent := by
  refine ⟨⟨true, 10, 10, true, true, false, false, 1, true, true⟩, ?_, rfl, by decide⟩
  have ten : Reachable ⟨true, 10, 10, true, true, false, false, 0, true, true⟩ :=
    .step (.step (.step (.step (.step (.step (.step (.step (.step (.step .initial .tick) .tick)
      .tick) .tick) .tick) .tick) .tick) .tick) .tick) .tick
  exact .step ten (.decide rfl (by decide) rfl rfl rfl rfl)

end Kv9.ReplicaRepair
