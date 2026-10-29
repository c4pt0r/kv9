import Std

namespace Kv9.RangeMerge

/-- One merge of two adjacent sibling ranges into one child (docs/RANGE-SPLIT.md §6).
`adjacent` says `low.end = high.start` in one keyspace, `sameMembers` that both parents
and the child live on the same committed members, `bothUnsealed` that no other operation
sealed a parent; `intents` counts merge intents for this pair; `childServes` the child's
leader and descriptor; `sealedLow`/`sealedHigh` the two seals; `copiedLow`/`copiedHigh`
the copied rows against `low`/`high`; `verified`, `evidence`, `published`, `routed` as in
the split model. -/
structure State where
  adjacent : Bool := true
  sameMembers : Bool := true
  bothUnsealed : Bool := true
  intents : Nat := 0
  childServes : Bool := false
  sealedLow : Bool := false
  sealedHigh : Bool := false
  low : Nat := 2
  high : Nat := 2
  copiedLow : Nat := 0
  copiedHigh : Nat := 0
  verified : Bool := false
  evidence : Nat := 0
  published : Bool := false
  routed : Bool := false
  retired : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | notAdjacent {s} : Step s {s with adjacent := false}
  | otherStores {s} : Step s {s with sameMembers := false}
  | foreignSeal {s} : Step s {s with bothUnsealed := false}
  /-- The intent: adjacent, same members, both unsealed, once per pair. -/
  | intend {s} (a : s.adjacent = true) (m : s.sameMembers = true) (u : s.bothUnsealed = true) :
      Step s {s with intents := if s.intents = 0 then 1 else s.intents}
  | serve {s} (i : s.intents = 1) : Step s {s with childServes := true}
  | sealLow {s} (c : s.childServes = true) : Step s {s with sealedLow := true}
  | sealHigh {s} (c : s.childServes = true) : Step s {s with sealedHigh := true}
  /-- Copy pages: the low parent first, only sealed parents, never past their size. -/
  | copyLow {s} (d : s.sealedLow = true) (e : s.sealedHigh = true) (more : s.copiedLow < s.low) :
      Step s {s with copiedLow := s.copiedLow + 1}
  | copyHigh {s} (d : s.sealedLow = true) (e : s.sealedHigh = true) (lowDone : s.copiedLow = s.low)
      (more : s.copiedHigh < s.high) : Step s {s with copiedHigh := s.copiedHigh + 1}
  | verify {s} (d : s.sealedLow = true) (e : s.sealedHigh = true) (l : s.copiedLow = s.low)
      (h : s.copiedHigh = s.high) : Step s {s with verified := true}
  | evidence {s} (v : s.verified = true) :
      Step s {s with evidence := if s.evidence = 0 then 1 else s.evidence}
  | publish {s} (e : s.evidence = 1) : Step s {s with published := true, routed := true}
  | retire {s} (p : s.published = true) : Step s {s with retired := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.intents ≤ 1 ∧ s.evidence ≤ 1 ∧ (s.evidence = 1 → s.verified = true) ∧
    (s.verified = true → s.sealedLow = true ∧ s.sealedHigh = true ∧ s.copiedLow = s.low ∧ s.copiedHigh = s.high) ∧
    s.copiedLow ≤ s.low ∧ s.copiedHigh ≤ s.high ∧ (0 < s.copiedHigh → s.copiedLow = s.low) ∧
    (s.published = true → s.evidence = 1) ∧ s.routed = s.published ∧
    ((s.sealedLow = true ∨ s.sealedHigh = true) → s.childServes = true) ∧
    (s.childServes = true → s.intents = 1) ∧ (s.retired = true → s.published = true)

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, hS, hC, hT⟩ := safe
  cases step with
  | notAdjacent => exact ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, hS, hC, hT⟩
  | otherStores => exact ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, hS, hC, hT⟩
  | foreignSeal => exact ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, hS, hC, hT⟩
  | intend a m u =>
      refine ⟨?_, hE, hEV, hV, hL, hH, hO, hP, hR, hS, fun c => ?_, hT⟩
      · by_cases h : s.intents = 0 <;> simp [h] <;> omega
      · have := hC c
        simp [this]
  | serve i => exact ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, fun _ => rfl, fun _ => i, hT⟩
  | sealLow c => exact ⟨hI, hE, hEV, fun v => ⟨rfl, (hV v).2.1, (hV v).2.2.1, (hV v).2.2.2⟩, hL, hH, hO, hP, hR, fun _ => c, hC, hT⟩
  | sealHigh c => exact ⟨hI, hE, hEV, fun v => ⟨(hV v).1, rfl, (hV v).2.2.1, (hV v).2.2.2⟩, hL, hH, hO, hP, hR, fun _ => c, hC, hT⟩
  | copyLow d e more =>
      refine ⟨hI, hE, hEV, fun v => ?_, by simp; omega, hH, fun o => ?_, hP, hR, hS, hC, hT⟩
      · have := (hV v).2.2.1; simp_all
      · have := hO o; simp_all
  | copyHigh d e lowDone more =>
      exact ⟨hI, hE, hEV, fun v => by have := (hV v).2.2.2; simp_all, hL, by simp; omega, fun _ => lowDone, hP, hR, hS, hC, hT⟩
  | verify d e l h => exact ⟨hI, hE, fun _ => rfl, fun _ => ⟨d, e, l, h⟩, hL, hH, hO, hP, hR, hS, hC, hT⟩
  | evidence v =>
      refine ⟨hI, ?_, fun _ => v, hV, hL, hH, hO, fun p => ?_, hR, hS, hC, hT⟩
      · by_cases h : s.evidence = 0 <;> simp [h] <;> omega
      · have := hP p; simp [this]
  | publish e => exact ⟨hI, hE, hEV, hV, hL, hH, hO, fun _ => e, rfl, hS, hC, fun _ => rfl⟩
  | retire p => exact ⟨hI, hE, hEV, hV, hL, hH, hO, hP, hR, hS, hC, fun _ => p⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem one_intent_per_pair {s : State} (run : Reachable s) : s.intents ≤ 1 := (run_safe run).1

theorem an_intent_needs_adjacent_unsealed_siblings_on_one_store_set {s t : State} (step : Step s t)
    (fresh : s.intents = 0) (intended : t.intents = 1) :
    s.adjacent = true ∧ s.sameMembers = true ∧ s.bothUnsealed = true := by
  cases step <;> simp_all

-- Evidence means both parents sealed and both copied whole.
theorem evidence_means_both_halves_verified {s : State} (run : Reachable s) (e : s.evidence = 1) :
    s.sealedLow = true ∧ s.sealedHigh = true ∧ s.copiedLow = s.low ∧ s.copiedHigh = s.high :=
  (run_safe run).2.2.2.1 ((run_safe run).2.2.1 e)

-- The copy order: the low parent is exhausted before the high one starts.
theorem the_low_parent_is_copied_first {s : State} (run : Reachable s) (h : 0 < s.copiedHigh) :
    s.copiedLow = s.low :=
  (run_safe run).2.2.2.2.2.2.1 h

theorem a_copy_reads_only_sealed_parents {s t : State} (step : Step s t)
    (progress : s.copiedLow < t.copiedLow ∨ s.copiedHigh < t.copiedHigh) :
    s.sealedLow = true ∧ s.sealedHigh = true := by
  cases step <;> simp_all

theorem a_seal_waits_for_the_child {s : State} (run : Reachable s)
    (d : s.sealedLow = true ∨ s.sealedHigh = true) : s.childServes = true :=
  (run_safe run).2.2.2.2.2.2.2.2.2.1 d

theorem publication_needs_the_evidence {s : State} (run : Reachable s) (p : s.published = true) :
    s.evidence = 1 :=
  (run_safe run).2.2.2.2.2.2.2.1 p

theorem routing_follows_publication_atomically {s : State} (run : Reachable s) :
    s.routed = s.published :=
  (run_safe run).2.2.2.2.2.2.2.2.1

theorem retirement_needs_publication {s : State} (run : Reachable s) (r : s.retired = true) :
    s.published = true :=
  (run_safe run).2.2.2.2.2.2.2.2.2.2.2 r

theorem the_chain_reaches_a_published_merge :
    ∃ s, Reachable s ∧ s.published = true ∧ s.routed = true ∧ s.copiedLow = s.low ∧ s.copiedHigh = s.high := by
  refine ⟨⟨true, true, true, 1, true, true, true, 2, 2, 2, 2, true, 1, true, true, true⟩, ?_, rfl, rfl, rfl, rfl⟩
  have i : Reachable ⟨true, true, true, 1, false, false, false, 2, 2, 0, 0, false, 0, false, false, false⟩ :=
    .step .initial (.intend rfl rfl rfl)
  have sv : Reachable ⟨true, true, true, 1, true, false, false, 2, 2, 0, 0, false, 0, false, false, false⟩ := .step i (.serve rfl)
  have sl : Reachable ⟨true, true, true, 1, true, true, false, 2, 2, 0, 0, false, 0, false, false, false⟩ := .step sv (.sealLow rfl)
  have sh : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 0, 0, false, 0, false, false, false⟩ := .step sl (.sealHigh rfl)
  have l1 : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 1, 0, false, 0, false, false, false⟩ := .step sh (.copyLow rfl rfl (by decide))
  have l2 : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 0, false, 0, false, false, false⟩ := .step l1 (.copyLow rfl rfl (by decide))
  have h1 : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 1, false, 0, false, false, false⟩ := .step l2 (.copyHigh rfl rfl rfl (by decide))
  have h2 : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 2, false, 0, false, false, false⟩ := .step h1 (.copyHigh rfl rfl rfl (by decide))
  have v : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 2, true, 0, false, false, false⟩ := .step h2 (.verify rfl rfl rfl rfl)
  have e : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 2, true, 1, false, false, false⟩ := .step v (.evidence rfl)
  have p : Reachable ⟨true, true, true, 1, true, true, true, 2, 2, 2, 2, true, 1, true, true, false⟩ := .step e (.publish rfl)
  exact .step p (.retire rfl)

end Kv9.RangeMerge
