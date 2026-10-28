import Std

namespace Kv9.CheckpointDrain

/-- One exact scope/generation. Each issued PUT owns one active attempt until
it receives an allowed single-attempt response or becomes permanently unknown.
This does not establish backend qualification or global publisher completeness. -/
structure State where
  jobs : Nat := 0
  active : Nat := 0
  issued : Nat := 0
  resolved : Nat := 0
  unknown : Nat := 0
  closed : Bool := false
  report : Option (Nat × Nat) := none

def Valid (s : State) : Prop :=
  s.issued = s.resolved + s.unknown + s.active ∧ s.active ≤ s.jobs ∧
  (∀ i r, s.report = some (i, r) →
    s.closed = true ∧ s.jobs = 0 ∧ i = s.issued ∧ r = s.resolved)

inductive Step : State → State → Prop where
  | enter (s) (admitted : s.closed = false) : Step s {s with jobs := s.jobs + 1}
  | issue (s) (job : s.active < s.jobs) :
      Step s {s with issued := s.issued + 1, active := s.active + 1}
  | reply (s) (active : s.active > 0) :
      Step s {s with resolved := s.resolved + 1, active := s.active - 1}
  | abandon (s) (active : s.active > 0) :
      Step s {s with unknown := s.unknown + 1, active := s.active - 1}
  | endJob (s) (idle : s.active < s.jobs) : Step s {s with jobs := s.jobs - 1}
  | close (s) : Step s {s with closed := true}
  | report (s) (closed : s.closed = true) (drained : s.jobs = 0) :
      Step s {s with report := some (s.issued, s.resolved)}
  | retryObservation (s) : Step s s

theorem initial_valid : Valid ({} : State) := by simp [Valid]

theorem step_preserves (h : Step s t) (good : Valid s) : Valid t := by
  rcases good with ⟨counts, capacity, stored⟩
  cases h with
  | enter admitted =>
    refine ⟨counts, by dsimp; omega, ?_⟩
    intro i r record
    have prior := stored i r record
    have impossible : (true : Bool) = false := prior.1.symm.trans admitted
    cases impossible
  | issue job =>
    refine ⟨by dsimp; omega, by dsimp; omega, ?_⟩
    intro i r record
    have zero := (stored i r record).2.1
    omega
  | reply active =>
    refine ⟨by dsimp; omega, by dsimp; omega, ?_⟩
    intro i r record
    have zero := (stored i r record).2.1
    omega
  | abandon active =>
    refine ⟨by dsimp; omega, by dsimp; omega, ?_⟩
    intro i r record
    have zero := (stored i r record).2.1
    omega
  | endJob idle =>
    refine ⟨counts, by dsimp; omega, ?_⟩
    intro i r record
    have zero := (stored i r record).2.1
    omega
  | close =>
    refine ⟨counts, capacity, ?_⟩
    intro i r record
    exact ⟨rfl, (stored i r record).2⟩
  | report closed drained =>
    refine ⟨counts, capacity, ?_⟩
    intro i r record
    have eq : s.issued = i ∧ s.resolved = r := by simpa using record
    exact ⟨closed, drained, eq.1.symm, eq.2.symm⟩
  | retryObservation => exact ⟨counts, capacity, stored⟩

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | next (before : Reachable s) (step : Step s t) : Reachable t

theorem reachable_valid (h : Reachable s) : Valid s := by
  induction h with
  | initial => exact initial_valid
  | next _ step ih => exact step_preserves step ih

theorem issued_never_decreases (h : Step s t) : s.issued ≤ t.issued := by
  cases h <;> simp

theorem unknown_never_decreases (h : Step s t) : s.unknown ≤ t.unknown := by
  cases h <;> simp

theorem report_requires_closed_drained_scope (h : Reachable s) (report : s.report = some (i, r)) :
    s.closed = true ∧ s.jobs = 0 ∧ s.active = 0 := by
  rcases reachable_valid h with ⟨counts, capacity, stored⟩
  rcases stored i r report with ⟨closed, jobs, issued, resolved⟩
  exact ⟨closed, jobs, by omega⟩

theorem zero_issued_report_has_no_write_debt (h : Reachable s) (report : s.report = some (0, r)) :
    s.issued = 0 ∧ s.unknown = 0 ∧ s.active = 0 := by
  rcases reachable_valid h with ⟨counts, capacity, stored⟩
  rcases stored 0 r report with ⟨closed, jobs, issued, resolved⟩
  omega

theorem equal_counts_report_has_no_unknown (h : Reachable s) (report : s.report = some (i, i)) :
    s.unknown = 0 := by
  rcases reachable_valid h with ⟨counts, capacity, stored⟩
  rcases stored i i report with ⟨closed, jobs, issued, resolved⟩
  omega

theorem drain_report_never_changes (h : Step s t) (good : Valid s) (report : s.report = some (i, r)) :
    t.report = some (i, r) := by
  have stored := good.2.2 i r report
  cases h <;> simp_all

theorem retry_success_does_not_erase_unknown (h : Step s t) (unknown : s.unknown > 0) :
    t.unknown > 0 := by
  have monotone := unknown_never_decreases h
  omega

end Kv9.CheckpointDrain
