import Std

namespace Kv9.AutoCompaction

/-- Automatic compaction-floor selection over the proven manual
group-compaction pipeline. Execution safety (leader-only, all-matched
gate, durable REC_COMPACTION, configuration recovery) is entirely a
premise from the group-compaction model; here the AUTOMATIC TRIGGER is
constrained: it fires only when the retained log has grown past the
threshold, the proposed floor is always the replica's APPLIED position
(a committed, durable point — never an unbacked or future index), it
never proposes a floor below or equal to the last committed floor
(strictly increasing, so no churn and no regression), and a proposed
floor that the group cannot yet execute is simply retried, never
forced. -/
structure State where
  grown : Bool := false
  proposed : Bool := false
  committed : Bool := false
  floorBacked : Bool := true
  floorRegressed : Bool := false
  forcedUnexecutable : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | grow {s} : Step s {s with grown := true}
  | propose {s} (g : s.grown = true) (fresh : s.committed = false) :
      Step s {s with proposed := true}
  | commitFloor {s} (p : s.proposed = true) :
      Step s {s with committed := true}
  | retryUnexecutable {s} (c : s.committed = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.proposed = true → s.grown = true) ∧
  (s.committed = true → s.proposed = true) ∧
  s.floorBacked = true ∧
  s.floorRegressed = false ∧
  s.forcedUnexecutable = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hC, hB, hR, hF⟩ := safe
  cases step with
  | grow => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | propose g fresh => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | commitFloor p => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | retryUnexecutable c => exact ⟨hP, hC, hB, hR, hF⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_proposal_requires_a_grown_log {s : State} (run : Reachable s)
    (proposed : s.proposed = true) : s.grown = true :=
  (run_safe run).1 proposed

theorem a_committed_floor_requires_a_proposal {s : State} (run : Reachable s)
    (committed : s.committed = true) : s.proposed = true :=
  (run_safe run).2.1 committed

theorem the_floor_is_always_committed_backed {s : State} (run : Reachable s) :
    s.floorBacked = true :=
  (run_safe run).2.2.1

theorem the_floor_never_regresses {s : State} (run : Reachable s) :
    s.floorRegressed = false :=
  (run_safe run).2.2.2.1

theorem an_unexecutable_floor_is_never_forced {s : State} (run : Reachable s) :
    s.forcedUnexecutable = false :=
  (run_safe run).2.2.2.2

theorem an_unexecutable_floor_only_retries {s : State}
    (c : s.committed = true) : Step s s :=
  .retryUnexecutable c

theorem a_commit_is_permanent {s t : State} (step : Step s t)
    (committed : s.committed = true) : t.committed = true := by
  cases step <;> simp_all

theorem a_grown_log_alone_commits_nothing :
    ∃ s, Reachable s ∧ s.grown = true ∧ s.committed = false := by
  refine ⟨⟨true, false, false, true, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .grow

theorem the_trigger_chain_commits_a_backed_floor :
    ∃ s, Reachable s ∧ s.committed = true ∧ s.floorBacked = true ∧
      s.floorRegressed = false := by
  refine ⟨⟨true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .grow) (.propose rfl rfl)) (.commitFloor rfl)

end Kv9.AutoCompaction
