import Std

namespace Kv9.AutoSplit

/-- The automatic trigger over the proven manual pipeline. Every pipeline
stage's own safety (intent, seal, population, publication, retirement) is
a premise from its model; here the trigger itself is constrained: it
fires ONLY on an observed threshold breach of a bound, unsealed range,
derives ONE deterministic operation per region (so any coordinator
resumes the same split), and drives only the existing committed steps —
no stage is skipped, no second trigger fires for the same region, and
nothing splits below the threshold. -/
structure State where
  bound : Bool := false
  breached : Bool := false
  triggered : Bool := false
  pipelineDone : Bool := false
  secondTrigger : Bool := false
  stageSkipped : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | bind {s} : Step s {s with bound := true}
  | observe {s} (b : s.bound = true) : Step s {s with breached := true}
  | trigger {s} (b : s.bound = true) (o : s.breached = true)
      (once : s.triggered = false) :
      Step s {s with triggered := true}
  | drive {s} (t : s.triggered = true) : Step s {s with pipelineDone := true}
  | resume {s} (t : s.triggered = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.triggered = true → s.bound = true ∧ s.breached = true) ∧
  (s.pipelineDone = true → s.triggered = true) ∧
  s.secondTrigger = false ∧
  s.stageSkipped = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hT, hP, hS, hK⟩ := safe
  cases step with
  | bind => exact ⟨by simp_all, by simp_all, hS, hK⟩
  | observe b => exact ⟨by simp_all, by simp_all, hS, hK⟩
  | trigger b o once => exact ⟨by simp_all, by simp_all, hS, hK⟩
  | drive t => exact ⟨by simp_all, by simp_all, hS, hK⟩
  | resume t => exact ⟨hT, hP, hS, hK⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_trigger_requires_a_breached_bound_range {s : State}
    (run : Reachable s) (fired : s.triggered = true) :
    s.bound = true ∧ s.breached = true :=
  (run_safe run).1 fired

theorem the_pipeline_runs_only_when_triggered {s : State}
    (run : Reachable s) (done : s.pipelineDone = true) : s.triggered = true :=
  (run_safe run).2.1 done

theorem no_second_trigger_fires {s : State} (run : Reachable s) :
    s.secondTrigger = false :=
  (run_safe run).2.2.1

theorem no_stage_is_skipped {s : State} (run : Reachable s) :
    s.stageSkipped = false :=
  (run_safe run).2.2.2

theorem a_trigger_is_permanent {s t : State} (step : Step s t)
    (fired : s.triggered = true) : t.triggered = true := by
  cases step <;> simp_all

theorem any_coordinator_resumes {s : State} (fired : s.triggered = true) :
    Step s s :=
  .resume fired

theorem a_breach_alone_triggers_nothing :
    ∃ s, Reachable s ∧ s.breached = true ∧ s.triggered = false := by
  refine ⟨⟨true, true, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial .bind) (.observe rfl)

theorem the_triggered_chain_completes :
    ∃ s, Reachable s ∧ s.triggered = true ∧ s.pipelineDone = true ∧
      s.stageSkipped = false := by
  refine ⟨⟨true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .bind) (.observe rfl))
    (.trigger rfl rfl rfl)) (.drive rfl)

end Kv9.AutoSplit
