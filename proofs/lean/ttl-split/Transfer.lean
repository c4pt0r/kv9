import Std

namespace Kv9.TtlSplit

/-- One exact, externally authenticated import specification. The hash is
opaque: this model proves its completion check, not collision resistance. -/
structure Spec where
  floor : Nat
  rows : Nat
  digest : Nat
  seed : Nat
structure State where
  copied : Nat
  chain : Nat
  clock : Nat
  complete : Bool
  bound : Bool

def initial (p : Spec) : State := ⟨0, p.seed, p.floor, false, false⟩
def Valid (p : Spec) (s : State) : Prop :=
  s.copied ≤ p.rows ∧ p.floor ≤ s.clock ∧
  (s.complete = false → s.clock = p.floor) ∧
  (s.complete = true → s.bound = true ∧ s.copied = p.rows ∧ s.chain = p.digest) ∧
  (s.bound = true → s.complete = true)

inductive Step (p : Spec) : State → State → Prop where
  | page (s) (rows hash : Nat) (copying : s.complete = false)
      (within : s.copied + rows ≤ p.rows) :
      Step p s {s with copied := s.copied + rows, chain := hash}
  | finish (s) (count : s.copied = p.rows) (digest : s.chain = p.digest) :
      Step p s {s with complete := true, bound := true}
  | observe (s) (now : Nat) (ready : s.complete = true) :
      Step p s {s with clock := max s.clock now}
  | retryOrRefusal (s) : Step p s s

theorem initial_valid : Valid p (initial p) := by simp [Valid, initial]

theorem step_preserves (h : Step p s t) (good : Valid p s) : Valid p t := by
  rcases good with ⟨limit, clock, fixed, finished, bound⟩
  cases h with
  | page rows hash copying within =>
    refine ⟨within, clock, fixed, ?_, ?_⟩
    · intro impossible; simp_all
    · intro published; have prior := bound published; simp_all
  | finish count digest => exact ⟨limit, clock, by simp, by simp_all, by simp⟩
  | observe now ready =>
    refine ⟨limit, by dsimp; omega, ?_, finished, bound⟩
    intro impossible; simp_all
  | retryOrRefusal => exact ⟨limit, clock, fixed, finished, bound⟩

inductive Reachable (p : Spec) : State → Prop where
  | initial : Reachable p (initial p)
  | next (prior : Reachable p s) (step : Step p s t) : Reachable p t

theorem reachable_valid (h : Reachable p s) : Valid p s := by
  induction h with
  | initial => exact initial_valid
  | next _ step ih => exact step_preserves step ih

theorem watermark_never_regresses (h : Step p s t) : s.clock ≤ t.clock := by
  cases h <;> simp <;> omega

theorem importing_keeps_source_watermark (h : Reachable p s) (copying : s.complete = false) :
    s.clock = p.floor := (reachable_valid h).2.2.1 copying

theorem publication_requires_complete_commitment (h : Reachable p s) (bound : s.bound = true) :
    s.complete = true ∧ s.copied = p.rows ∧ s.chain = p.digest := by
  have valid := reachable_valid h
  have done := valid.2.2.2.2 bound
  exact ⟨done, (valid.2.2.2.1 done).2⟩

theorem completed_cursor_cannot_change (h : Step p s t) (done : s.complete = true) :
    t.copied = s.copied ∧ t.chain = s.chain := by
  cases h <;> simp_all

theorem completion_is_permanent (h : Step p s t) (done : s.complete = true) : t.complete = true := by
  cases h <;> simp_all

theorem copied_count_never_regresses (h : Step p s t) : s.copied ≤ t.copied := by
  cases h <;> simp

end Kv9.TtlSplit
