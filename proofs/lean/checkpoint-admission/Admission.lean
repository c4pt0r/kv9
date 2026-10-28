import Std

namespace Kv9.CheckpointAdmission

inductive Phase where
  | absent | held | published | quiesced | released
  deriving DecidableEq

/-- One executor's projection of a shared gate, with arbitrary generations.
Atomic metadata apply and unchanged global coverage are composition premises.
Already admitted work, SDK cancellation and legacy backfill are not modeled. -/
structure State where
  closed : Bool := false
  phase : Phase := .absent
  generation : Nat := 0
  entries : Nat := 0
  publications : Nat := 0
  fenceEntries : Nat := 0
  fencePublications : Nat := 0
  covered : Bool := true

def Eligible (s : State) : Prop := s.phase = .absent ∨ s.phase = .released
def Valid (s : State) : Prop :=
  s.covered = true ∧
  (s.closed = true → s.entries = s.fenceEntries ∧ s.publications = s.fencePublications)

inductive Step : State → State → Prop where
  | enter (s) (g : Nat) (admitted : s.closed = false) (eligible : Eligible s)
      (next : g = s.generation + 1) :
      Step s {s with phase := .held, generation := g, entries := s.entries + 1}
  | publish (s) (admitted : s.closed = false)
      (entered : s.phase = .held ∨ s.phase = .published) :
      Step s {s with phase := .published, publications := s.publications + 1}
  | close (s) : Step s {s with closed := true, fenceEntries := s.entries, fencePublications := s.publications}
  | cancel (s) (g : Nat) (eligible : Eligible s) (next : g = s.generation + 1) :
      Step s {s with phase := .released, generation := g}
  | drain (s) (entered : s.phase = .held ∨ s.phase = .published) (covered : s.covered = true) :
      Step s {s with phase := .quiesced}
  | release (s) (drained : s.phase = .quiesced) : Step s {s with phase := .released}
  | unchanged (s) : Step s s

theorem initial_valid : Valid ({} : State) := by simp [Valid]

theorem step_preserves (h : Step s t) (good : Valid s) : Valid t := by
  cases h <;> simp_all [Valid]

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | next (before : Reachable s) (step : Step s t) : Reachable t

theorem reachable_valid (h : Reachable s) : Valid s := by
  induction h with
  | initial => exact initial_valid
  | next _ step ih => exact step_preserves step ih

theorem closed_step_blocks_entry_and_publish (h : Step s t) (closed : s.closed = true) :
    t.closed = true ∧ t.entries = s.entries ∧ t.publications = s.publications := by
  cases h <;> simp_all

theorem generations_never_decrease (h : Step s t) : s.generation ≤ t.generation := by
  cases h <;> simp_all

theorem entered_cannot_skip_drain (h : Step s t)
    (active : s.phase = .held ∨ s.phase = .published) : t.phase ≠ .released := by
  cases h <;> simp_all [Eligible]
  all_goals rcases active with h | h <;> (try simp_all)

theorem released_cannot_reenter_same_generation (h : Step s t)
    (released : s.phase = .released) (entered : t.phase = .held) :
    s.generation < t.generation := by
  cases h <;> simp_all [Eligible]

theorem closure_preserves_global_coverage (h : Reachable s) : s.covered = true := by
  exact (reachable_valid h).1

end Kv9.CheckpointAdmission
