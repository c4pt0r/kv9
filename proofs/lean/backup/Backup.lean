import Std

namespace Kv9.Backup

/-- One backup of a keyspace as the cluster drives it (docs/BACKUP-RESTORE.md §3–§7).
`total` ranges; `anchored` of them have a committed anchor; `pins` counts the published
owner pairs; `complete` says the completion row exists, `aborted` an effective abort,
`retired` the retirement row; `released` that the pins were released; `splitsWhileHeld`
counts splits (or merges, or plans) admitted on a range the backup still held. -/
structure State where
  total : Nat := 2
  anchored : Nat := 0
  pins : Nat := 0
  complete : Bool := false
  aborted : Bool := false
  retired : Bool := false
  released : Bool := false
  splitsWhileHeld : Nat := 0
  deriving DecidableEq, Repr

/-- The backup holds its ranges while it is live and not every range is anchored. -/
def held (s : State) : Bool := !s.aborted && !s.complete && decide (s.anchored < s.total)

inductive Step : State → State → Prop where
  /-- The metadata leader publishes one range's owner pair: only for a live backup. -/
  | bind {s} (live : s.aborted = false) (open_ : s.complete = false) :
      Step s {s with pins := s.pins + 1}
  /-- A range leader commits its anchor: only while the backup is live and the range
  is not anchored yet. -/
  | anchor {s} (live : s.aborted = false) (open_ : s.complete = false)
      (left : s.anchored < s.total) :
      Step s {s with anchored := s.anchored + 1}
  /-- The metadata leader writes the bundle and commits the completion: only once every
  range is anchored and the backup is live. -/
  | complete {s} (live : s.aborted = false) (all : s.anchored = s.total) :
      Step s {s with complete := true}
  /-- An operator aborts: only before completion (a complete backup retires). -/
  | abort {s} (open_ : s.complete = false) :
      Step s {s with aborted := true}
  /-- An operator retires a COMPLETE backup; this is the only release of the pins. -/
  | retire {s} (done : s.complete = true) :
      Step s {s with retired := true, released := true}
  /-- A split, merge or replica plan on one of the backup's ranges: admitted only when
  the backup no longer holds it. -/
  | split {s} (free : held s = false) :
      Step s {s with splitsWhileHeld := if held s then s.splitsWhileHeld + 1 else s.splitsWhileHeld}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  s.anchored ≤ s.total
    ∧ (s.complete = true → s.anchored = s.total)
    ∧ (s.retired = true → s.complete = true)
    ∧ (s.released = true → s.retired = true)
    ∧ ¬ (s.aborted = true ∧ s.complete = true)
    ∧ s.splitsWhileHeld = 0

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hA, hC, hR, hL, hX, hS⟩ := safe
  cases step with
  | bind live open_ => exact ⟨hA, hC, hR, hL, hX, hS⟩
  | anchor live open_ left => exact ⟨by simp; omega, fun h => by simp [open_] at h, hR, hL, hX, hS⟩
  | complete live all => exact ⟨hA, fun _ => all, fun _ => rfl, hL, by simp [live], hS⟩
  | abort open_ => exact ⟨hA, by simp [open_], hR, hL, by simp [open_], hS⟩
  | retire done => exact ⟨hA, hC, fun _ => done, fun _ => rfl, hX, hS⟩
  | split free => exact ⟨hA, hC, hR, hL, hX, by simp [free, hS]⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- B5/B4: completion needs every range anchored.
theorem completion_needs_every_anchor {s t : State} (step : Step s t)
    (was : s.complete = false) (now : t.complete = true) : s.anchored = s.total := by
  cases step with
  | bind _ _ => simp at now; exact absurd now (by simp [was])
  | anchor _ _ _ => simp at now; exact absurd now (by simp [was])
  | complete _ all => exact all
  | abort _ => simp at now; exact absurd now (by simp [was])
  | retire _ => simp at now; exact absurd now (by simp [was])
  | split _ => simp at now; exact absurd now (by simp [was])

-- B6: an anchor is committed only for a live, open backup.
theorem an_anchor_needs_a_live_backup {s t : State} (step : Step s t)
    (grew : s.anchored < t.anchored) : s.aborted = false ∧ s.complete = false := by
  cases step with
  | bind _ _ => simp at grew
  | anchor live open_ _ => exact ⟨live, open_⟩
  | complete _ _ => simp at grew
  | abort _ => simp at grew
  | retire _ => simp at grew
  | split _ => simp at grew

-- A complete backup retires; it does not abort.
theorem abort_refuses_after_completion {s t : State} (step : Step s t)
    (was : s.aborted = false) (now : t.aborted = true) : s.complete = false := by
  cases step with
  | bind _ _ => simp at now; exact absurd now (by simp [was])
  | anchor _ _ _ => simp at now; exact absurd now (by simp [was])
  | complete _ _ => simp at now; exact absurd now (by simp [was])
  | abort open_ => exact open_
  | retire _ => simp at now; exact absurd now (by simp [was])
  | split _ => simp at now; exact absurd now (by simp [was])

-- B1: the pins are released only by a retirement, and only a complete backup retires.
theorem release_only_by_retirement {s t : State} (step : Step s t)
    (was : s.released = false) (now : t.released = true) : t.retired = true ∧ s.complete = true := by
  cases step with
  | bind _ _ => simp at now; exact absurd now (by simp [was])
  | anchor _ _ _ => simp at now; exact absurd now (by simp [was])
  | complete _ _ => simp at now; exact absurd now (by simp [was])
  | abort _ => simp at now; exact absurd now (by simp [was])
  | retire done => exact ⟨rfl, done⟩
  | split _ => simp at now; exact absurd now (by simp [was])

-- B7 / §11: a range the backup holds takes no split, merge or plan.
theorem a_held_range_takes_no_split {s : State} (run : Reachable s) : s.splitsWhileHeld = 0 :=
  (run_safe run).2.2.2.2.2

-- Anchors never exceed the ranges.
theorem anchors_are_bounded {s : State} (run : Reachable s) : s.anchored ≤ s.total :=
  (run_safe run).1

-- A retired backup was complete.
theorem retirement_needs_completion {s : State} (run : Reachable s) (retired : s.retired = true) :
    s.complete = true :=
  (run_safe run).2.2.1 retired

end Kv9.Backup
