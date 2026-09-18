import Std

namespace Kv9.AdmissionFloor

/-- The metadata admission floor. The admission ledger's own accounting
(bounded counts and bytes, release on drop) is a premise from its
implementation; here the FLOOR is constrained: raw and transaction load
can fill only the SHARED capacity — the reserved slots are never
consumed by a non-metadata class — a saturated shared pool refuses
non-metadata work with a TYPED pre-append refusal (nothing proposed,
nothing queued), metadata work stays admissible up to the FULL limit
throughout the flood, and releasing shared capacity reopens it. -/
structure State where
  sharedFull : Bool := false
  metadataAdmitted : Bool := false
  totalFull : Bool := false
  floorConsumedByRaw : Bool := false
  metadataStarved : Bool := false
  untypedRefusal : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | rawAdmit {s} (open_ : s.sharedFull = false) : Step s s
  | floodFillsShared {s} : Step s {s with sharedFull := true}
  | rawRefusedTyped {s} (full : s.sharedFull = true) : Step s s
  | metaAdmit {s} (capacity : s.totalFull = false) :
      Step s {s with metadataAdmitted := true}
  | metaFillsTotal {s} (m : s.metadataAdmitted = true)
      (f : s.sharedFull = true) : Step s {s with totalFull := true}
  | metaRefusedAtTotal {s} (full : s.totalFull = true) : Step s s
  | release {s} : Step s {s with sharedFull := false, totalFull := false}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.totalFull = true → s.sharedFull = true) ∧
  s.floorConsumedByRaw = false ∧
  s.metadataStarved = false ∧
  s.untypedRefusal = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hT, hF, hM, hU⟩ := safe
  cases step with
  | rawAdmit open_ => exact ⟨hT, hF, hM, hU⟩
  | floodFillsShared => exact ⟨by simp_all, hF, hM, hU⟩
  | rawRefusedTyped full => exact ⟨hT, hF, hM, hU⟩
  | metaAdmit capacity => exact ⟨by simp_all, hF, hM, hU⟩
  | metaFillsTotal m f => exact ⟨by simp_all, hF, hM, hU⟩
  | metaRefusedAtTotal full => exact ⟨hT, hF, hM, hU⟩
  | release => exact ⟨by simp_all, hF, hM, hU⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem no_raw_work_ever_consumes_the_floor {s : State} (run : Reachable s) :
    s.floorConsumedByRaw = false :=
  (run_safe run).2.1

theorem metadata_is_never_starved {s : State} (run : Reachable s) :
    s.metadataStarved = false :=
  (run_safe run).2.2.1

theorem every_refusal_is_typed_and_pre_append {s : State} (run : Reachable s) :
    s.untypedRefusal = false :=
  (run_safe run).2.2.2

theorem the_total_fills_only_beyond_the_shared_pool {s : State}
    (run : Reachable s) (full : s.totalFull = true) : s.sharedFull = true :=
  (run_safe run).1 full

theorem a_saturated_shared_pool_refuses_raw_work_typed {s : State}
    (full : s.sharedFull = true) : Step s s :=
  .rawRefusedTyped full

theorem metadata_admits_while_capacity_remains {s : State}
    (capacity : s.totalFull = false) : Step s {s with metadataAdmitted := true} :=
  .metaAdmit capacity

theorem a_release_reopens_the_shared_pool {s : State} :
    Step s {s with sharedFull := false, totalFull := false} :=
  .release

theorem the_flood_cannot_reach_the_total_limit_alone {s t : State}
    (step : Step s t) (before : s.totalFull = false)
    (meta_idle : s.metadataAdmitted = false) : t.totalFull = false := by
  cases step <;> simp_all

theorem metadata_admits_through_a_full_shared_pool :
    ∃ s, Reachable s ∧ s.sharedFull = true ∧ s.metadataAdmitted = true ∧
      s.metadataStarved = false := by
  refine ⟨⟨true, true, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step .initial .floodFillsShared) (.metaAdmit rfl)

theorem only_metadata_consumes_the_last_capacity :
    ∃ s, Reachable s ∧ s.totalFull = true ∧ s.metadataAdmitted = true ∧
      s.floorConsumedByRaw = false := by
  refine ⟨⟨true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .floodFillsShared) (.metaAdmit rfl))
    (.metaFillsTotal rfl rfl)

theorem the_released_pool_admits_raw_work_again :
    ∃ s, Reachable s ∧ s.sharedFull = false ∧ Step s s := by
  refine ⟨⟨false, false, false, false, false, false⟩, ?_, rfl, .rawAdmit rfl⟩
  exact .step (.step .initial .floodFillsShared) .release

end Kv9.AdmissionFloor
