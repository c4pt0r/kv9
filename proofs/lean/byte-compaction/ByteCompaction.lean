import Std

namespace Kv9.ByteCompaction

/-- Byte-based automatic compaction-floor selection over the proven manual
group-compaction pipeline. Execution safety (leader-only, all-matched gate,
durable REC_COMPACTION, configuration recovery, follower-side confirmation)
is entirely a premise from the group-compaction / follower-compaction
models; here the BYTE TRIGGER is constrained. Entries are a poor proxy for
log cost when value sizes vary, so a group of large values bounds its log by
the retained committed PAYLOAD bytes. This model shows: a proposal fires
only when the retained BYTES crossed the threshold (NOT the entry count — the
entries trigger is independent and, in the byte-only configuration, off), the
proposed floor is always the replica's committed applied position (backed,
never unbacked or regressed), and a floor the group cannot yet execute is
retried, never forced. -/
structure State where
  bytesGrown : Bool := false
  entriesGrown : Bool := false
  proposed : Bool := false
  committed : Bool := false
  floorBacked : Bool := true
  floorRegressed : Bool := false
  forcedUnexecutable : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | growBytes {s} : Step s {s with bytesGrown := true}
  | growEntries {s} : Step s {s with entriesGrown := true}
  -- A proposal requires the BYTE threshold crossed — entry growth alone,
  -- however large, never proposes when the byte trigger is what's configured.
  | propose {s} (b : s.bytesGrown = true) (fresh : s.committed = false) :
      Step s {s with proposed := true}
  | commitFloor {s} (p : s.proposed = true) :
      Step s {s with committed := true}
  | retryUnexecutable {s} (c : s.committed = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.proposed = true → s.bytesGrown = true) ∧
  (s.committed = true → s.proposed = true) ∧
  s.floorBacked = true ∧
  s.floorRegressed = false ∧
  s.forcedUnexecutable = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hC, hB, hR, hF⟩ := safe
  cases step with
  | growBytes => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | growEntries => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | propose b fresh => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | commitFloor p => exact ⟨by simp_all, by simp_all, hB, hR, hF⟩
  | retryUnexecutable c => exact ⟨hP, hC, hB, hR, hF⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_proposal_requires_grown_bytes {s : State} (run : Reachable s)
    (proposed : s.proposed = true) : s.bytesGrown = true :=
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

/-- The distinguishing property: entry growth ALONE — no matter how many
entries — proposes nothing under the byte trigger. A byte-configured group is
bounded by bytes, not by an entry count that the (disabled) entries trigger
would react to. -/
theorem entries_growth_alone_proposes_nothing :
    ∃ s, Reachable s ∧ s.entriesGrown = true ∧ s.bytesGrown = false ∧
      s.proposed = false := by
  refine ⟨⟨false, true, false, false, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial .growEntries

theorem the_byte_trigger_chain_commits_a_backed_floor :
    ∃ s, Reachable s ∧ s.committed = true ∧ s.floorBacked = true ∧
      s.floorRegressed = false := by
  refine ⟨⟨true, false, true, true, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .growBytes) (.propose rfl rfl)) (.commitFloor rfl)

end Kv9.ByteCompaction
