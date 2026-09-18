import Std

namespace Kv9.MigrationAbort

/-- The committed migration abort: the stranded-destination recovery.
Each underlying mechanism's safety (the migration intent, install
evidence, the retention ledger's pin discipline, learner attach) is a
premise from its own model; here the SETTLEMENT algebra is constrained:
evidence and abort are mutually exclusive per operation — permanently, in
both directions — the source pin releases only against one committed
settlement, the stranded learner detaches and the region re-migrates only
under the committed abort, an aborted operation never adopts, and the
destination image pin never drops. -/
structure State where
  migrated : Bool := false
  evidenced : Bool := false
  aborted : Bool := false
  released : Bool := false
  detached : Bool := false
  remigrated : Bool := false
  adoptedAfterAbort : Bool := false
  destinationPinDropped : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | migrate {s} : Step s {s with migrated := true}
  | evidence {s} (m : s.migrated = true) (clean : s.aborted = false) :
      Step s {s with evidenced := true}
  | abort {s} (m : s.migrated = true) (clean : s.evidenced = false) :
      Step s {s with aborted := true}
  | release {s} (settled : s.evidenced = true ∨ s.aborted = true) :
      Step s {s with released := true}
  | detach {s} (a : s.aborted = true) : Step s {s with detached := true}
  | remigrate {s} (a : s.aborted = true) : Step s {s with remigrated := true}
  | confirmAbort {s} (a : s.aborted = true) : Step s s

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  ¬(s.evidenced = true ∧ s.aborted = true) ∧
  (s.released = true → s.evidenced = true ∨ s.aborted = true) ∧
  (s.detached = true → s.aborted = true) ∧
  (s.remigrated = true → s.aborted = true) ∧
  s.adoptedAfterAbort = false ∧
  s.destinationPinDropped = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hX, hR, hD, hM, hA, hP⟩ := safe
  cases step with
  | migrate => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | evidence m clean => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | abort m clean => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | release settled => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | detach a => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | remigrate a => exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, hA, hP⟩
  | confirmAbort a => exact ⟨hX, hR, hD, hM, hA, hP⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem evidence_and_abort_never_coexist {s : State} (run : Reachable s) :
    ¬(s.evidenced = true ∧ s.aborted = true) :=
  (run_safe run).1

theorem release_requires_one_committed_settlement {s : State}
    (run : Reachable s) (released : s.released = true) :
    s.evidenced = true ∨ s.aborted = true :=
  (run_safe run).2.1 released

theorem detach_requires_the_committed_abort {s : State} (run : Reachable s)
    (detached : s.detached = true) : s.aborted = true :=
  (run_safe run).2.2.1 detached

theorem remigration_requires_the_committed_abort {s : State}
    (run : Reachable s) (fresh : s.remigrated = true) : s.aborted = true :=
  (run_safe run).2.2.2.1 fresh

theorem no_aborted_operation_ever_adopts {s : State} (run : Reachable s) :
    s.adoptedAfterAbort = false :=
  (run_safe run).2.2.2.2.1

theorem the_destination_pin_never_drops {s : State} (run : Reachable s) :
    s.destinationPinDropped = false :=
  (run_safe run).2.2.2.2.2

theorem an_abort_is_permanent {s t : State} (step : Step s t)
    (a : s.aborted = true) : t.aborted = true := by
  cases step <;> simp_all

theorem a_confirmation_changes_nothing {s : State} (a : s.aborted = true) :
    Step s s :=
  .confirmAbort a

theorem a_migration_alone_settles_nothing :
    ∃ s, Reachable s ∧ s.migrated = true ∧ s.evidenced = false ∧
      s.aborted = false ∧ s.released = false := by
  refine ⟨⟨true, false, false, false, false, false, false, false⟩, ?_, rfl, rfl, rfl, rfl⟩
  exact .step .initial .migrate

theorem the_stranded_chain_recovers :
    ∃ s, Reachable s ∧ s.aborted = true ∧ s.released = true ∧
      s.detached = true ∧ s.remigrated = true ∧
      s.destinationPinDropped = false := by
  refine ⟨⟨true, false, true, true, true, true, false, false⟩, ?_, rfl, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step .initial .migrate)
    (.abort rfl rfl)) (.release (Or.inr rfl))) (.detach rfl)) (.remigrate rfl)

end Kv9.MigrationAbort
