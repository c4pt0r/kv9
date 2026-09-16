import Std

/-
This is a conditional ownership-transfer model of ErasedPtr::map_owned.
Addresses are abstract allocation identities, not Rust pointer provenance.
`counts` includes every strong reference; `views` counts retained external
references. The temporary owner accounts for one additional reference.

The callback is allowed to replace the allocation and update the heap before
either return or unwind. Its final state is a premise, not an assumption that
panic leaves the initial pointer unchanged. The model assumes lawful from_raw,
as_ptr, ManuallyDrop and Rust unwinding, and excludes abort/double panic.
It does not prove these Rust/library/compiler premises.
-/
namespace Kv9.StaticPointerCallback

abbrev Address := Nat

inductive Exit (R : Type) where
  | normal (value : R)
  | unwind
deriving DecidableEq

structure OwnedState where
  current : Address
  counts : Address → Nat
  views : Address → Nat

structure Outcome (R : Type) where
  state : OwnedState
  exit : Exit R

structure Completed (R : Type) where
  slot : Address
  counts : Address → Nat
  views : Address → Nat
  exit : Exit R

def token (owner address : Address) : Nat := if address = owner then 1 else 0

def OwnedValid (state : OwnedState) : Prop :=
  ∀ address, state.views address + token state.current address ≤ state.counts address

def SlotValid (state : Completed R) : Prop :=
  ∀ address, state.views address + token state.slot address ≤ state.counts address

-- `callback` is the complete abstract effect of f, including its exit kind.
-- Both implementations reconstruct the same owner and invoke it exactly once.
-- `dynamic` corresponds to the erased function pointer stored in WriteBack.
def dynamic (asPtr : Address → Address) (done : Outcome R) : Completed R :=
  ⟨asPtr done.state.current, done.state.counts, done.state.views, done.exit⟩

-- The candidate stores the function-item adapter, retaining ManuallyDrop.
-- Pointer erasure is represented by the common address identity abstraction.
def static (asPtr : Address → Address) (done : Outcome R) : Completed R :=
  let adapter := fun owned => asPtr owned
  ⟨adapter done.state.current, done.state.counts, done.state.views, done.exit⟩

theorem callback_adapter_equal (asPtr : Address → Address) :
    (fun owned => asPtr owned) = asPtr := rfl

theorem guard_equivalence (oldPtr newPtr : Address → Address)
    (same : ∀ owned, newPtr owned = oldPtr owned) (done : Outcome R) :
    static newPtr done = dynamic oldPtr done := by
  simp only [static, dynamic, same]

theorem transaction_equivalence (oldPtr newPtr : Address → Address)
    (same : ∀ owned, newPtr owned = oldPtr owned)
    (callback : OwnedState → Outcome R) (initial : OwnedState) :
    static newPtr (callback initial) = dynamic oldPtr (callback initial) :=
  guard_equivalence oldPtr newPtr same (callback initial)

theorem normal_equivalence (asPtr : Address → Address) (state : OwnedState) (value : R) :
    static asPtr ⟨state, .normal value⟩ = dynamic asPtr ⟨state, .normal value⟩ := rfl

theorem unwind_equivalence (asPtr : Address → Address) (state : OwnedState) :
    static asPtr (⟨state, .unwind⟩ : Outcome R) = dynamic asPtr ⟨state, .unwind⟩ := rfl

theorem current_slot_restored (asPtr : Address → Address) (done : Outcome R)
    (lawful : asPtr done.state.current = done.state.current) :
    (static asPtr done).slot = done.state.current := lawful

theorem heap_unchanged_by_guard (asPtr : Address → Address) (done : Outcome R) :
    (static asPtr done).counts = done.state.counts := rfl

theorem views_unchanged_by_guard (asPtr : Address → Address) (done : Outcome R) :
    (static asPtr done).views = done.state.views := rfl

theorem exit_preserved (asPtr : Address → Address) (done : Outcome R) :
    (static asPtr done).exit = done.exit := rfl

-- The same one-reference token moves from the temporary owner to the slot;
-- no decrement or increment occurs in this guard operation.
theorem one_reference_transferred (asPtr : Address → Address) (done : Outcome R)
    (lawful : asPtr done.state.current = done.state.current) (address : Address) :
    token (static asPtr done).slot address = token done.state.current address := by
  rw [current_slot_restored asPtr done lawful]

theorem validity_restored (asPtr : Address → Address) (done : Outcome R)
    (lawful : asPtr done.state.current = done.state.current) (valid : OwnedValid done.state) :
    SlotValid (static asPtr done) := by
  intro address
  simpa only [static, lawful] using valid address

theorem retained_views_preserved (asPtr : Address → Address) (initial : OwnedState)
    (done : Outcome R) (callback_preserves : done.state.views = initial.views) :
    (static asPtr done).views = initial.views := callback_preserves

theorem unwind_after_replacement (newAddress : Address) (counts views : Address → Nat) :
    (static id (⟨⟨newAddress, counts, views⟩, .unwind⟩ : Outcome R)).slot = newAddress := rfl

theorem clone_failure_preserves_ownership (initial : OwnedState) (valid : OwnedValid initial) :
    SlotValid (static id (⟨initial, .unwind⟩ : Outcome R)) ∧
    (static id (⟨initial, .unwind⟩ : Outcome R)).counts = initial.counts := by
  exact ⟨validity_restored id _ rfl valid, rfl⟩

-- A witness that distinguishes the current owner from the pre-call owner.
-- Old allocation 0 has no remaining references; current allocation 1 has one.
def replacement : Outcome Unit :=
  ⟨⟨1, fun address => if address = 1 then 1 else 0, fun _ => 0⟩, .unwind⟩

theorem replacement_valid : OwnedValid replacement.state := by
  intro address
  simp only [replacement, token, Nat.zero_add]
  exact Nat.le_refl _

theorem stale_writeback_is_invalid :
    ¬ SlotValid { static id replacement with slot := 0 } := by
  intro valid
  have impossible := valid 0
  simp [static, replacement, token] at impossible

theorem dropping_current_reference_is_invalid :
    ¬ SlotValid { static id replacement with
      counts := fun address => replacement.state.counts address - token replacement.state.current address } := by
  intro valid
  have impossible := valid 1
  simp [static, replacement, token] at impossible

theorem missing_unwind_writeback_is_stale :
    (static id replacement).slot ≠ 0 := by decide

end Kv9.StaticPointerCallback
