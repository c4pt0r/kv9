import Std

namespace Kv9.StorageReclamation

/-- Physical reclamation of a RETIRED local group's payload. Retirement's
own safety (committed removal / published split authority, the durable
Retired fence) is a premise from its models; here the DELETION is
constrained: it happens only after the durable Reclaimed record commits
it, that record itself requires the retired fence AND the re-verified
committed authority, the fence is never lost (the record survives every
deletion), active leaves are never touched, a reclaimed group never
serves again, and a crash between the record and the deletion resumes
idempotently. -/
structure State where
  retired : Bool := false
  authorityVerified : Bool := false
  recordDurable : Bool := false
  payloadDeleted : Bool := false
  fenceLost : Bool := false
  leafTouched : Bool := false
  served : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | retire {s} : Step s {s with retired := true}
  | verifyAuthority {s} (r : s.retired = true) :
      Step s {s with authorityVerified := true}
  | commitRecord {s} (r : s.retired = true) (a : s.authorityVerified = true) :
      Step s {s with recordDurable := true}
  | deletePayload {s} (rec : s.recordDurable = true) :
      Step s {s with payloadDeleted := true}
  | resumeAfterCrash {s} (rec : s.recordDurable = true) : Step s s
  | confirm {s} (p : s.payloadDeleted = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.payloadDeleted = true → s.recordDurable = true) ∧
  (s.recordDurable = true → s.retired = true ∧ s.authorityVerified = true) ∧
  s.fenceLost = false ∧
  s.leafTouched = false ∧
  s.served = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hP, hR, hF, hL, hS⟩ := safe
  cases step with
  | retire => exact ⟨by simp_all, by simp_all, hF, hL, hS⟩
  | verifyAuthority r => exact ⟨by simp_all, by simp_all, hF, hL, hS⟩
  | commitRecord r a => exact ⟨by simp_all, by simp_all, hF, hL, hS⟩
  | deletePayload rec => exact ⟨by simp_all, by simp_all, hF, hL, hS⟩
  | resumeAfterCrash rec => exact ⟨hP, hR, hF, hL, hS⟩
  | confirm p => exact ⟨hP, hR, hF, hL, hS⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem deletion_requires_the_durable_record {s : State} (run : Reachable s)
    (deleted : s.payloadDeleted = true) : s.recordDurable = true :=
  (run_safe run).1 deleted

theorem the_durable_record_requires_the_retired_fence {s : State}
    (run : Reachable s) (rec : s.recordDurable = true) : s.retired = true :=
  ((run_safe run).2.1 rec).1

theorem the_durable_record_requires_committed_authority {s : State}
    (run : Reachable s) (rec : s.recordDurable = true) :
    s.authorityVerified = true :=
  ((run_safe run).2.1 rec).2

theorem the_fence_is_never_lost {s : State} (run : Reachable s) :
    s.fenceLost = false :=
  (run_safe run).2.2.1

theorem no_active_leaf_is_ever_touched {s : State} (run : Reachable s) :
    s.leafTouched = false :=
  (run_safe run).2.2.2.1

theorem a_reclaimed_group_never_serves {s : State} (run : Reachable s) :
    s.served = false :=
  (run_safe run).2.2.2.2

theorem deletion_is_permanent {s t : State} (step : Step s t)
    (deleted : s.payloadDeleted = true) : t.payloadDeleted = true := by
  cases step <;> simp_all

theorem a_crash_after_the_record_resumes {s : State}
    (rec : s.recordDurable = true) : Step s s :=
  .resumeAfterCrash rec

theorem a_retired_group_alone_deletes_nothing :
    ∃ s, Reachable s ∧ s.retired = true ∧ s.recordDurable = false ∧
      s.payloadDeleted = false := by
  refine ⟨⟨true, false, false, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step .initial .retire

theorem the_authorized_chain_completes :
    ∃ s, Reachable s ∧ s.payloadDeleted = true ∧ s.fenceLost = false ∧
      s.served = false := by
  refine ⟨⟨true, true, true, true, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .retire) (.verifyAuthority rfl))
    (.commitRecord rfl rfl)) (.deletePayload rfl)

end Kv9.StorageReclamation
