import Std

namespace Kv9.InstallEvidence

/-- One migration operation's settlement: from committed migration and
published image pins, through the destination's committed install-evidence
row, to the source pin's quiesce and release. Catalog row immutability,
ledger atomicity and adoption exactness are premises from their own models.
The committed evidence row is the ONLY key that unlocks the migration
quiesce fence, and the destination pin — the live replica's protection —
has no dropping step. `sourcePhase` uses the ledger's own numbering:
0 absent, 2 published, 3 quiesced, 4 released. -/
structure State where
  migration : Bool := false
  destinationPinned : Bool := false
  adopted : Bool := false
  evidence : Bool := false
  sourcePhase : Nat := 0
  wrongSubject : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | commit {s} : Step s {s with migration := true}
  | pin {s} (m : s.migration = true) (fresh : s.sourcePhase = 0) :
      Step s {s with sourcePhase := 2, destinationPinned := true}
  | adopt {s} (m : s.migration = true) (p : s.destinationPinned = true) :
      Step s {s with adopted := true}
  | record {s} (a : s.adopted = true) (pub : s.sourcePhase = 2) :
      Step s {s with evidence := true}
  | quiesce {s} (e : s.evidence = true) (pub : s.sourcePhase = 2)
      (pinned : s.destinationPinned = true) :
      Step s {s with sourcePhase := 3}
  | release {s} (q : s.sourcePhase = 3) :
      Step s {s with sourcePhase := 4}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.evidence = true → s.adopted = true ∧ s.destinationPinned = true) ∧
  (s.adopted = true → s.migration = true ∧ s.destinationPinned = true) ∧
  (3 ≤ s.sourcePhase → s.evidence = true) ∧
  (s.sourcePhase ≠ 0 → s.migration = true ∧ s.destinationPinned = true) ∧
  (s.destinationPinned = true → s.sourcePhase ≠ 0) ∧
  (s.sourcePhase = 0 ∨ s.sourcePhase = 2 ∨ s.sourcePhase = 3 ∨ s.sourcePhase = 4) ∧
  s.wrongSubject = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hE, hA, hQ, hP, hD, hRange, hW⟩ := safe
  cases step with
  | commit => exact ⟨by simp_all, by simp_all, hQ, by simp_all, hD, hRange, hW⟩
  | pin m fresh =>
      refine ⟨by simp_all, by simp_all, ?_, by simp_all, by simp, by simp, hW⟩
      intro h
      have : (3 : Nat) ≤ 2 := h
      omega
  | adopt m p => exact ⟨by simp_all, by simp_all, hQ, hP, hD, hRange, hW⟩
  | record a pub =>
      exact ⟨by simp_all, hA, by simp_all, hP, hD, hRange, hW⟩
  | quiesce e pub pinned =>
      exact ⟨by simp_all, by simp_all, by simp_all, by simp_all, by simp, by simp, hW⟩
  | release q =>
      refine ⟨by simp_all, by simp_all, ?_, by simp_all, by simp, by simp, hW⟩
      intro _
      exact hQ (by omega)

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem quiesce_and_release_require_committed_evidence {s : State}
    (run : Reachable s) (settled : 3 ≤ s.sourcePhase) : s.evidence = true :=
  (run_safe run).2.2.1 settled

theorem evidence_requires_adoption_under_the_pins {s : State}
    (run : Reachable s) (recorded : s.evidence = true) :
    s.adopted = true ∧ s.destinationPinned = true :=
  (run_safe run).1 recorded

theorem adoption_requires_the_committed_migration_and_pins {s : State}
    (run : Reachable s) (adopted : s.adopted = true) :
    s.migration = true ∧ s.destinationPinned = true :=
  (run_safe run).2.1 adopted

theorem a_pinned_operation_names_a_committed_migration {s : State}
    (run : Reachable s) (pinned : s.sourcePhase ≠ 0) : s.migration = true :=
  ((run_safe run).2.2.2.1 pinned).1

theorem no_divergent_subject_ever_commits {s : State} (run : Reachable s) :
    s.wrongSubject = false :=
  (run_safe run).2.2.2.2.2.2

theorem the_destination_pin_never_drops {s t : State} (step : Step s t)
    (pinned : s.destinationPinned = true) : t.destinationPinned = true := by
  cases step <;> simp_all

theorem the_source_phase_never_regresses {s t : State} (step : Step s t) :
    s.sourcePhase ≤ t.sourcePhase := by
  cases step <;> simp_all <;> omega

theorem evidence_is_permanent {s t : State} (step : Step s t)
    (recorded : s.evidence = true) : t.evidence = true := by
  cases step <;> simp_all

theorem an_adopted_replica_alone_releases_nothing :
    ∃ s, Reachable s ∧ s.adopted = true ∧ s.evidence = false ∧
      s.sourcePhase = 2 := by
  refine ⟨⟨true, true, true, false, 2, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step .initial .commit) (.pin rfl rfl)) (.adopt rfl rfl)

theorem the_committed_chain_reaches_release :
    ∃ s, Reachable s ∧ s.sourcePhase = 4 ∧ s.evidence = true ∧
      s.destinationPinned = true := by
  refine ⟨⟨true, true, true, true, 4, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step (.step .initial .commit)
    (.pin rfl rfl)) (.adopt rfl rfl)) (.record rfl rfl))
    (.quiesce rfl rfl rfl)) (.release rfl)

end Kv9.InstallEvidence
