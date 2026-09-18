import Std

namespace Kv9.DeferredSync

/-- Deferred engine-apply synchronization under raft-log durability. The
raft log's own persistence contract (fsync before send/commit) and the
recovery machinery (torn-tail truncation, committed-entry replay) are
premises from their designs; here the DEFERRAL is constrained: an
acknowledgement requires the raft-durable entry and the applied write —
never the engine sync; a crash loses only the unsynced engine tail and
replay from the durable raft log restores the applied write, so no
acknowledged write is ever lost; and the replay source is never
discarded under deferral — compaction requires the sync barrier first. -/
structure State where
  raftDurable : Bool := false
  engineWritten : Bool := false
  acked : Bool := false
  engineSynced : Bool := false
  crashed : Bool := false
  replayed : Bool := false
  ackedWriteLost : Bool := false
  compactedUnderDeferral : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | persistRaft {s} : Step s {s with raftDurable := true}
  | applyWrite {s} (r : s.raftDurable = true) :
      Step s {s with engineWritten := true}
  | ack {s} (r : s.raftDurable = true) (w : s.engineWritten = true) :
      Step s {s with acked := true}
  | syncEngine {s} (w : s.engineWritten = true) :
      Step s {s with engineSynced := true}
  | compact {s} (synced : s.engineSynced = true) : Step s s
  | crash {s} : Step s {s with crashed := true, engineWritten := s.engineSynced}
  | replay {s} (c : s.crashed = true) (r : s.raftDurable = true) :
      Step s {s with engineWritten := true, replayed := true, crashed := false}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.acked = true → s.raftDurable = true) ∧
  (s.engineSynced = true → s.engineWritten = true ∨ s.crashed = true) ∧
  s.ackedWriteLost = false ∧
  s.compactedUnderDeferral = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hA, hS, hL, hC⟩ := safe
  cases step with
  | persistRaft => exact ⟨by simp_all, by simp_all, hL, hC⟩
  | applyWrite r => exact ⟨by simp_all, by simp_all, hL, hC⟩
  | ack r w => exact ⟨by simp_all, by simp_all, hL, hC⟩
  | syncEngine w => exact ⟨by simp_all, by simp_all, hL, hC⟩
  | compact synced => exact ⟨hA, hS, hL, hC⟩
  | crash => exact ⟨by simp_all, by simp_all, hL, hC⟩
  | replay c r => exact ⟨by simp_all, by simp_all, hL, hC⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem an_ack_requires_the_durable_raft_entry {s : State} (run : Reachable s)
    (acked : s.acked = true) : s.raftDurable = true :=
  (run_safe run).1 acked

theorem no_acked_write_is_ever_lost {s : State} (run : Reachable s) :
    s.ackedWriteLost = false :=
  (run_safe run).2.2.1

theorem the_replay_source_is_never_discarded_under_deferral {s : State}
    (run : Reachable s) : s.compactedUnderDeferral = false :=
  (run_safe run).2.2.2

theorem an_ack_requires_the_applied_write {s : State}
    (r : s.raftDurable = true) (w : s.engineWritten = true) :
    Step s {s with acked := true} :=
  .ack r w

theorem compaction_requires_the_sync_barrier {s : State}
    (synced : s.engineSynced = true) : Step s s :=
  .compact synced

theorem raft_durability_survives_a_crash {s t : State} (step : Step s t)
    (r : s.raftDurable = true) : t.raftDurable = true := by
  cases step <;> simp_all

theorem an_acknowledgement_survives_a_crash {s t : State} (step : Step s t)
    (a : s.acked = true) : t.acked = true := by
  cases step <;> simp_all

theorem a_crash_loses_only_the_unsynced_tail {s : State}
    (synced : s.engineSynced = true) (not_crashed : s.crashed = false) :
    ∀ t, Step s t → (t.crashed = true → t.engineWritten = true) := by
  intro t step
  cases step <;> simp_all

theorem replay_restores_the_applied_write {s : State}
    (c : s.crashed = true) (r : s.raftDurable = true) :
    Step s {s with engineWritten := true, replayed := true, crashed := false} :=
  .replay c r

theorem an_unsynced_crash_leaves_the_write_to_replay :
    ∃ s, Reachable s ∧ s.acked = true ∧ s.crashed = true ∧
      s.engineWritten = false ∧ s.raftDurable = true := by
  refine ⟨⟨true, false, true, false, true, false, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .persistRaft) (.applyWrite rfl))
    (.ack rfl rfl)) .crash

theorem the_crashed_acked_write_reads_back_after_replay :
    ∃ s, Reachable s ∧ s.acked = true ∧ s.replayed = true ∧
      s.engineWritten = true ∧ s.ackedWriteLost = false := by
  refine ⟨⟨true, true, true, false, false, true, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .persistRaft)
    (.applyWrite rfl)) (.ack rfl rfl)) .crash) (.replay rfl rfl)

end Kv9.DeferredSync
