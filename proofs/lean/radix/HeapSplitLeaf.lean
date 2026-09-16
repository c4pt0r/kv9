import HeapSplitMove

set_option autoImplicit false

namespace Kv9.Radix

def splitLeafPlace (state : MutHeap) (others : List NodeId) (depth sharedLength : Nat) (fresh : Entry) : Option MutHeap := do
  let focus ← placeTarget state
  match heapRead state.heap focus with
  | some (.leaf old) =>
      let cut := depth + sharedLength
      if depth ≤ cut ∧ cut ≤ old.key.length then
        let pfx := (old.key.drop depth).take sharedLength
        match old.key[cut]?, fresh.key[cut]? with
        | none, some byte => do
            let (taken, terminal) ← takeLeafEntryPlace state others
            wrapFreshLeafPlace taken others pfx (some terminal) byte fresh
        | some byte, none => wrapCurrentPlace state others pfx (some fresh) byte
        | some oldByte, some newByte => wrapCurrentPairPlace state others pfx oldByte newByte fresh
        | none, none => none
      else none
  | _ => none

theorem split_leaf_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame) (depth sharedLength : Nat) (replacement : Tree)
    (inv : MutationInvariant state others focus (.leaf old) frames)
    (result : splitLeafAt depth sharedLength old fresh = some replacement) :
    ∃ next, splitLeafPlace state others depth sharedLength fresh = some next ∧
      MutationResult state others frames replacement next := by
  have target := mutation_invariant_target state others focus (.leaf old) frames inv
  have read : heapRead state.heap focus = some (.leaf old) := by cases inv.focused; assumption
  by_cases bounds : depth ≤ depth + sharedLength ∧ depth + sharedLength ≤ old.key.length
  · cases oldByte : old.key[depth + sharedLength]? with
    | none =>
        cases newByte : fresh.key[depth + sharedLength]? with
        | none => simp [splitLeafAt, bounds, oldByte, newByte, leafSplitBytes] at result
        | some byte =>
            simp only [splitLeafAt, if_pos bounds, oldByte, newByte, leafSplitBytes, Option.some.injEq] at result
            subst replacement
            obtain ⟨taken, address, takenFrames, moved, takenInv, values, preserve⟩ := take_leaf_entry_refines state others focus old frames inv
            obtain ⟨next, completed, wrapped⟩ := wrap_fresh_leaf_result taken others address (.leaf movedEmptyEntry) takenFrames
              ((old.key.drop depth).take sharedLength) (some old) byte fresh takenInv
            refine ⟨next, ?_, ⟨wrapped.owned, ?_, ?_⟩⟩
            · simp [splitLeafPlace, target, read, bounds, oldByte, newByte, moved, completed]
            · simpa only [values] using wrapped.rootValue
            · intro saved value member original
              exact wrapped.saved saved value member (preserve saved value member original)
    | some oldByteValue =>
        cases newByte : fresh.key[depth + sharedLength]? with
        | none =>
            simp only [splitLeafAt, if_pos bounds, oldByte, newByte, leafSplitBytes, Option.some.injEq] at result
            subst replacement
            obtain ⟨next, completed, wrapped⟩ := wrap_current_place_result state others focus (.leaf old) frames
              ((old.key.drop depth).take sharedLength) (some fresh) oldByteValue inv
            exact ⟨next, by simp [splitLeafPlace, target, read, bounds, oldByte, newByte, completed], wrapped⟩
        | some newByteValue =>
            simp only [splitLeafAt, if_pos bounds, oldByte, newByte, leafSplitBytes, Option.some.injEq] at result
            subst replacement
            obtain ⟨next, completed, wrapped⟩ := wrap_current_pair_result state others focus (.leaf old) frames
              ((old.key.drop depth).take sharedLength) oldByteValue newByteValue fresh inv
            exact ⟨next, by simp [splitLeafPlace, target, read, bounds, oldByte, newByte, completed], wrapped⟩
  · rw [splitLeafAt, if_neg bounds] at result
    contradiction

theorem split_leaf_place_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame) (depth sharedLength : Nat) (replacement : Tree)
    (inv : MutationInvariant state others focus (.leaf old) frames)
    (result : splitLeafAt depth sharedLength old fresh = some replacement) :
    executeInsert depth fresh (.leaf old) (.splitLeaf sharedLength) = .done replacement true ∧
      ∃ next, splitLeafPlace state others depth sharedLength fresh = some next ∧ MutationResult state others frames replacement next :=
  ⟨by simp [executeInsert, result], split_leaf_place_refines state others focus old fresh frames depth sharedLength replacement inv result⟩

end Kv9.Radix
