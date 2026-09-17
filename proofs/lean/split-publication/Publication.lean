import Std

namespace Kv9.SplitPublication

/-- The manual split's finale: the atomic one-to-two publication. Intent,
fence, population and partition-directory semantics are premises from
their own models. Publication happens ONLY with the committed intent, the
durable fence and BOTH digest-verified children; it is one atomic
transition that flips routing to the children exactly when the directory
commits; it never publishes partially, never loses a row, and confirms
idempotently ever after. -/
structure State where
  intent : Bool := false
  fenced : Bool := false
  lowExact : Bool := false
  highExact : Bool := false
  published : Bool := false
  routedToChildren : Bool := false
  partialPublish : Bool := false
  lostData : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with intent := true}
  | fence {s} (i : s.intent = true) : Step s {s with fenced := true}
  | verifyLow {s} (f : s.fenced = true) : Step s {s with lowExact := true}
  | verifyHigh {s} (f : s.fenced = true) : Step s {s with highExact := true}
  | publish {s} (i : s.intent = true) (f : s.fenced = true)
      (lo : s.lowExact = true) (hi : s.highExact = true)
      (fresh : s.published = false) :
      Step s {s with published := true, routedToChildren := true}
  | confirm {s} (p : s.published = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.published = true →
    s.intent = true ∧ s.fenced = true ∧ s.lowExact = true ∧ s.highExact = true) ∧
  s.routedToChildren = s.published ∧
  s.partialPublish = false ∧
  s.lostData = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hR, hX, hL⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, by simp_all, hX, hL⟩
  | fence i => exact ⟨by simp_all, by simp_all, hX, hL⟩
  | verifyLow f => exact ⟨by simp_all, by simp_all, hX, hL⟩
  | verifyHigh f => exact ⟨by simp_all, by simp_all, hX, hL⟩
  | publish i f lo hi fresh => exact ⟨by simp_all, by simp_all, hX, hL⟩
  | confirm p => exact ⟨hP, hR, hX, hL⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem publication_requires_the_full_chain {s : State} (run : Reachable s)
    (published : s.published = true) :
    s.intent = true ∧ s.fenced = true ∧ s.lowExact = true ∧
      s.highExact = true :=
  (run_safe run).1 published

theorem routing_flips_exactly_at_publication {s : State} (run : Reachable s) :
    s.routedToChildren = s.published :=
  (run_safe run).2.1

theorem no_partial_publication_and_no_lost_row {s : State} (run : Reachable s) :
    s.partialPublish = false ∧ s.lostData = false :=
  ⟨(run_safe run).2.2.1, (run_safe run).2.2.2⟩

theorem publication_is_permanent {s t : State} (step : Step s t)
    (published : s.published = true) : t.published = true := by
  cases step <;> simp_all

theorem a_confirmation_changes_nothing {s : State}
    (published : s.published = true) : Step s s :=
  .confirm published

theorem verified_population_alone_publishes_nothing :
    ∃ s, Reachable s ∧ s.lowExact = true ∧ s.highExact = true ∧
      s.published = false ∧ s.routedToChildren = false := by
  refine ⟨⟨true, true, true, true, false, false, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .commit) (.fence rfl))
    (.verifyLow rfl)) (.verifyHigh rfl)

theorem the_chain_reaches_the_published_split :
    ∃ s, Reachable s ∧ s.published = true ∧ s.routedToChildren = true ∧
      s.lostData = false := by
  refine ⟨⟨true, true, true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .commit) (.fence rfl))
    (.verifyLow rfl)) (.verifyHigh rfl)) (.publish rfl rfl rfl rfl rfl)

end Kv9.SplitPublication
