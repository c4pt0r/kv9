import Outline

/- The pointer guard is unchanged in both programs. Only the inner clone path
   is extracted. This composition inherits the explicit ownership/provenance,
   ordinary-unwind and compiler premises of WriteBack and Outline; it is not
   verified Rust extraction or an independent proof of the original Arc. -/
namespace Kv9.IsolatedOutline
open Kv9.StaticPointerCallback Kv9.OutlinedMutation

theorem dynamic_guard_extraction (unique : Bool)
    (cloneReplace : OwnedState → Outcome Unit) (expose : OwnedState → Reference)
    (owner : OwnedState) (asPtr : Address → Address) :
    dynamic asPtr (outlined unique cloneReplace expose owner) =
      dynamic asPtr (original unique cloneReplace expose owner) := by
  rw [extraction_equivalence]

theorem dynamic_observed_extraction (observe : OwnedState → Bool × OwnedState)
    (cloneReplace : OwnedState → Outcome Unit) (expose : OwnedState → Reference)
    (owner : OwnedState) (asPtr : Address → Address) :
    dynamic asPtr (runOutlined observe cloneReplace expose owner) =
      dynamic asPtr (runOriginal observe cloneReplace expose owner) := by
  rw [observe_once_equivalence]

theorem dynamic_unwind_restores_current_owner (cloneReplace : OwnedState → Outcome Unit)
    (expose : OwnedState → Reference) (owner during : OwnedState)
    (panics : cloneReplace owner = ⟨during, .unwind⟩) :
    (dynamic id (outlined false cloneReplace expose owner)).slot = during.current := by
  rw [clone_unwind_preserved cloneReplace expose owner during panics]
  rfl

end Kv9.IsolatedOutline
