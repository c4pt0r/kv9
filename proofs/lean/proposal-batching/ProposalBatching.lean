import Std

namespace Kv9.ProposalBatching

/-- Bounded proposal aggregation for public raw writes. The underlying
submission path's safety (persist-before-send, exact retry identity,
epoch fencing at apply) is a premise from the write-path design; here the
AGGREGATION is constrained: writes merge only under one equal region
fence (an epoch change closes the batch), the merged batch is proposed
as ONE entry and applied atomically at ONE position, every participant's
receipt is exactly that position (fanned out only after the apply), a
taken batch never admits another writer, and arrival order inside the
batch is never reordered. -/
structure State where
  opened : Bool := false
  taken : Bool := false
  proposed : Bool := false
  applied : Bool := false
  fanned : Bool := false
  crossEpochMerged : Bool := false
  joinedAfterTake : Bool := false
  reordered : Bool := false
  lostWrite : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | openBatch {s} : Step s {s with opened := true}
  | join {s} (o : s.opened = true) (live : s.taken = false) : Step s s
  | takeBatch {s} (o : s.opened = true) : Step s {s with taken := true}
  | propose {s} (t : s.taken = true) : Step s {s with proposed := true}
  | applyBatch {s} (p : s.proposed = true) : Step s {s with applied := true}
  | fanOut {s} (a : s.applied = true) : Step s {s with fanned := true}
  | retryIdentical {s} (p : s.proposed = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.fanned = true → s.applied = true) ∧
  (s.applied = true → s.proposed = true) ∧
  (s.proposed = true → s.taken = true) ∧
  (s.taken = true → s.opened = true) ∧
  s.crossEpochMerged = false ∧
  s.joinedAfterTake = false ∧
  s.reordered = false ∧
  s.lostWrite = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hF, hA, hP, hT, hX, hJ, hR, hL⟩ := safe
  cases step with
  | openBatch => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hX, hJ, hR, hL⟩
  | join o live => exact ⟨hF, hA, hP, hT, hX, hJ, hR, hL⟩
  | takeBatch o => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hX, hJ, hR, hL⟩
  | propose t => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hX, hJ, hR, hL⟩
  | applyBatch p => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hX, hJ, hR, hL⟩
  | fanOut a => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hX, hJ, hR, hL⟩
  | retryIdentical p => exact ⟨hF, hA, hP, hT, hX, hJ, hR, hL⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem receipts_fan_out_only_after_the_apply {s : State} (run : Reachable s)
    (fanned : s.fanned = true) : s.applied = true :=
  (run_safe run).1 fanned

theorem the_batch_applies_only_after_its_one_proposal {s : State}
    (run : Reachable s) (applied : s.applied = true) : s.proposed = true :=
  (run_safe run).2.1 applied

theorem the_proposal_requires_the_taken_batch {s : State} (run : Reachable s)
    (proposed : s.proposed = true) : s.taken = true :=
  (run_safe run).2.2.1 proposed

theorem no_cross_epoch_write_ever_merges {s : State} (run : Reachable s) :
    s.crossEpochMerged = false :=
  (run_safe run).2.2.2.2.1

theorem no_writer_joins_a_taken_batch {s : State} (run : Reachable s) :
    s.joinedAfterTake = false :=
  (run_safe run).2.2.2.2.2.1

theorem arrival_order_is_never_reordered {s : State} (run : Reachable s) :
    s.reordered = false :=
  (run_safe run).2.2.2.2.2.2.1

theorem no_write_is_ever_lost {s : State} (run : Reachable s) :
    s.lostWrite = false :=
  (run_safe run).2.2.2.2.2.2.2

theorem a_join_requires_a_live_open_batch {s : State}
    (o : s.opened = true) (live : s.taken = false) : Step s s :=
  .join o live

theorem a_retry_keeps_the_exact_identity {s : State}
    (p : s.proposed = true) : Step s s :=
  .retryIdentical p

theorem the_take_is_permanent {s t : State} (step : Step s t)
    (taken : s.taken = true) : t.taken = true := by
  cases step <;> simp_all

theorem an_open_batch_alone_proposes_nothing :
    ∃ s, Reachable s ∧ s.opened = true ∧ s.proposed = false := by
  refine ⟨⟨true, false, false, false, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .openBatch

theorem the_batched_chain_completes :
    ∃ s, Reachable s ∧ s.fanned = true ∧ s.applied = true ∧
      s.lostWrite = false := by
  refine ⟨⟨true, true, true, true, true, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .openBatch) (.takeBatch rfl))
    (.propose rfl)) (.applyBatch rfl)) (.fanOut rfl)

end Kv9.ProposalBatching
