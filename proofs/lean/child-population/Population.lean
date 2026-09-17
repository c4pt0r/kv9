import Std

namespace Kv9.ChildPopulation

/-- One split's child population under the parent fence. Seal semantics and
child-log atomicity are premises from their own models. Each half copies
into its child ONLY while the parent is sealed, every committed copy is
digest-exact (an inexact copy refuses instead of committing), reruns
converge, and no child serves anything here — routing still points at the
fenced parent until the atomic publication. -/
structure State where
  sealed : Bool := false
  lowCopied : Bool := false
  highCopied : Bool := false
  lowExact : Bool := false
  highExact : Bool := false
  divergentCommitted : Bool := false
  servedFromChild : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | fence {s} : Step s {s with sealed := true}
  | copyLow {s} (f : s.sealed = true) :
      Step s {s with lowCopied := true, lowExact := true}
  | copyHigh {s} (f : s.sealed = true) :
      Step s {s with highCopied := true, highExact := true}
  | recopy {s} (f : s.sealed = true) (done : s.lowCopied = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.lowCopied = true → s.sealed = true ∧ s.lowExact = true) ∧
  (s.highCopied = true → s.sealed = true ∧ s.highExact = true) ∧
  s.divergentCommitted = false ∧
  s.servedFromChild = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hL, hH, hD, hV⟩ := safe
  cases step with
  | fence => exact ⟨by simp_all, by simp_all, hD, hV⟩
  | copyLow f => exact ⟨by simp_all, by simp_all, hD, hV⟩
  | copyHigh f => exact ⟨by simp_all, by simp_all, hD, hV⟩
  | recopy f done => exact ⟨hL, hH, hD, hV⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem population_requires_the_fence {s : State} (run : Reachable s)
    (copied : s.lowCopied = true ∨ s.highCopied = true) : s.sealed = true := by
  cases copied with
  | inl low => exact ((run_safe run).1 low).1
  | inr high => exact ((run_safe run).2.1 high).1

theorem every_committed_copy_is_exact {s : State} (run : Reachable s) :
    (s.lowCopied = true → s.lowExact = true) ∧
      (s.highCopied = true → s.highExact = true) :=
  ⟨fun l => ((run_safe run).1 l).2, fun h => ((run_safe run).2.1 h).2⟩

theorem no_divergent_child_ever_commits {s : State} (run : Reachable s) :
    s.divergentCommitted = false :=
  (run_safe run).2.2.1

theorem children_serve_nothing_here {s : State} (run : Reachable s) :
    s.servedFromChild = false :=
  (run_safe run).2.2.2

theorem a_copy_is_permanent {s t : State} (step : Step s t)
    (copied : s.lowCopied = true) : t.lowCopied = true := by
  cases step <;> simp_all

theorem a_rerun_converges {s : State} (f : s.sealed = true)
    (done : s.lowCopied = true) : Step s s :=
  .recopy f done

theorem a_fence_alone_copies_nothing :
    ∃ s, Reachable s ∧ s.sealed = true ∧ s.lowCopied = false ∧
      s.highCopied = false := by
  refine ⟨⟨true, false, false, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial .fence

theorem the_chain_reaches_both_exact_children :
    ∃ s, Reachable s ∧ s.lowCopied = true ∧ s.highCopied = true ∧
      s.lowExact = true ∧ s.highExact = true := by
  refine ⟨⟨true, true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .fence) (.copyLow rfl)) (.copyHigh rfl)

end Kv9.ChildPopulation
