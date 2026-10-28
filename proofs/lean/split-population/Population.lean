import Std

namespace Kv9.SplitPopulation

/-- One key under the new population tag only. Legacy Write replay, remote
source certification, complete image transfer and TTL split are outside scope. -/
structure State where
  owned : Bool := false
  sealed : Bool := false
  ttl : Bool := false
  value : Option Nat := none

inductive Command where
  | copy (value : Option Nat)
  | client (value : Option Nat)
  | bind
  | seal
  | enable
  | idle

def step (s : State) (cmd : Command) : State :=
  match cmd with
  | .copy value => if s.owned = false ∧ s.ttl = false then {s with value} else s
  | .client value => if s.owned = true ∧ s.sealed = false then {s with value} else s
  | .bind => {s with owned := true}
  | .seal => if s.owned = true then {s with sealed := true} else s
  | .enable => if s.owned = true ∧ s.sealed = false then {s with ttl := true} else s
  | .idle => s

theorem ownership_is_permanent (h : s.owned = true) : (step s cmd).owned = true := by
  cases cmd <;> simp [step, h] <;> split <;> simp_all

theorem population_after_binding_is_inert (h : s.owned = true) : step s (.copy value) = s := by
  simp [step, h]

theorem ttl_cannot_use_default_cf_population (h : s.ttl = true) : step s (.copy value) = s := by
  simp [step, h]

theorem changed_copy_requires_unbound_non_ttl (h : (step s (.copy v)).value ≠ s.value) :
    s.owned = false ∧ s.ttl = false := by
  by_cases allowed : s.owned = false ∧ s.ttl = false
  · exact allowed
  · simp [step, allowed] at h

def copies (s : State) : List (Option Nat) → State
  | [] => s
  | value :: rest => copies (step s (.copy value)) rest

theorem any_delayed_copy_history_is_inert (h : s.owned = true) (values : List (Option Nat)) :
    copies s values = s := by
  induction values with
  | nil => rfl
  | cons value rest ih => simpa [copies, population_after_binding_is_inert h] using ih

theorem deleted_child_key_cannot_be_resurrected_by_copy (h : s.owned = true)
    (deleted : s.value = none) (values : List (Option Nat)) : (copies s values).value = none := by
  rw [any_delayed_copy_history_is_inert h values]
  exact deleted

theorem sealed_parent_value_is_immutable (owned : s.owned = true) (sealed : s.sealed = true) :
    (step s cmd).value = s.value := by
  cases cmd <;> simp [step, owned, sealed]

end Kv9.SplitPopulation
