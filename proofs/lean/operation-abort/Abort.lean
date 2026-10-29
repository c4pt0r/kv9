import Std

namespace Kv9.OperationAbort

/-- One split or merge intent and its abort (docs/RANGE-SPLIT.md §12). `aborts` counts
abort rows for the operation, `sealEvidence` the committed seal evidence rows,
`published` the publication; `parentMoves` counts membership changes accepted on the
parent (allowed only while no live intent binds it); `childrenRetired` says the
abandoned children retired. -/
structure State where
  aborts : Nat := 0
  sealEvidence : Nat := 0
  published : Bool := false
  parentMoves : Nat := 0
  childrenRetired : Bool := false
  deriving DecidableEq, Repr

/-- The catalog's rule: an abort row exists and no seal evidence names the operation. -/
def Effective (s : State) : Prop := 0 < s.aborts ∧ s.sealEvidence = 0

inductive Step : State → State → Prop where
  /-- The abort planner: refuses once any seal evidence exists; a repeat confirms. -/
  | abort {s} (u : s.sealEvidence = 0) (p : s.published = false) :
      Step s {s with aborts := if s.aborts = 0 then 1 else s.aborts}
  /-- A parent leader seals and commits the evidence that makes the operation
  irrevocable. Taken from a state with an abort, this is the one race: the abort is
  then read as ineffective by everyone. -/
  | sealParent {s} (p : s.published = false) : Step s {s with sealEvidence := s.sealEvidence + 1}
  /-- Publication: never for an effective abort. -/
  | publish {s} (e : ¬ Effective s) : Step s {s with published := true}
  /-- A move on the parent: only once the abort freed it. -/
  | moveParent {s} (e : Effective s) : Step s {s with parentMoves := s.parentMoves + 1}
  /-- Every node retires the abandoned children on an effective abort. -/
  | retireChildren {s} (e : Effective s) : Step s {s with childrenRetired := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.aborts ≤ 1 ∧ (s.published = true → ¬ Effective s) ∧
    (0 < s.parentMoves → 0 < s.aborts) ∧ (s.childrenRetired = true → 0 < s.aborts) ∧
    (s.published = true → s.childrenRetired = true → 0 < s.sealEvidence)

theorem initial_safe : Safe {} := by simp [Safe, Effective]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hA, hP, hM, hC, hPC⟩ := safe
  cases step with
  | abort u p =>
      refine ⟨?_, fun h => by simp_all, fun m => ?_, fun c => ?_, fun h => by simp_all⟩
      · by_cases h : s.aborts = 0 <;> simp [h] <;> omega
      · have := hM m; by_cases h : s.aborts = 0 <;> simp [h] <;> omega
      · have := hC c; by_cases h : s.aborts = 0 <;> simp [h] <;> omega
  | sealParent p =>
      refine ⟨hA, fun h => by simp_all, hM, hC, fun h => by simp_all⟩
  | publish e =>
      refine ⟨hA, fun _ => e, hM, hC, fun _ c => ?_⟩
      have := hC c
      show 0 < s.sealEvidence
      by_cases z : s.sealEvidence = 0
      · exact absurd ⟨this, z⟩ e
      · omega
  | moveParent e =>
      refine ⟨hA, hP, fun _ => e.1, hC, hPC⟩
  | retireChildren e =>
      refine ⟨hA, hP, hM, fun _ => e.1, fun h _ => ?_⟩
      exact absurd e (hP h)

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem one_abort_per_operation {s : State} (run : Reachable s) : s.aborts ≤ 1 := (run_safe run).1

-- A published operation's abort is ineffective.
theorem published_means_the_abort_is_ineffective {s : State} (run : Reachable s) (p : s.published = true) :
    ¬ Effective s :=
  (run_safe run).2.1 p

-- The parent's membership moves only after an abort freed it.
theorem a_move_needs_the_abort {s : State} (run : Reachable s) (m : 0 < s.parentMoves) : 0 < s.aborts :=
  (run_safe run).2.2.1 m

theorem children_retire_only_on_abort {s : State} (run : Reachable s) (c : s.childrenRetired = true) :
    0 < s.aborts :=
  (run_safe run).2.2.2.1 c

-- An abort after seal evidence is refused by the planner.
theorem an_abort_is_refused_after_a_seal {s t : State} (step : Step s t) (grew : s.aborts < t.aborts) :
    s.sealEvidence = 0 := by
  cases step <;> simp_all

-- The one race, decided by the catalog: a seal that lands after an abort makes it ineffective.
theorem a_late_seal_makes_the_abort_ineffective {s t : State} (step : Step s t)
    (sealed : s.sealEvidence < t.sealEvidence) : ¬ Effective t := by
  cases step <;> simp_all [Effective]

theorem an_abort_frees_the_parent :
    ∃ s, Reachable s ∧ Effective s ∧ 0 < s.parentMoves ∧ s.childrenRetired = true := by
  refine ⟨⟨1, 0, false, 1, true⟩, ?_, ⟨by decide, rfl⟩, by decide, rfl⟩
  have a : Reachable ⟨1, 0, false, 0, false⟩ := .step .initial (.abort rfl rfl)
  have m : Reachable ⟨1, 0, false, 1, false⟩ := .step a (.moveParent (by simp [Effective]))
  exact .step m (.retireChildren (by simp [Effective]))

end Kv9.OperationAbort
