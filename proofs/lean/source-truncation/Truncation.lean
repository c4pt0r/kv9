import Std

namespace Kv9.SourceTruncation

/-- One source group's log-prefix compaction under committed authority.
Evidence/settlement semantics are premises from the install-evidence model;
storage atomicity from the persistence model. Positions are Nats:
`evidenceCut` is the committed evidence cut, `floor` the committed decision
floor (0 = no decision), `retained` the log's first unavailable bound
(first_index − 1; 0 = full log), `matched` the least progress any tracked
peer has matched. Compaction never exceeds the committed floor, the floor
never exceeds the evidence cut, no peer is ever left behind the retained
bound, and a compacted replica restarts only under the committed decision. -/
structure State where
  evidence : Bool := false
  evidenceCut : Nat := 0
  floor : Nat := 0
  retained : Nat := 0
  matched : Nat := 0
  restartedUnauthorized : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commitEvidence {s} (c : Nat) (exact : 0 < c) (fresh : s.evidence = false) :
      Step s {s with evidence := true, evidenceCut := c}
  | decide {s} (e : s.evidence = true) (f : Nat) (exact : 0 < f)
      (bounded : f ≤ s.evidenceCut) (fresh : s.floor = 0) :
      Step s {s with floor := f}
  | advance {s} (m : Nat) : Step s {s with matched := s.matched + m}
  | compact {s} (decided : 0 < s.floor) (progressed : s.floor ≤ s.matched) :
      Step s {s with retained := s.floor}
  | restart {s} (authorized : s.retained = 0 ∨ s.retained = s.floor) :
      Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (0 < s.floor → s.evidence = true ∧ s.floor ≤ s.evidenceCut) ∧
  (0 < s.retained → s.retained = s.floor ∧ s.retained ≤ s.matched) ∧
  s.restartedUnauthorized = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hF, hR, hU⟩ := safe
  cases step with
  | commitEvidence c exact fresh =>
      refine ⟨fun d => ⟨rfl, ?_⟩, hR, hU⟩
      · have := hF d
        simp_all
  | decide e f exact bounded fresh =>
      refine ⟨fun _ => ⟨e, bounded⟩, fun r => ?_, hU⟩
      · have := hR r
        simp_all
  | advance m =>
      refine ⟨hF, fun r => ⟨(hR r).1, ?_⟩, hU⟩
      have := (hR r).2
      show s.retained ≤ s.matched + m
      omega
  | compact decided progressed =>
      exact ⟨hF, fun _ => ⟨rfl, progressed⟩, hU⟩
  | restart authorized => exact ⟨hF, hR, hU⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_decision_requires_evidence_below_its_cut {s : State}
    (run : Reachable s) (decided : 0 < s.floor) :
    s.evidence = true ∧ s.floor ≤ s.evidenceCut :=
  (run_safe run).1 decided

theorem compaction_never_exceeds_the_committed_floor {s : State}
    (run : Reachable s) (compacted : 0 < s.retained) : s.retained = s.floor :=
  ((run_safe run).2.1 compacted).1

theorem no_tracked_peer_is_left_behind_the_retained_bound {s : State}
    (run : Reachable s) (compacted : 0 < s.retained) :
    s.retained ≤ s.matched :=
  ((run_safe run).2.1 compacted).2

theorem a_compacted_log_implies_the_full_committed_chain {s : State}
    (run : Reachable s) (compacted : 0 < s.retained) :
    s.evidence = true ∧ s.retained ≤ s.evidenceCut := by
  have floor := compaction_never_exceeds_the_committed_floor run compacted
  have decided : 0 < s.floor := by omega
  have chain := a_decision_requires_evidence_below_its_cut run decided
  exact ⟨chain.1, by omega⟩

theorem no_unauthorized_restart_ever_happens {s : State} (run : Reachable s) :
    s.restartedUnauthorized = false :=
  (run_safe run).2.2

theorem the_retained_bound_never_regresses {s t : State} (step : Step s t)
    (safe : Safe s) : s.retained ≤ t.retained := by
  cases step <;> simp_all [Safe] <;> omega

theorem the_floor_is_immutable_once_decided {s t : State} (step : Step s t)
    (decided : 0 < s.floor) : t.floor = s.floor := by
  cases step <;> simp_all <;> omega

theorem matched_progress_is_monotone {s t : State} (step : Step s t) :
    s.matched ≤ t.matched := by
  cases step <;> simp_all

theorem a_decision_alone_compacts_nothing :
    ∃ s, Reachable s ∧ 0 < s.floor ∧ s.retained = 0 := by
  refine ⟨⟨true, 5, 5, 0, 0, false⟩, ?_, by decide, rfl⟩
  exact .step (.step .initial (.commitEvidence 5 (by decide) rfl))
    (.decide rfl 5 (by decide) (by decide) rfl)

theorem the_committed_chain_reaches_compaction_and_restart :
    ∃ s, Reachable s ∧ s.retained = s.floor ∧ 0 < s.retained ∧
      Step s s := by
  refine ⟨⟨true, 5, 5, 5, 7, false⟩, ?_, rfl, by decide, .restart (by simp)⟩
  exact .step (.step (.step (.step .initial (.commitEvidence 5 (by decide) rfl))
    (.decide rfl 5 (by decide) (by decide) rfl)) (.advance 7))
    (.compact (by decide) (by decide))

end Kv9.SourceTruncation
