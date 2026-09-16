import Std

namespace Kv9.DataRange

/-- Identity abstracts exact root, creation digest and Raft group. Namespace
and logical bounds are immutable after initial installation. -/
structure Range where
  identity : Nat
  space : Nat
  lower : Nat
  upper : Option Nat
  conf : Nat := 1
  version : Nat := 1
  sealed : Bool := false
  deriving DecidableEq, Repr

structure State where
  range : Option Range := none
  batches : List (List Nat) := []
  deriving DecidableEq, Repr

def contains (r : Range) (key : Nat) : Bool :=
  decide (r.lower ≤ key) && r.upper.all (fun hi => decide (key < hi))

def install (identity : Nat) (s : State) (r : Range) : State :=
  if r.identity = identity ∧ r.conf = 1 ∧ r.version = 1 ∧ r.sealed = false ∧ s.range = none
  then {s with range := some r} else s

def sealedRange (r : Range) : Range := {r with version := r.version + 1, sealed := true}

/-- Equality abstracts exact CAS-digest equality, under collision resistance. -/
def closeRange (s : State) (expected : Range) : State :=
  if s.range = some expected ∧ expected.sealed = false
  then {s with range := some (sealedRange expected)} else s

def allowed (s : State) (identity space conf version : Nat) (keys : List Nat) : Bool :=
  match s.range with
  | none => false
  | some r => !r.sealed && decide (r.identity = identity) && decide (r.space = space) &&
      decide (r.conf = conf) && decide (r.version = version) && keys.all (contains r)

def write (s : State) (identity space conf version : Nat) (keys : List Nat) : State :=
  if allowed s identity space conf version keys
  then {s with batches := s.batches ++ [keys]} else s

theorem no_write_before_install (s : State) (empty : s.range = none) (identity space conf version : Nat) (keys : List Nat) :
    write s identity space conf version keys = s := by simp [write, allowed, empty]

theorem accepted_write_has_exact_scope (s : State) (identity space conf version : Nat) (keys : List Nat)
    (accepted : allowed s identity space conf version keys = true) :
    ∃ r, s.range = some r ∧ r.sealed = false ∧ r.identity = identity ∧
      r.space = space ∧ r.conf = conf ∧ r.version = version ∧ keys.all (contains r) = true := by
  cases h : s.range with
  | none => simp [allowed, h] at accepted
  | some r => exact ⟨r, rfl, by simpa [allowed, h, and_assoc] using accepted⟩

theorem future_epoch_refused (s : State) (r : Range) (present : s.range = some r)
    (identity space conf version : Nat) (keys : List Nat) (future : r.version < version) :
    write s identity space conf version keys = s := by
  have ne : r.version ≠ version := Nat.ne_of_lt future
  simp [write, allowed, present, ne]

theorem stale_conf_refused (s : State) (r : Range) (present : s.range = some r)
    (identity space conf version : Nat) (keys : List Nat) (stale : conf < r.conf) :
    write s identity space conf version keys = s := by
  have ne : r.conf ≠ conf := Nat.ne_of_gt stale
  simp [write, allowed, present, ne]

theorem sealed_refuses_every_write (s : State) (r : Range) (present : s.range = some r)
    (closed : r.sealed = true) (identity space conf version : Nat) (keys : List Nat) :
    write s identity space conf version keys = s := by simp [write, allowed, present, closed]

theorem batch_is_all_or_none (s : State) (identity space conf version : Nat) (keys : List Nat) :
    (write s identity space conf version keys).batches = s.batches ∨
    (write s identity space conf version keys).batches = s.batches ++ [keys] := by
  simp only [write]; split <;> simp

theorem rejected_batch_has_no_prefix (s : State) (identity space conf version : Nat) (keys : List Nat)
    (rejected : allowed s identity space conf version keys = false) :
    write s identity space conf version keys = s := by simp [write, rejected]

theorem existing_range_cannot_be_reinitialized (identity : Nat) (s : State) (old next : Range)
    (present : s.range = some old) : install identity s next = s := by simp [install, present]

theorem install_cannot_change_data (identity : Nat) (s : State) (r : Range) :
    (install identity s r).batches = s.batches := by simp only [install]; split <;> rfl

theorem sealing_preserves_namespace_and_bounds (r : Range) :
    (sealedRange r).identity = r.identity ∧ (sealedRange r).space = r.space ∧
    (sealedRange r).lower = r.lower ∧ (sealedRange r).upper = r.upper := by simp [sealedRange]

inductive Step (identity : Nat) : State → State → Prop where
  | install (s) (r) : Step identity s (install identity s r)
  | closeRange (s) (r) : Step identity s (closeRange s r)
  | write (s) (space conf version) (keys) : Step identity s (write s identity space conf version keys)
  | restart (s) : Step identity s s

def Sealed (s : State) : Prop := ∃ r, s.range = some r ∧ r.sealed = true

theorem exact_open_seal_establishes_terminal_state (s : State) (r : Range)
    (present : s.range = some r) (openRange : r.sealed = false) :
    Sealed (closeRange s r) := by
  simp [closeRange, present, openRange, Sealed, sealedRange]

theorem seal_is_terminal (identity : Nat) {s t : State} (step : Step identity s t) (closed : Sealed s) : Sealed t := by
  obtain ⟨r, present, sealed⟩ := closed
  cases step <;> simp_all [install, closeRange, write, allowed, Sealed]
  split <;> simp_all

inductive Run (identity : Nat) : State → State → Prop where
  | nil (s) : Run identity s s
  | next {s t u} : Run identity s t → Step identity t u → Run identity s u

theorem seal_survives_arbitrary_traces (identity : Nat) {s t : State}
    (run : Run identity s t) (closed : Sealed s) : Sealed t := by
  induction run with
  | nil => exact closed
  | next _ step ih => exact seal_is_terminal identity step ih

theorem lower_bound_is_inclusive (r : Range)
    (upper : r.upper.all (fun hi => decide (r.lower < hi)) = true) : contains r r.lower = true := by
  simp [contains, upper]

theorem upper_bound_is_exclusive (r : Range) (hi : Nat) (upper : r.upper = some hi) : contains r hi = false := by
  simp [contains, upper]

theorem single_read_view_cannot_authorize_different_scope (s : State) (identity space conf version : Nat) (keys : List Nat)
    (accepted : allowed s identity space conf version keys = true) :
    ∃ r, s.range = some r ∧ r.space = space ∧ r.conf = conf ∧ r.version = version := by
  obtain ⟨r, present, _, _, ns, c, v, _⟩ := accepted_write_has_exact_scope s identity space conf version keys accepted
  exact ⟨r, present, ns, c, v⟩

end Kv9.DataRange
