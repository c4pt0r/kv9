import Std

namespace Kv9.CheckpointSweep

/-- One already-closed attempt, projected onto its ordered ID domain. `bound`
is the exclusive end of that domain, not a production owner-count limit.
The complete indexed page supplies the clear interval. New Cancel rows never
add Held; neither this model nor a cursor certifies legacy backfill. -/
structure State where
  held : Nat → Bool
  frontier : Nat := 0
  sequence : Nat := 0
  done : Bool := false
  published : Bool := true
  covered : Bool := true

def Valid (bound : Nat) (s : State) : Prop :=
  s.frontier ≤ bound ∧
  (∀ i, i < s.frontier → s.held i = false) ∧
  (s.done = true → s.frontier = bound) ∧ s.covered = true

inductive Step (bound : Nat) : State → State → Prop where
  | retire (s) (i : Nat) :
      Step bound s {s with held := fun j => if j = i then false else s.held j}
  | advance (s) (expected next visits : Nat)
      (cas : expected = s.sequence) (active : s.done = false)
      (forward : s.frontier ≤ next) (endBound : next ≤ bound)
      (budget : visits ≤ 4)
      (pageClear : ∀ i, s.frontier ≤ i → i < next → s.held i = false) :
      Step bound s {s with frontier := next, sequence := expected + 1, done := decide (next = bound)}
  | drainPublished (s) : Step bound s {s with published := false}
  | cancelUnentered (s) : Step bound s s
  | lostAck (s) : Step bound s s
  | restart (s) : Step bound s s

theorem initial_valid (bound : Nat) (held : Nat → Bool) :
    Valid bound {held := held} := by simp [Valid]

theorem step_preserves (h : Step bound s t) (good : Valid bound s) : Valid bound t := by
  rcases good with ⟨bounded, clearPrefix, complete, covered⟩
  cases h with
  | retire i =>
    refine ⟨bounded, ?_, complete, covered⟩
    intro j before
    simp only
    split
    · rfl
    · exact clearPrefix j before
  | advance expected next visits cas active forward endBound budget pageClear =>
    refine ⟨endBound, ?_, ?_, covered⟩
    · intro i before
      by_cases previous : i < s.frontier
      · exact clearPrefix i previous
      · exact pageClear i (by omega) before
    · simp
  | drainPublished => exact ⟨bounded, clearPrefix, complete, covered⟩
  | cancelUnentered => exact ⟨bounded, clearPrefix, complete, covered⟩
  | lostAck => exact ⟨bounded, clearPrefix, complete, covered⟩
  | restart => exact ⟨bounded, clearPrefix, complete, covered⟩

inductive Reachable (bound : Nat) : State → Prop where
  | initial (held : Nat → Bool) : Reachable bound {held := held}
  | next (before : Reachable bound s) (step : Step bound s t) : Reachable bound t

theorem reachable_valid (h : Reachable bound s) : Valid bound s := by
  induction h with
  | initial held => exact initial_valid bound held
  | next _ step ih => exact step_preserves step ih

theorem passed_prefix_has_no_held (h : Reachable bound s) (before : i < s.frontier) :
    s.held i = false := (reachable_valid h).2.1 i before

theorem done_excludes_indexed_held (h : Reachable bound s) (done : s.done = true)
    (inside : i < bound) : s.held i = false := by
  have good := reachable_valid h
  exact good.2.1 i (by rw [good.2.2.1 done]; exact inside)

theorem cursor_never_moves_backwards (h : Step bound s t) : s.frontier ≤ t.frontier := by
  cases h <;> simp_all

theorem sequence_never_moves_backwards (h : Step bound s t) : s.sequence ≤ t.sequence := by
  cases h <;> simp_all

theorem sweep_keeps_global_protection (h : Reachable bound s) : s.covered = true :=
  (reachable_valid h).2.2.2

/-- An explicit reachable completed sweep with an outstanding publisher.
This counterexample prevents strengthening Done into a drain certificate. -/
theorem done_allows_published : ∃ s, Reachable 1 s ∧ s.done = true ∧ s.published = true := by
  let initial : State := {held := fun _ => false}
  let final := {initial with frontier := 1, sequence := 1, done := true}
  refine ⟨final, ?_, rfl, rfl⟩
  apply Reachable.next (Reachable.initial (fun _ => false))
  exact Step.advance (bound := 1) initial 0 1 1 rfl rfl (by decide) (by decide) (by decide)
    (by intros; rfl)

end Kv9.CheckpointSweep
