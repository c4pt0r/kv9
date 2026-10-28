import Std

namespace Kv9.CheckpointUnpublished

inductive Phase where
  | absent | held | published | quiesced | released
  deriving DecidableEq

/-- Projection for one immutable root/owner/generation token. New generations
are different tokens, never reset this projection. Exact Raft receipts and
all uploaders requiring Publish are composition premises. -/
structure State where
  closed : Bool := false
  phase : Phase := .absent
  published : Bool := false
  receipt : Bool := false
  used : Bool := false
  retired : Bool := false
  covered : Bool := true

def Valid (s : State) : Prop :=
  s.covered = true ∧
  ((s.phase = .absent ∨ s.phase = .held) → s.published = false) ∧
  ((s.receipt = true ∨ s.used = true) → s.published = true) ∧
  (s.retired = true → s.closed = true ∧ s.phase = .released ∧
    s.published = false ∧ s.receipt = false ∧ s.used = false)

inductive Step : State → State → Prop where
  | enter (s) (available : s.phase = .absent) (admitted : s.closed = false) :
      Step s {s with phase := .held}
  | publish (s) (held : s.phase = .held ∨ s.phase = .published) (admitted : s.closed = false) :
      Step s {s with phase := .published, published := true}
  | receipt (s) (committed : s.published = true) : Step s {s with receipt := true}
  | use (s) (confirmed : s.receipt = true) : Step s {s with used := true}
  | close (s) : Step s {s with closed := true}
  | retire (s) (closed : s.closed = true) (held : s.phase = .held) :
      Step s {s with phase := .released, retired := true}
  | drain (s) (active : s.phase = .held ∨ s.phase = .published) : Step s {s with phase := .quiesced}
  | release (s) (drained : s.phase = .quiesced) : Step s {s with phase := .released}
  | cancel (s) (absent : s.phase = .absent) : Step s {s with phase := .released}
  | restart (s) : Step s {s with receipt := false}
  | unchanged (s) : Step s s

theorem initial_valid : Valid ({} : State) := by simp [Valid]

theorem step_preserves (h : Step s t) (good : Valid s) : Valid t := by
  cases h <;> simp_all [Valid]
  all_goals cases a : s.receipt <;> cases b : s.used <;> cases c : s.retired <;>
    cases d : s.phase <;> (try simp_all)

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | next (before : Reachable s) (step : Step s t) : Reachable t

theorem reachable_valid (h : Reachable s) : Valid s := by
  induction h with
  | initial => exact initial_valid
  | next _ step ih => exact step_preserves step ih

theorem retired_never_authorized_or_used (h : Reachable s) (retired : s.retired = true) :
    s.published = false ∧ s.receipt = false ∧ s.used = false := by
  exact ((reachable_valid h).2.2.2 retired).2.2

theorem retired_has_closed_gate_and_terminal_phase (h : Reachable s) (retired : s.retired = true) :
    s.closed = true ∧ s.phase = .released := by
  have proof := (reachable_valid h).2.2.2 retired
  exact ⟨proof.1, proof.2.1⟩

theorem closed_never_reopens (h : Step s t) (closed : s.closed = true) : t.closed = true := by
  cases h <;> simp_all

theorem retirement_evidence_never_disappears (h : Step s t) (retired : s.retired = true) :
    t.retired = true := by
  cases h <;> simp_all

theorem published_receipt_excludes_retirement (h : Reachable s) (receipt : s.receipt = true) :
    s.retired = false := by
  have good := reachable_valid h
  cases eq : s.retired <;> simp_all [Valid]

theorem retirement_keeps_global_protection (h : Reachable s) : s.covered = true := by
  exact (reachable_valid h).1

end Kv9.CheckpointUnpublished
