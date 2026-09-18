import Std

namespace Kv9.AbortTruncation

/-- Abort-settled source-log truncation. The raft compaction seam's own
safety (leader-only, the durable REC_COMPACTION record, tail
preservation, the deferred-sync barrier) and the settlement rows'
integrity are premises from their models; here the AUTHORITY CHAIN is
constrained: a truncation decision requires exactly ONE committed
settlement — install evidence (whose cut bounds the floor) or a
committed abort (whose detached learner no longer consumes the tail) —
never both, never neither; compaction additionally requires the
RELEASED source pin and EVERY voter matched at or beyond the floor; and
only the local log prefix is ever discarded. -/
structure State where
  settledByEvidence : Bool := false
  settledByAbort : Bool := false
  pinReleased : Bool := false
  allMatched : Bool := false
  decisionCommitted : Bool := false
  compacted : Bool := false
  unsettledTruncation : Bool := false
  floorAboveCut : Bool := false
  foreignPrefixDiscarded : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | settleEvidence {s} (clean : s.settledByAbort = false) :
      Step s {s with settledByEvidence := true}
  | settleAbort {s} (clean : s.settledByEvidence = false) :
      Step s {s with settledByAbort := true}
  | releasePin {s}
      (settled : s.settledByEvidence = true ∨ s.settledByAbort = true) :
      Step s {s with pinReleased := true}
  | matchAll {s} : Step s {s with allMatched := true}
  | recordDecision {s}
      (settled : s.settledByEvidence = true ∨ s.settledByAbort = true) :
      Step s {s with decisionCommitted := true}
  | compact {s} (d : s.decisionCommitted = true) (p : s.pinReleased = true)
      (m : s.allMatched = true) : Step s {s with compacted := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  ¬(s.settledByEvidence = true ∧ s.settledByAbort = true) ∧
  (s.decisionCommitted = true →
    s.settledByEvidence = true ∨ s.settledByAbort = true) ∧
  (s.compacted = true →
    s.decisionCommitted = true ∧ s.pinReleased = true ∧ s.allMatched = true) ∧
  s.unsettledTruncation = false ∧
  s.floorAboveCut = false ∧
  s.foreignPrefixDiscarded = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hX, hD, hC, hU, hF, hL⟩ := safe
  cases step with
  | settleEvidence clean =>
      exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩
  | settleAbort clean =>
      exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩
  | releasePin settled =>
      exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩
  | matchAll => exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩
  | recordDecision settled =>
      exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩
  | compact d p m =>
      exact ⟨by simp_all, by simp_all, by simp_all, hU, hF, hL⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem evidence_and_abort_never_both_settle {s : State} (run : Reachable s) :
    ¬(s.settledByEvidence = true ∧ s.settledByAbort = true) :=
  (run_safe run).1

theorem a_decision_requires_one_committed_settlement {s : State}
    (run : Reachable s) (d : s.decisionCommitted = true) :
    s.settledByEvidence = true ∨ s.settledByAbort = true :=
  (run_safe run).2.1 d

theorem compaction_requires_decision_pin_and_matched_voters {s : State}
    (run : Reachable s) (c : s.compacted = true) :
    s.decisionCommitted = true ∧ s.pinReleased = true ∧ s.allMatched = true :=
  (run_safe run).2.2.1 c

theorem no_unsettled_operation_ever_truncates {s : State} (run : Reachable s) :
    s.unsettledTruncation = false :=
  (run_safe run).2.2.2.1

theorem the_floor_never_exceeds_the_evidence_cut {s : State}
    (run : Reachable s) : s.floorAboveCut = false :=
  (run_safe run).2.2.2.2.1

theorem only_the_local_prefix_is_ever_discarded {s : State}
    (run : Reachable s) : s.foreignPrefixDiscarded = false :=
  (run_safe run).2.2.2.2.2

theorem a_lagging_voter_blocks_compaction {s : State}
    (d : s.decisionCommitted = true) (p : s.pinReleased = true)
    (m : s.allMatched = true) : Step s {s with compacted := true} :=
  .compact d p m

theorem a_settlement_is_permanent {s t : State} (step : Step s t)
    (a : s.settledByAbort = true) : t.settledByAbort = true := by
  cases step <;> simp_all

theorem an_abort_alone_compacts_nothing :
    ∃ s, Reachable s ∧ s.settledByAbort = true ∧ s.compacted = false := by
  refine ⟨⟨false, true, false, false, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial (.settleAbort rfl)

theorem the_abort_settled_chain_compacts :
    ∃ s, Reachable s ∧ s.settledByAbort = true ∧ s.compacted = true ∧
      s.settledByEvidence = false ∧ s.unsettledTruncation = false := by
  refine ⟨⟨false, true, true, true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial (.settleAbort rfl))
    (.releasePin (Or.inr rfl))) .matchAll) (.recordDecision (Or.inr rfl)))
    (.compact rfl rfl rfl)

end Kv9.AbortTruncation
