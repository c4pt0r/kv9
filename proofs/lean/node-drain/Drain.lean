import Std

namespace Kv9.NodeDrain

/-- One drain of store `X` as the metadata leader drives it (docs/DRAIN-NODE.md §3–§5).
`groupsLeft` counts the data groups that still hold `X`; `inFlight` the unsettled move
plans with source `X`, bounded by `parallel`; `metaVoters` the metadata group's voter
count with `xVoter` / `xLearner` `X`'s own membership; `learnerReady` says a caught-up
learner other than `X` exists; `destPlaceable` that an active, non-draining outsider
exists and `destDraining` whether the candidate at hand is itself draining;
`groupSize` is the size of the group being moved and `received` counts replicas placed
onto a draining store. -/
structure State where
  groupsLeft : Nat := 2
  inFlight : Nat := 0
  parallel : Nat := 1
  metaVoters : Nat := 3
  xVoter : Bool := true
  xLearner : Bool := false
  learnerReady : Bool := true
  destPlaceable : Bool := true
  destDraining : Bool := false
  groupSize : Nat := 3
  received : Nat := 0
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  /-- Commit one move plan `X → D`: only under the concurrency bound, only for a group
  that still holds `X`, only to a placeable, non-draining destination. -/
  | plan {s} (bound : s.inFlight < s.parallel) (left : 0 < s.groupsLeft)
      (placeable : s.destPlaceable = true) (outsider : s.destDraining = false) :
      Step s {s with inFlight := s.inFlight + 1,
                     received := if s.destDraining then s.received + 1 else s.received}
  /-- The plan driver settles a plan: the destination is a voter and `X` is removed in
  that order, so the group keeps its size. -/
  | settle {s} (flying : 0 < s.inFlight) :
      Step s {s with inFlight := s.inFlight - 1, groupsLeft := s.groupsLeft - 1,
                     groupSize := s.groupSize}
  /-- Promote a caught-up metadata learner first when only three voters remain. -/
  | promote {s} (voter : s.xVoter = true) (done : s.groupsLeft = 0)
      (three : s.metaVoters ≤ 3) (ready : s.learnerReady = true) :
      Step s {s with metaVoters := s.metaVoters + 1}
  /-- Remove `X` from the voters: only once no group holds it and only from four or more. -/
  | removeVoter {s} (voter : s.xVoter = true) (done : s.groupsLeft = 0)
      (four : 3 < s.metaVoters) :
      Step s {s with metaVoters := s.metaVoters - 1, xVoter := false}
  /-- A learner leaves without touching the voter count. -/
  | removeLearner {s} (learner : s.xLearner = true) (done : s.groupsLeft = 0) :
      Step s {s with xLearner := false}
  | candidateDrains {s} : Step s {s with destDraining := true}
  | candidateFresh {s} : Step s {s with destDraining := false}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.inFlight ≤ s.parallel ∧ 3 ≤ s.metaVoters ∧ s.received = 0 ∧ s.groupSize = 3

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hB, hV, hR, hG⟩ := safe
  cases step with
  | plan bound left placeable outsider =>
      refine ⟨by simp; omega, hV, ?_, hG⟩
      simp [outsider, hR]
  | settle flying => exact ⟨by simp; omega, hV, hR, hG⟩
  | promote voter done three ready => exact ⟨hB, by simp; omega, hR, hG⟩
  | removeVoter voter done four => exact ⟨hB, by simp; omega, hR, hG⟩
  | removeLearner learner done => exact ⟨hB, hV, hR, hG⟩
  | candidateDrains => exact ⟨hB, hV, hR, hG⟩
  | candidateFresh => exact ⟨hB, hV, hR, hG⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- N1: a group never shrinks; every membership change is a move.
theorem a_group_never_shrinks {s : State} (run : Reachable s) : s.groupSize = 3 :=
  (run_safe run).2.2.2

-- N2: the metadata group never has fewer than three voters.
theorem metadata_never_below_three {s : State} (run : Reachable s) : 3 ≤ s.metaVoters :=
  (run_safe run).2.1

-- N3: a draining store receives nothing.
theorem a_draining_store_receives_nothing {s : State} (run : Reachable s) : s.received = 0 :=
  (run_safe run).2.2.1

-- N4: at most `parallel` plans with the drained source are unsettled.
theorem bounded_concurrency {s : State} (run : Reachable s) : s.inFlight ≤ s.parallel :=
  (run_safe run).1

-- Membership leaves last: no step removes `X` from the metadata group while a data
-- group still holds it.
theorem removal_waits_for_the_groups {s t : State} (step : Step s t)
    (left : 0 < s.groupsLeft) : t.xVoter = s.xVoter ∧ t.xLearner = s.xLearner := by
  cases step <;> simp_all

-- A three-voter group is never shrunk: a step that lowers the voter count started
-- from four or more.
theorem a_three_voter_group_promotes_first {s t : State} (step : Step s t)
    (lowered : t.metaVoters < s.metaVoters) : 3 < s.metaVoters := by
  cases step <;> simp_all <;> omega

-- A promotion is the three-voter rule at work: only once the groups are done, only
-- from three voters, only with a caught-up learner.
theorem a_promotion_needs_a_ready_learner {s t : State} (step : Step s t)
    (raised : s.metaVoters < t.metaVoters) :
    s.learnerReady = true ∧ s.metaVoters ≤ 3 ∧ s.groupsLeft = 0 := by
  cases step <;> simp_all <;> omega

-- A plan is only ever committed to a placeable, non-draining destination.
theorem a_plan_needs_a_placeable_outsider {s t : State} (step : Step s t)
    (planned : s.inFlight < t.inFlight) :
    s.destPlaceable = true ∧ s.destDraining = false ∧ s.inFlight < s.parallel := by
  cases step <;> simp_all <;> omega

-- N5: with a placeable destination and one caught-up learner, a drain of a three-voter
-- member holding two groups finishes without operator steps.
theorem a_drain_finishes :
    ∃ s, Reachable s ∧ s.groupsLeft = 0 ∧ s.inFlight = 0 ∧ s.xVoter = false ∧
      s.xLearner = false ∧ 3 ≤ s.metaVoters := by
  refine ⟨⟨0, 0, 1, 3, false, false, true, true, false, 3, 0⟩, ?_, rfl, rfl, rfl, rfl, by decide⟩
  have one : Reachable ⟨2, 1, 1, 3, true, false, true, true, false, 3, 0⟩ :=
    .step .initial (.plan (by decide) (by decide) rfl rfl)
  have two : Reachable ⟨1, 0, 1, 3, true, false, true, true, false, 3, 0⟩ :=
    .step one (.settle (by decide))
  have three : Reachable ⟨1, 1, 1, 3, true, false, true, true, false, 3, 0⟩ :=
    .step two (.plan (by decide) (by decide) rfl rfl)
  have four : Reachable ⟨0, 0, 1, 3, true, false, true, true, false, 3, 0⟩ :=
    .step three (.settle (by decide))
  have five : Reachable ⟨0, 0, 1, 4, true, false, true, true, false, 3, 0⟩ :=
    .step four (.promote rfl rfl (by decide) rfl)
  exact .step five (.removeVoter rfl rfl (by decide))

end Kv9.NodeDrain
