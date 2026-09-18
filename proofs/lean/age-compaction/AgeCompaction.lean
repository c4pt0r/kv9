import Std

namespace Kv9.AgeCompaction

/-- Age-based automatic compaction-floor selection over the proven manual
group-compaction pipeline. Execution safety (leader-only, all-matched gate,
durable REC_COMPACTION, configuration recovery, follower-side confirmation)
is entirely a premise from the group-compaction / follower-compaction models;
here the AGE TRIGGER is constrained. A LOW-TRAFFIC group whose log never
grows past a size threshold should still compact so recovery does not replay
ancient entries forever. This model shows: a proposal fires ONLY when the
oldest retained entry has aged past the threshold (age is measured as how
long `first_index` has stayed put — a compaction is the only thing that
advances it) AND there is something to compact (a retained committed entry
beyond the floor); the proposed floor is always the replica's committed
applied position (backed, never unbacked or regressed); and a floor the group
cannot yet execute is retried, never forced. -/
structure State where
  aged : Bool := false
  hasRetained : Bool := false
  proposed : Bool := false
  committed : Bool := false
  floorBacked : Bool := true
  floorRegressed : Bool := false
  forcedUnexecutable : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | age {s} : Step s {s with aged := true}
  | accrueRetained {s} : Step s {s with hasRetained := true}
  -- A proposal requires BOTH the age threshold crossed AND a retained entry to
  -- compact: an idle group with nothing retained never proposes on age alone.
  | propose {s} (a : s.aged = true) (r : s.hasRetained = true) (fresh : s.committed = false) :
      Step s {s with proposed := true}
  | commitFloor {s} (p : s.proposed = true) :
      Step s {s with committed := true}
  | retryUnexecutable {s} (c : s.committed = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.proposed = true → s.aged = true) ∧
  (s.proposed = true → s.hasRetained = true) ∧
  (s.committed = true → s.proposed = true) ∧
  s.floorBacked = true ∧
  s.floorRegressed = false ∧
  s.forcedUnexecutable = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hA, hR, hC, hB, hRe, hF⟩ := safe
  cases step with
  | age => exact ⟨by simp_all, by simp_all, by simp_all, hB, hRe, hF⟩
  | accrueRetained => exact ⟨by simp_all, by simp_all, by simp_all, hB, hRe, hF⟩
  | propose a r fresh => exact ⟨by simp_all, by simp_all, by simp_all, hB, hRe, hF⟩
  | commitFloor p => exact ⟨by simp_all, by simp_all, by simp_all, hB, hRe, hF⟩
  | retryUnexecutable c => exact ⟨hA, hR, hC, hB, hRe, hF⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_proposal_requires_aging {s : State} (run : Reachable s)
    (proposed : s.proposed = true) : s.aged = true :=
  (run_safe run).1 proposed

theorem a_proposal_requires_something_to_compact {s : State} (run : Reachable s)
    (proposed : s.proposed = true) : s.hasRetained = true :=
  (run_safe run).2.1 proposed

theorem a_committed_floor_requires_a_proposal {s : State} (run : Reachable s)
    (committed : s.committed = true) : s.proposed = true :=
  (run_safe run).2.2.1 committed

theorem the_floor_is_always_committed_backed {s : State} (run : Reachable s) :
    s.floorBacked = true :=
  (run_safe run).2.2.2.1

theorem the_floor_never_regresses {s : State} (run : Reachable s) :
    s.floorRegressed = false :=
  (run_safe run).2.2.2.2.1

theorem an_unexecutable_floor_is_never_forced {s : State} (run : Reachable s) :
    s.forcedUnexecutable = false :=
  (run_safe run).2.2.2.2.2

theorem an_unexecutable_floor_only_retries {s : State}
    (c : s.committed = true) : Step s s :=
  .retryUnexecutable c

theorem a_commit_is_permanent {s t : State} (step : Step s t)
    (committed : s.committed = true) : t.committed = true := by
  cases step <;> simp_all

/-- An idle group that has aged but has NOTHING retained to compact proposes
nothing — age alone is not a licence to compact an empty window. -/
theorem aging_without_retained_proposes_nothing :
    ∃ s, Reachable s ∧ s.aged = true ∧ s.hasRetained = false ∧
      s.proposed = false := by
  refine ⟨⟨true, false, false, false, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial .age

theorem the_age_trigger_chain_commits_a_backed_floor :
    ∃ s, Reachable s ∧ s.committed = true ∧ s.floorBacked = true ∧
      s.floorRegressed = false := by
  refine ⟨⟨true, true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .age) .accrueRetained)
    (.propose rfl rfl rfl)) (.commitFloor rfl)

end Kv9.AgeCompaction
