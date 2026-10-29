import Std

namespace Kv9.LeaderTransfer

/-- One group during a leadership transfer (docs/LEADER-TRANSFER.md §2). `leader` is
the node this term's raft leadership belongs to (raft's single-leader-per-term is a
premise, so it is one value); `lease` says the leader holds a usable lease
certificate; `transferring` is the window between the decision and raft's outcome;
`servedAfterDecision` records a lease read answered after the decision — the bad
event. The transferee's membership and progress are observations the verb guards on. -/
structure State where
  leader : Nat := 1
  term : Nat := 1
  lease : Bool := true
  transferring : Bool := false
  transferee : Nat := 2
  transfereeVoter : Bool := true
  transfereeCaughtUp : Bool := true
  servedAfterDecision : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  /-- The verb: only the current leader, only a caught-up voter, never twice; it
  revokes the lease in the same critical section that asks raft to transfer. -/
  | nominate {s} (v : s.transfereeVoter = true) (c : s.transfereeCaughtUp = true)
      (idle : s.transferring = false) (notSelf : s.transferee ≠ s.leader) :
      Step s {s with transferring := true, lease := false}
  /-- A lease read: the fast path needs a usable certificate; it records the bad
  event if it ran after a decision. -/
  | leaseRead {s} (l : s.lease = true) :
      Step s {s with servedAfterDecision := s.servedAfterDecision || s.transferring}
  /-- raft's `MsgTimeoutNow` outcome: the transferee leads a new term; it has no
  lease yet. -/
  | complete {s} (t : s.transferring = true) :
      Step s {s with leader := s.transferee, term := s.term + 1, transferring := false,
                     lease := false}
  /-- raft abandons the transfer after an election timeout: the old leader keeps
  leading this term, still without a lease. -/
  | expire {s} (t : s.transferring = true) : Step s {s with transferring := false}
  /-- A lease is earned only by the current leader and never while transferring. -/
  | grant {s} (idle : s.transferring = false) : Step s {s with lease := true}
  /-- The transferee's observed state may change at any time (another node may win
  an election, the transferee may lag): the guard is re-evaluated at the decision. -/
  | observe {s} (v c : Bool) : Step s {s with transfereeVoter := v, transfereeCaughtUp := c}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.transferring = true → s.lease = false) ∧ s.servedAfterDecision = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hL, hS⟩ := safe
  cases step with
  | nominate v c idle notSelf => exact ⟨by simp, by simp [hS]⟩
  | leaseRead l =>
      refine ⟨by simp_all, ?_⟩
      -- a usable lease means no transfer is in flight, so the read is not after a decision
      have : s.transferring = false := by
        cases h : s.transferring
        · rfl
        · exact absurd (hL h) (by simp [l])
      simp [this, hS]
  | complete t => exact ⟨by simp, by simp [hS]⟩
  | expire t => exact ⟨by simp, by simp [hS]⟩
  | grant idle => exact ⟨by simp [idle], by simp [hS]⟩
  | observe v c => exact ⟨by simp_all, by simp [hS]⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- T2: no lease read is served after the decision, and no lease exists in the window.
theorem no_lease_read_after_the_decision {s : State} (run : Reachable s) :
    s.servedAfterDecision = false :=
  (run_safe run).2

theorem the_window_holds_no_lease {s : State} (run : Reachable s)
    (t : s.transferring = true) : s.lease = false :=
  (run_safe run).1 t

-- T3: only a caught-up voter is nominated, never the leader itself, never twice.
theorem nomination_requires_a_caught_up_voter {s t : State} (step : Step s t)
    (started : s.transferring = false) (now : t.transferring = true) :
    s.transfereeVoter = true ∧ s.transfereeCaughtUp = true ∧ s.transferee ≠ s.leader := by
  cases step <;> simp_all

theorem a_transfer_is_not_nominated_twice {s t : State} (step : Step s t)
    (busy : s.transferring = true) : t.term = s.term ∨ t.leader = s.transferee := by
  cases step <;> simp_all

-- T1 (as a premise made explicit): a completed transfer moves leadership to the
-- transferee in a strictly higher term; the old term keeps its single leader.
theorem completion_advances_the_term_to_the_transferee {s t : State} (step : Step s t)
    (busy : s.transferring = true) (moved : t.leader ≠ s.leader) :
    t.leader = s.transferee ∧ t.term = s.term + 1 ∧ t.lease = false := by
  cases step <;> simp_all

-- T4: from the window both outcomes are reachable, and nothing else leaves it.
theorem the_window_ends_by_completion_or_expiry {s t : State} (step : Step s t)
    (busy : s.transferring = true) (ended : t.transferring = false) :
    (t.leader = s.transferee ∧ t.term = s.term + 1) ∨ (t.leader = s.leader ∧ t.term = s.term) := by
  cases step <;> simp_all

theorem a_transfer_can_complete :
    ∃ s, Reachable s ∧ s.leader = 2 ∧ s.term = 2 ∧ s.transferring = false ∧ s.lease = false := by
  refine ⟨⟨2, 2, false, false, 2, true, true, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step .initial (.nominate rfl rfl rfl (by decide))) (.complete rfl)

theorem a_transfer_can_expire_and_the_leader_relearns_its_lease :
    ∃ s, Reachable s ∧ s.leader = 1 ∧ s.term = 1 ∧ s.transferring = false ∧ s.lease = true ∧
      s.servedAfterDecision = false := by
  refine ⟨⟨1, 1, true, false, 2, true, true, false⟩, ?_, rfl, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial (.nominate rfl rfl rfl (by decide))) (.expire rfl)) (.grant rfl)

end Kv9.LeaderTransfer
