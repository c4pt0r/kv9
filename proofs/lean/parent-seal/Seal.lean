import Std

namespace Kv9.ParentSeal

/-- One split parent's group-side write fence. Split-intent authority and
group-log atomicity are premises from their own models. The committed
intent is the ONE authority for sealing the parent's own range row; the
seal is one-way and durable across restarts; a sealed parent serves
nothing; and the catalog directory is untouched here — its atomic
one-to-two publication is a later increment's only transition. -/
structure State where
  intent : Bool := false
  sealed : Bool := false
  servedWhileSealed : Bool := false
  catalogChanged : Bool := false
  unsealedAgain : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with intent := true}
  | fence {s} (i : s.intent = true) (fresh : s.sealed = false) :
      Step s {s with sealed := true}
  | restart {s} (r : s.sealed = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.sealed = true → s.intent = true) ∧
  s.servedWhileSealed = false ∧
  s.catalogChanged = false ∧
  s.unsealedAgain = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hI, hV, hC, hU⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, hV, hC, hU⟩
  | fence i fresh => exact ⟨by simp_all, hV, hC, hU⟩
  | restart r => exact ⟨hI, hV, hC, hU⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_seal_requires_the_committed_intent {s : State}
    (run : Reachable s) (fenced : s.sealed = true) : s.intent = true :=
  (run_safe run).1 fenced

theorem nothing_serves_through_a_seal {s : State} (run : Reachable s) :
    s.servedWhileSealed = false :=
  (run_safe run).2.1

theorem the_catalog_is_untouched_here {s : State} (run : Reachable s) :
    s.catalogChanged = false :=
  (run_safe run).2.2.1

theorem the_seal_never_reverts {s : State} (run : Reachable s) :
    s.unsealedAgain = false :=
  (run_safe run).2.2.2

theorem a_seal_is_permanent {s t : State} (step : Step s t)
    (fenced : s.sealed = true) : t.sealed = true := by
  cases step <;> simp_all

theorem restart_preserves_the_fence {s : State} (fenced : s.sealed = true) :
    Step s s :=
  .restart fenced

theorem an_intent_alone_fences_nothing :
    ∃ s, Reachable s ∧ s.intent = true ∧ s.sealed = false := by
  refine ⟨⟨true, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step .initial .commit

theorem the_committed_chain_reaches_the_fence :
    ∃ s, Reachable s ∧ s.sealed = true ∧ s.catalogChanged = false := by
  refine ⟨⟨true, true, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial .commit) (.fence rfl rfl)

end Kv9.ParentSeal
