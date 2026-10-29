import Std

namespace Kv9.RangeSplitPopulation

/-- One split after its intent, as the driver sees it (docs/RANGE-SPLIT.md §3–§4).
`childrenServe` says both children have a leader and an applied half descriptor;
`sealed` the parent's descriptor; `copied` how many rows a child has copied, `half` how
many the sealed parent half holds; `verified` that the child's rows equal the half;
`evidence` counts committed evidence rows for the child; `published` and `routed` are
part 4, never touched here; `parentWrites` counts writes the parent accepted after the
seal (must stay zero). -/
structure State where
  childrenServe : Bool := false
  sealed : Bool := false
  half : Nat := 3
  copied : Nat := 0
  verified : Bool := false
  evidence : Nat := 0
  published : Bool := false
  routed : Bool := false
  parentWrites : Nat := 0
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | serve {s} : Step s {s with childrenServe := true}
  /-- The seal: only once both children serve. -/
  | sealParent {s} (c : s.childrenServe = true) : Step s {s with sealed := true}
  /-- A write the parent would accept: only while unsealed. -/
  | parentWrite {s} (u : s.sealed = false) :
      Step s {s with half := s.half + 1, parentWrites := if s.sealed then s.parentWrites + 1 else s.parentWrites}
  /-- One page copied from the sealed parent half into the child. -/
  | copy {s} (d : s.sealed = true) (more : s.copied < s.half) : Step s {s with copied := s.copied + 1}
  /-- Verification: the half is exhausted and the rows match. -/
  | verify {s} (d : s.sealed = true) (all : s.copied = s.half) : Step s {s with verified := true}
  /-- Evidence: only after verification, once per child (a repeat confirms). -/
  | evidence {s} (v : s.verified = true) :
      Step s {s with evidence := if s.evidence = 0 then 1 else s.evidence}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.parentWrites = 0 ∧ s.evidence ≤ 1 ∧ (s.evidence = 1 → s.verified = true) ∧
    (s.verified = true → s.sealed = true ∧ s.copied = s.half) ∧ s.copied ≤ s.half ∧
    s.published = false ∧ s.routed = false ∧ (s.sealed = true → s.childrenServe = true)

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hW, hE, hEV, hV, hC, hP, hR, hS⟩ := safe
  cases step with
  | serve => exact ⟨hW, hE, hEV, hV, hC, hP, hR, fun _ => rfl⟩
  | sealParent c => exact ⟨hW, hE, hEV, fun v => ⟨rfl, (hV v).2⟩, hC, hP, hR, fun _ => c⟩
  | parentWrite u =>
      refine ⟨by simp [u, hW], hE, hEV, ?_, by simp; omega, hP, hR, hS⟩
      intro v
      have := (hV v).1
      simp_all
  | copy d more => exact ⟨hW, hE, hEV, fun v => by have := (hV v).2; simp_all, by simp; omega, hP, hR, hS⟩
  | verify d all => exact ⟨hW, hE, fun _ => rfl, fun _ => ⟨d, all⟩, hC, hP, hR, hS⟩
  | evidence v =>
      refine ⟨hW, ?_, fun _ => v, hV, hC, hP, hR, hS⟩
      by_cases h : s.evidence = 0 <;> simp [h] <;> omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- S5: evidence exists only for a verified copy of the whole sealed half.
theorem evidence_means_a_verified_copy {s : State} (run : Reachable s) (e : s.evidence = 1) :
    s.sealed = true ∧ s.copied = s.half := by
  have safe := run_safe run
  exact safe.2.2.2.1 (safe.2.2.1 e)

theorem one_evidence_per_child {s : State} (run : Reachable s) : s.evidence ≤ 1 :=
  (run_safe run).2.1

-- The parent accepts nothing after the seal: the copy reads an immutable source.
theorem the_sealed_parent_accepts_no_write {s : State} (run : Reachable s) : s.parentWrites = 0 :=
  (run_safe run).1

theorem a_copy_reads_only_a_sealed_parent {s t : State} (step : Step s t)
    (progress : s.copied < t.copied) : s.sealed = true := by
  cases step <;> simp_all

-- The seal waits for both children to serve.
theorem the_seal_waits_for_the_children {s : State} (run : Reachable s) (d : s.sealed = true) :
    s.childrenServe = true :=
  (run_safe run).2.2.2.2.2.2.2 d

-- Parts 2–3 publish and route nothing.
theorem nothing_is_published_or_routed_here {s : State} (run : Reachable s) :
    s.published = false ∧ s.routed = false :=
  ⟨(run_safe run).2.2.2.2.2.1, (run_safe run).2.2.2.2.2.2.1⟩

-- The chain reaches evidence for a half of three rows.
theorem the_chain_reaches_evidence :
    ∃ s, Reachable s ∧ s.evidence = 1 ∧ s.sealed = true ∧ s.copied = s.half := by
  refine ⟨⟨true, true, 3, 3, true, 1, false, false, 0⟩, ?_, rfl, rfl, rfl⟩
  have a : Reachable ⟨true, false, 3, 0, false, 0, false, false, 0⟩ := .step .initial .serve
  have b : Reachable ⟨true, true, 3, 0, false, 0, false, false, 0⟩ := .step a (.sealParent rfl)
  have c1 : Reachable ⟨true, true, 3, 1, false, 0, false, false, 0⟩ := .step b (.copy rfl (by decide))
  have c2 : Reachable ⟨true, true, 3, 2, false, 0, false, false, 0⟩ := .step c1 (.copy rfl (by decide))
  have c3 : Reachable ⟨true, true, 3, 3, false, 0, false, false, 0⟩ := .step c2 (.copy rfl (by decide))
  have v : Reachable ⟨true, true, 3, 3, true, 0, false, false, 0⟩ := .step c3 (.verify rfl rfl)
  exact .step v (.evidence rfl)

end Kv9.RangeSplitPopulation
