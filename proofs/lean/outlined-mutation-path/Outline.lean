import WriteBack

/- Control-flow extraction relative to the original smart-pointer contract.
   `cloneReplace` includes the existing clone, allocation and owner replacement,
   with all its heap effects and any unwind. The pointer conversion runs only
   after a normal result. `observe` is evaluated once in either program.
   This is not verified Rust extraction or a proof of the original Arc library. -/
namespace Kv9.OutlinedMutation
open Kv9.StaticPointerCallback

def exposeAfter (expose : OwnedState → Reference) (done : Outcome Unit) : Outcome Reference :=
  ⟨done.state, match done.exit with
    | .normal _ => .normal (expose done.state)
    | .unwind => .unwind⟩

def original (unique : Bool) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) : Outcome Reference :=
  if unique then exposeAfter expose ⟨owner, .normal ()⟩
  else exposeAfter expose (cloneReplace owner)

def sharedHelper (cloneReplace : OwnedState → Outcome Unit) (owner : OwnedState) : Outcome Unit :=
  cloneReplace owner

def outlined (unique : Bool) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) : Outcome Reference :=
  let done := if unique then ⟨owner, .normal ()⟩ else sharedHelper cloneReplace owner
  exposeAfter expose done

def runOriginal (observe : OwnedState → Bool × OwnedState) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) : Outcome Reference :=
  let (unique, state) := observe owner
  original unique cloneReplace expose state

def runOutlined (observe : OwnedState → Bool × OwnedState) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) : Outcome Reference :=
  let (unique, state) := observe owner
  outlined unique cloneReplace expose state

theorem extraction_equivalence (unique : Bool) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) :
    outlined unique cloneReplace expose owner = original unique cloneReplace expose owner := by
  cases unique <;> rfl

theorem observe_once_equivalence (observe : OwnedState → Bool × OwnedState)
    (cloneReplace : OwnedState → Outcome Unit) (expose : OwnedState → Reference) (owner : OwnedState) :
    runOutlined observe cloneReplace expose owner = runOriginal observe cloneReplace expose owner := by
  simp only [runOutlined, runOriginal, extraction_equivalence]

theorem unique_path_does_not_clone (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) :
    outlined true cloneReplace expose owner = ⟨owner, .normal (expose owner)⟩ := rfl

theorem shared_success_exposes_new_owner (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner next : OwnedState)
    (success : cloneReplace owner = ⟨next, .normal ()⟩) :
    outlined false cloneReplace expose owner = ⟨next, .normal (expose next)⟩ := by
  simp only [outlined, Bool.false_eq_true, ↓reduceIte, sharedHelper, success, exposeAfter]

theorem clone_unwind_preserved (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner during : OwnedState)
    (panics : cloneReplace owner = ⟨during, .unwind⟩) :
    outlined false cloneReplace expose owner = ⟨during, .unwind⟩ := by
  simp only [outlined, Bool.false_eq_true, ↓reduceIte, sharedHelper, panics, exposeAfter]

theorem unwind_before_replacement_preserves_slot (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner during : OwnedState)
    (panics : cloneReplace owner = ⟨during, .unwind⟩) (sameOwner : during.current = owner.current) :
    (static id (outlined false cloneReplace expose owner)).slot = owner.current := by
  rw [clone_unwind_preserved cloneReplace expose owner during panics]
  exact sameOwner

theorem heap_effects_preserved (unique : Bool) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) :
    (outlined unique cloneReplace expose owner).state.counts =
      (original unique cloneReplace expose owner).state.counts := by
  rw [extraction_equivalence]

theorem guarded_extraction_equivalence (unique : Bool) (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner : OwnedState) (asPtr : Address → Address) :
    static asPtr (outlined unique cloneReplace expose owner) =
      dynamic asPtr (original unique cloneReplace expose owner) := by
  rw [extraction_equivalence]
  rfl

end Kv9.OutlinedMutation
