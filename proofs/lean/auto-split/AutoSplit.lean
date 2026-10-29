import Std

namespace Kv9.AutoSplit

/-- One data group as its leader's trigger sees it (docs/RANGE-SPLIT.md §11). `bytes` is
the replica footprint, `threshold` the configured trigger; `leader` the leader role;
`bound`/`sealed` the binding; `busy` a live split, merge or unsettled plan on the group;
`cooled` the node's cooldown; `rows` the sampled row count; `intents` the committed split
intents for this binding; `keyInside` whether the decided key lies strictly inside. -/
structure State where
  bytes : Nat := 0
  threshold : Nat := 4
  leader : Bool := true
  bound : Bool := true
  sealed : Bool := false
  busy : Bool := false
  cooled : Bool := true
  rows : Nat := 2
  intents : Nat := 0
  keyInside : Bool := true
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | write {s} : Step s {s with bytes := s.bytes + 1, rows := s.rows + 1}
  | loseLeadership {s} : Step s {s with leader := false}
  | lead {s} : Step s {s with leader := true}
  | otherOperation {s} : Step s {s with busy := true}
  | settle {s} : Step s {s with busy := false}
  | heat {s} : Step s {s with cooled := false}
  | cool {s} : Step s {s with cooled := true}
  /-- The decision: leader, bound and unsealed, nothing else live on the group, cooled,
  the footprint at or above the threshold, at least two rows so a median lies strictly
  inside; the derived operation makes a repeat a confirmation. -/
  | decide {s} (l : s.leader = true) (b : s.bound = true) (u : s.sealed = false) (q : s.busy = false)
      (c : s.cooled = true) (t : s.threshold ≤ s.bytes) (r : 2 ≤ s.rows) :
      Step s {s with intents := if s.intents = 0 then 1 else s.intents, keyInside := true, cooled := false}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop := s.intents ≤ 1 ∧ s.keyInside = true

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hI, hK⟩ := safe
  cases step with
  | decide l b u q c t r =>
      refine ⟨?_, rfl⟩
      by_cases h : s.intents = 0 <;> simp [h] <;> omega
  | _ => exact ⟨hI, hK⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- One intent per binding: a repeated decision confirms the derived row.
theorem one_intent_per_binding {s : State} (run : Reachable s) : s.intents ≤ 1 := (run_safe run).1

theorem a_repeated_decision_confirms {s t : State} (step : Step s t) (had : s.intents = 1) : t.intents = 1 := by
  cases step <;> simp_all

-- A decision needs every guard: leadership, a bound unsealed quiet group, the cooldown,
-- the threshold and enough rows for a median strictly inside.
theorem a_decision_needs_every_guard {s t : State} (step : Step s t) (fresh : s.intents = 0)
    (decided : t.intents = 1) :
    s.leader = true ∧ s.bound = true ∧ s.sealed = false ∧ s.busy = false ∧ s.cooled = true ∧
      s.threshold ≤ s.bytes ∧ 2 ≤ s.rows := by
  cases step <;> simp_all

-- Deciding heats the node: two decisions never follow each other without a cooldown.
theorem a_decision_starts_the_cooldown {s t : State} (step : Step s t) (fresh : s.intents = 0)
    (decided : t.intents = 1) : t.cooled = false := by
  cases step <;> simp_all

theorem the_key_is_inside {s : State} (run : Reachable s) : s.keyInside = true := (run_safe run).2

-- Sustained writes alone reach a decision.
theorem writes_alone_reach_a_decision :
    ∃ s, Reachable s ∧ s.intents = 1 ∧ s.threshold ≤ s.bytes := by
  refine ⟨⟨4, 4, true, true, false, false, false, 6, 1, true⟩, ?_, rfl, by decide⟩
  have w : Reachable ⟨4, 4, true, true, false, false, true, 6, 0, true⟩ :=
    .step (.step (.step (.step .initial .write) .write) .write) .write
  exact .step w (.decide rfl rfl rfl rfl rfl (by decide) (by decide))

end Kv9.AutoSplit
