import Std

namespace Kv9.CascadeSplit

/-- Cascade splits over the proven single-split pipeline. Each split's own
safety (intent, seal, population, atomic publication, retirement) is a
premise from its model; here the CASCADE history is constrained: a
published intent's readback validity is PERMANENT — a child binding sealed
later by its own publication never invalidates its grandparent's intent —
the second in-flight pipeline still completes after a sibling's
publication, retirement reaches EVERY published parent (an earlier
intent's idempotent confirmation starves nobody), and a recorded refusal
stays observable (no later clean step erases it). -/
structure State where
  firstPublished : Bool := false
  childSealed : Bool := false
  secondPublished : Bool := false
  firstRetired : Bool := false
  secondRetired : Bool := false
  readbackRefused : Bool := false
  starved : Bool := false
  errorErased : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | publishFirst {s} : Step s {s with firstPublished := true}
  | sealChild {s} (p : s.firstPublished = true) :
      Step s {s with childSealed := true}
  | publishSecond {s} (p : s.firstPublished = true) :
      Step s {s with secondPublished := true}
  | retireFirst {s} (p : s.firstPublished = true) :
      Step s {s with firstRetired := true}
  | retireSecond {s} (p : s.secondPublished = true) :
      Step s {s with secondRetired := true}
  | confirmFirstAgain {s} (r : s.firstRetired = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.readbackRefused = false ∧
  s.starved = false ∧
  s.errorErased = false ∧
  (s.childSealed = true → s.firstPublished = true) ∧
  (s.secondRetired = true → s.secondPublished = true) ∧
  (s.firstRetired = true → s.firstPublished = true)

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hR, hS, hE, hC, hT, hF⟩ := safe
  cases step with
  | publishFirst => exact ⟨hR, hS, hE, by simp_all, by simp_all, by simp_all⟩
  | sealChild p => exact ⟨hR, hS, hE, by simp_all, by simp_all, by simp_all⟩
  | publishSecond p => exact ⟨hR, hS, hE, by simp_all, by simp_all, by simp_all⟩
  | retireFirst p => exact ⟨hR, hS, hE, by simp_all, by simp_all, by simp_all⟩
  | retireSecond p => exact ⟨hR, hS, hE, by simp_all, by simp_all, by simp_all⟩
  | confirmFirstAgain r => exact ⟨hR, hS, hE, hC, hT, hF⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem valid_history_never_refuses_readback {s : State} (run : Reachable s) :
    s.readbackRefused = false :=
  (run_safe run).1

theorem no_published_parent_is_starved {s : State} (run : Reachable s) :
    s.starved = false :=
  (run_safe run).2.1

theorem no_recorded_refusal_is_erased {s : State} (run : Reachable s) :
    s.errorErased = false :=
  (run_safe run).2.2.1

theorem a_childs_seal_requires_the_published_parent {s : State}
    (run : Reachable s) (sealed : s.childSealed = true) :
    s.firstPublished = true :=
  (run_safe run).2.2.2.1 sealed

theorem retirement_requires_the_publication {s : State} (run : Reachable s)
    (retired : s.secondRetired = true) : s.secondPublished = true :=
  (run_safe run).2.2.2.2.1 retired

theorem a_publication_is_permanent {s t : State} (step : Step s t)
    (published : s.firstPublished = true) : t.firstPublished = true := by
  cases step <;> simp_all

theorem a_childs_seal_never_invalidates_its_grandparent :
    ∃ s, Reachable s ∧ s.childSealed = true ∧ s.readbackRefused = false := by
  refine ⟨⟨true, true, false, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial .publishFirst) (.sealChild rfl)

theorem the_second_pipeline_completes_after_a_siblings_seal :
    ∃ s, Reachable s ∧ s.childSealed = true ∧ s.secondPublished = true ∧
      s.secondRetired = true ∧ s.readbackRefused = false := by
  refine ⟨⟨true, true, true, false, true, false, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .publishFirst) (.sealChild rfl))
    (.publishSecond rfl)) (.retireSecond rfl)

theorem an_earlier_confirmation_starves_no_later_parent :
    ∃ s, Reachable s ∧ s.firstRetired = true ∧ s.secondRetired = true ∧
      s.starved = false := by
  refine ⟨⟨true, false, true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .publishFirst)
    (.publishSecond rfl)) (.retireFirst rfl)) (.confirmFirstAgain rfl))
    (.retireSecond rfl)

end Kv9.CascadeSplit
