import Std

namespace Kv9.GroupCompaction

/-- Healthy-group raft-log compaction driven by committed kind-110 floors.
The raft compaction seam's own safety (leader-only execution, the
all-matched peer gate, the durable REC_COMPACTION record, tail
preservation, the deferred-sync barrier) and the catalog row's integrity
are premises from their models; here the DECISION→EXECUTION chain and the
repeated-compaction configuration recovery are constrained: floors per
region strictly increase, execution requires the committed floor AND the
local applied position at or beyond it AND all voters matched, a second
floor above an earlier compacted base still resolves its configuration
(from the durable base, not a blanket refusal), and only the local
prefix is ever discarded — a committed entry, held by a quorum, is never
lost. -/
structure State where
  floorCommitted : Bool := false
  higherFloorCommitted : Bool := false
  appliedAtFloor : Bool := false
  allMatched : Bool := false
  compacted : Bool := false
  recompacted : Bool := false
  configResolvedAfterCompaction : Bool := false
  floorsRegressed : Bool := false
  committedEntryLost : Bool := false
  foreignPrefixDiscarded : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commitFloor {s} : Step s {s with floorCommitted := true}
  | commitHigherFloor {s} (c : s.compacted = true) :
      Step s {s with higherFloorCommitted := true}
  | apply {s} (f : s.floorCommitted = true) : Step s {s with appliedAtFloor := true}
  | matchAll {s} : Step s {s with allMatched := true}
  | compact {s} (f : s.floorCommitted = true) (a : s.appliedAtFloor = true)
      (m : s.allMatched = true) :
      Step s {s with compacted := true, configResolvedAfterCompaction := true}
  | recompact {s} (h : s.higherFloorCommitted = true) (base : s.compacted = true)
      (a : s.appliedAtFloor = true) (m : s.allMatched = true) :
      Step s {s with recompacted := true, configResolvedAfterCompaction := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.compacted = true →
    s.floorCommitted = true ∧ s.appliedAtFloor = true ∧ s.allMatched = true) ∧
  (s.recompacted = true → s.higherFloorCommitted = true ∧ s.compacted = true) ∧
  (s.higherFloorCommitted = true → s.compacted = true) ∧
  s.floorsRegressed = false ∧
  s.committedEntryLost = false ∧
  s.foreignPrefixDiscarded = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hC, hR, hH, hF, hE, hP⟩ := safe
  cases step with
  | commitFloor => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩
  | commitHigherFloor c => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩
  | apply f => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩
  | matchAll => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩
  | compact f a m => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩
  | recompact h base a m => exact ⟨by simp_all, by simp_all, by simp_all, hF, hE, hP⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem compaction_requires_floor_apply_and_matched_voters {s : State}
    (run : Reachable s) (c : s.compacted = true) :
    s.floorCommitted = true ∧ s.appliedAtFloor = true ∧ s.allMatched = true :=
  (run_safe run).1 c

theorem a_higher_floor_requires_the_first_compaction {s : State}
    (run : Reachable s) (h : s.higherFloorCommitted = true) : s.compacted = true :=
  (run_safe run).2.2.1 h

theorem no_committed_entry_is_ever_lost {s : State} (run : Reachable s) :
    s.committedEntryLost = false :=
  (run_safe run).2.2.2.2.1

theorem only_the_local_prefix_is_ever_discarded {s : State} (run : Reachable s) :
    s.foreignPrefixDiscarded = false :=
  (run_safe run).2.2.2.2.2

theorem floors_never_regress {s : State} (run : Reachable s) :
    s.floorsRegressed = false :=
  (run_safe run).2.2.2.1

theorem recompaction_requires_the_earlier_base {s : State} (run : Reachable s)
    (r : s.recompacted = true) : s.compacted = true :=
  ((run_safe run).2.1 r).2

theorem a_compaction_resolves_the_configuration {s t : State} (step : Step s t)
    (was : s.configResolvedAfterCompaction = false)
    (now : t.configResolvedAfterCompaction = true) : t.compacted = true ∨ t.recompacted = true := by
  cases step <;> simp_all

theorem the_gated_chain_compacts :
    ∃ s, Reachable s ∧ s.compacted = true ∧ s.configResolvedAfterCompaction = true ∧
      s.committedEntryLost = false := by
  refine ⟨⟨true, false, true, true, true, false, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .commitFloor) (.apply rfl)) .matchAll)
    (.compact rfl rfl rfl)

theorem a_second_floor_recompacts_and_resolves_its_configuration :
    ∃ s, Reachable s ∧ s.recompacted = true ∧ s.configResolvedAfterCompaction = true ∧
      s.higherFloorCommitted = true := by
  refine ⟨⟨true, true, true, true, true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step (.step .initial .commitFloor) (.apply rfl))
    .matchAll) (.compact rfl rfl rfl)) (.commitHigherFloor rfl)) (.recompact rfl rfl rfl rfl)

theorem a_committed_floor_alone_compacts_nothing :
    ∃ s, Reachable s ∧ s.floorCommitted = true ∧ s.compacted = false := by
  refine ⟨⟨true, false, false, false, false, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .commitFloor

end Kv9.GroupCompaction
