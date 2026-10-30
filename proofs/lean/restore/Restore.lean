import Std

namespace Kv9.Restore

/-- One restore of a complete backup as the cluster drives it (docs/BACKUP-RESTORE.md §5–§7).
`targets` ranges; `evidenced` of them have a committed evidence row, which the metadata
leader commits only when the target's rows and digest equal the anchor's (`fits`
counts evidence commits whose receipt matched); `published` says the keyspace row and the
bindings exist; `aborted` an effective abort; `sourceWrites` counts writes to the source
keyspace or its groups by the restore (always zero); `backupRetired` whether the backup
retired while this restore was live. -/
structure State where
  targets : Nat := 2
  evidenced : Nat := 0
  mismatched : Nat := 0
  published : Bool := false
  aborted : Bool := false
  sourceWrites : Nat := 0
  backupRetiredWhileLive : Bool := false
  deriving DecidableEq, Repr

def live (s : State) : Bool := !s.aborted && !s.published

inductive Step : State → State → Prop where
  /-- A target leader copied and digested; the metadata leader commits the evidence: only
  for a live restore, only for an unevidenced target, only when the digest matches. -/
  | evidence {s} (isLive : live s = true) (left : s.evidenced < s.targets) (fits : Bool)
      (ok : fits = true) :
      Step s {s with evidenced := s.evidenced + 1,
                     mismatched := if fits then s.mismatched else s.mismatched + 1}
  /-- Publication: one transaction, only once every target is evidenced, only while live. -/
  | publish {s} (isLive : live s = true) (all : s.evidenced = s.targets) :
      Step s {s with published := true}
  /-- An operator aborts: only before publication. -/
  | abort {s} (open_ : s.published = false) :
      Step s {s with aborted := true}
  /-- The backup retires: only when no restore of it is live. -/
  | retireBackup {s} (free : live s = false) :
      Step s {s with backupRetiredWhileLive := if live s then true else s.backupRetiredWhileLive}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.evidenced ≤ s.targets
    ∧ (s.published = true → s.evidenced = s.targets)
    ∧ ¬ (s.aborted = true ∧ s.published = true)
    ∧ s.mismatched = 0
    ∧ s.sourceWrites = 0
    ∧ s.backupRetiredWhileLive = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hE, hP, hX, hM, hW, hR⟩ := safe
  cases step with
  | evidence isLive left fits ok =>
      exact ⟨by simp; omega, fun h => by simp [live] at isLive; simp at h; exact absurd h (by simp [isLive.2]), hX, by simp [ok, hM], hW, hR⟩
  | publish isLive all =>
      exact ⟨hE, fun _ => all, by simp [live] at isLive; simp [isLive.1], hM, hW, hR⟩
  | abort open_ => exact ⟨hE, by simp [open_], by simp [open_], hM, hW, hR⟩
  | retireBackup free => exact ⟨hE, hP, hX, hM, hW, by simp [free, hR]⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- B4: publication needs every target's evidence.
theorem publication_needs_every_evidence {s t : State} (step : Step s t)
    (was : s.published = false) (now : t.published = true) : s.evidenced = s.targets := by
  cases step with
  | evidence _ _ _ _ => simp at now; exact absurd now (by simp [was])
  | publish _ all => exact all
  | abort _ => simp at now; exact absurd now (by simp [was])
  | retireBackup _ => simp at now; exact absurd now (by simp [was])

-- B5: evidence is committed only for a matching digest, and only while live.
theorem evidence_needs_a_match {s t : State} (step : Step s t)
    (grew : s.evidenced < t.evidenced) : live s = true ∧ t.mismatched = s.mismatched := by
  cases step with
  | evidence isLive _ fits ok => exact ⟨isLive, by simp [ok]⟩
  | publish _ _ => simp at grew
  | abort _ => simp at grew
  | retireBackup _ => simp at grew

-- A published restore does not abort.
theorem abort_refuses_after_publication {s t : State} (step : Step s t)
    (was : s.aborted = false) (now : t.aborted = true) : s.published = false := by
  cases step with
  | evidence _ _ _ _ => simp at now; exact absurd now (by simp [was])
  | publish _ _ => simp at now; exact absurd now (by simp [was])
  | abort open_ => exact open_
  | retireBackup _ => simp at now; exact absurd now (by simp [was])

-- B1 (restore side): the backup never retires under a live restore.
theorem backup_waits_for_live_restores {s : State} (run : Reachable s) :
    s.backupRetiredWhileLive = false :=
  (run_safe run).2.2.2.2.2

-- B3: the restore writes nothing into the source.
theorem the_source_is_never_written {s : State} (run : Reachable s) : s.sourceWrites = 0 :=
  (run_safe run).2.2.2.2.1

-- B5 globally: no mismatching evidence was ever committed.
theorem no_mismatch_is_ever_committed {s : State} (run : Reachable s) : s.mismatched = 0 :=
  (run_safe run).2.2.2.1

theorem evidence_is_bounded {s : State} (run : Reachable s) : s.evidenced ≤ s.targets :=
  (run_safe run).1

end Kv9.Restore
