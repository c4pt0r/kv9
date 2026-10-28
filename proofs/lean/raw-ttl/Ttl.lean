import Std

namespace Kv9.RawTtl

/-- A version abstracts (creation digest, term, index), whose uniqueness is a
separate replication/identity obligation. Each statement concerns one key. -/
structure Record where
  version : Nat
  deadline : Nat
  deriving DecidableEq

structure State where
  clock : Nat
  current : Option Record

def advance (s : State) (observed : Nat) : State :=
  { s with clock := max s.clock observed }

def expired (record : Record) (clock : Nat) : Prop := record.deadline ≤ clock

def cleanup (s : State) (observed : Record) : State :=
  if s.current = some observed ∧ observed.deadline ≤ s.clock then
    { s with current := none }
  else s

theorem clock_never_regresses (s : State) (observed : Nat) :
    s.clock ≤ (advance s observed).clock := by
  exact Nat.le_max_left _ _

theorem old_sample_keeps_floor (s : State) (observed : Nat) (old : observed ≤ s.clock) :
    (advance s observed).clock = s.clock := by
  exact Nat.max_eq_left old

theorem expiry_never_reverses (record : Record) (s : State) (observed : Nat)
    (hidden : expired record s.clock) : expired record (advance s observed).clock := by
  exact Nat.le_trans hidden (clock_never_regresses s observed)

/-- A positive TTL deadline uses the clock interval's upper bound. If the
committed expiry floor is a true lower bound at read time, expiry cannot occur
before the promised duration. Overflow is refused separately by the Rust codec. -/
theorem no_early_expiry (written upper duration floor readTime : Nat)
    (clockBound : written ≤ upper) (floorBound : floor ≤ readTime)
    (expiredNow : upper + duration ≤ floor) : written + duration ≤ readTime := by
  omega

theorem delayed_apply_keeps_deadline (upper duration later : Nat)
    (late : upper + duration ≤ later) :
    expired ⟨1, upper + duration⟩ later := late

theorem cleanup_preserves_new_version (s : State) (observed current : Record)
    (present : s.current = some current) (different : current.version ≠ observed.version) :
    (cleanup s observed).current = s.current := by
  have unequal : current ≠ observed := by intro h; exact different (congrArg Record.version h)
  simp [cleanup, present, unequal]

theorem cleanup_preserves_unexpired (s : State) (observed : Record)
    (live : s.clock < observed.deadline) : (cleanup s observed).current = s.current := by
  have h : ¬observed.deadline ≤ s.clock := by omega
  simp [cleanup, h]

theorem cleanup_requires_exact_current (s : State) (observed : Record)
    (changed : (cleanup s observed).current ≠ s.current) :
    s.current = some observed ∧ expired observed s.clock := by
  unfold cleanup at changed
  split at changed
  next h => exact h
  next h => exact False.elim (changed rfl)

theorem cleanup_does_not_move_clock (s : State) (observed : Record) :
    (cleanup s observed).clock = s.clock := by
  unfold cleanup
  split <;> rfl

theorem inherited_floor_keeps_source_expiry (record : Record) (left right : Nat)
    (hidden : expired record left) : expired record (max left right) := by
  exact Nat.le_trans hidden (Nat.le_max_left _ _)

end Kv9.RawTtl
