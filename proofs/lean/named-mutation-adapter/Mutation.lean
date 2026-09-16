import WriteBack

/-
The mutation operation is abstract and may replace its owner before returning
or unwinding. Rust's from_mut pointer conversion is represented by a total,
pure function applied only on normal return. The named function and former
closure have the same body. Compiler inlining is an explicit external premise,
not a change to this model's ownership or resource semantics.
-/
namespace Kv9.NamedMutationAdapter
open Kv9.StaticPointerCallback

def convertOutcome (convert : Reference → Pointer) (done : Outcome Reference) : Outcome Pointer :=
  ⟨done.state, match done.exit with
    | .normal value => .normal (convert value)
    | .unwind => .unwind⟩

def closure (mutate : OwnedState → Outcome Reference) (convert : Reference → Pointer) :
    OwnedState → Outcome Pointer :=
  fun owner => convertOutcome convert (mutate owner)

def named (mutate : OwnedState → Outcome Reference) (convert : Reference → Pointer)
    (owner : OwnedState) : Outcome Pointer :=
  convertOutcome convert (mutate owner)

theorem adapter_equivalence (mutate : OwnedState → Outcome Reference)
    (convert : Reference → Pointer) : named mutate convert = closure mutate convert := rfl

theorem guarded_transaction_equivalence (mutate : OwnedState → Outcome Reference)
    (convert : Reference → Pointer) (asPtr : Address → Address) (owner : OwnedState) :
    static asPtr (named mutate convert owner) = dynamic asPtr (closure mutate convert owner) := rfl

theorem normal_result_preserved (convert : Reference → Pointer) (state : OwnedState) (value : Reference) :
    (convertOutcome convert ⟨state, .normal value⟩).exit = .normal (convert value) := rfl

theorem unwind_result_preserved (convert : Reference → Pointer) (state : OwnedState) :
    (convertOutcome convert ⟨state, .unwind⟩).exit = .unwind := rfl

theorem mutation_state_preserved (mutate : OwnedState → Outcome Reference)
    (convert : Reference → Pointer) (owner : OwnedState) :
    (named mutate convert owner).state = (mutate owner).state := rfl

theorem replacement_during_unwind_published (mutate : OwnedState → Outcome Reference)
    (convert : Reference → Pointer) (initial current : OwnedState)
    (unwinds : mutate initial = ⟨current, .unwind⟩) :
    (static id (named mutate convert initial)).slot = current.current := by
  simp only [named, unwinds, convertOutcome, static, id_eq]

end Kv9.NamedMutationAdapter
