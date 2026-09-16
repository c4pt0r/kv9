import HeapSplitLeaf

set_option autoImplicit false

namespace Kv9.Radix

def drainBranchPlace (state : MutHeap) (others : List NodeId) (sharedLength : Nat) : Option (MutHeap × Key × UInt8) := do
  let (next, focus) ← makePlaceUnique state others
  match heapRead next.heap focus with
  | some (.branch pfx terminal edges) =>
      let byte ← pfx[sharedLength]?
      let node := StoredNode.branch (pfx.drop (sharedLength + 1)) terminal edges
      some ({ next with heap := heapWrite next.heap focus (some node) }, pfx.take sharedLength, byte)
  | _ => none

theorem drain_branch_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (sharedLength : Nat) (byte : UInt8)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (access : pfx[sharedLength]? = some byte) :
    ∃ next address nextFrames, drainBranchPlace state others sharedLength = some (next, pfx.take sharedLength, byte) ∧
      MutationInvariant next others address (.branch (pfx.drop (sharedLength + 1)) terminal children) nextFrames ∧
      nextFrames.map HeapFrame.value = frames.map HeapFrame.value ∧
      (∀ saved value, saved ∈ others → NodeRep state.heap saved value → NodeRep next.heap saved value) := by
  obtain ⟨unique, address, uniqueFrames, completed, uniqueInv, one, _, values, preserve⟩ :=
    make_place_unique_invariant state others focus (.branch pfx terminal children) frames inv
  have stored : ∃ edges, heapRead unique.heap address = some (.branch pfx terminal edges) := by
    cases uniqueInv.focused with
    | branch _ _ _ edges _ read _ => exact ⟨edges, read⟩
  obtain ⟨edges, read⟩ := stored
  have focused := write_branch_payload_refines unique.heap address pfx (pfx.drop (sharedLength + 1)) terminal terminal edges children uniqueInv.focused read
  obtain ⟨nextInv, _, keep⟩ := mutation_same_children_write unique others address (.branch pfx terminal children)
    (.branch (pfx.drop (sharedLength + 1)) terminal children) uniqueFrames (.branch pfx terminal edges)
    (.branch (pfx.drop (sharedLength + 1)) terminal edges) uniqueInv one read rfl focused
  exact ⟨{ unique with heap := heapWrite unique.heap address (some (.branch (pfx.drop (sharedLength + 1)) terminal edges)) }, address, uniqueFrames,
    by simp [drainBranchPlace, completed, read, access], nextInv, values,
    fun saved value member original => keep saved value member (preserve saved value member original)⟩

def splitBranchPlace (state : MutHeap) (others : List NodeId) (depth sharedLength : Nat) (fresh : Entry) : Option MutHeap := do
  let (shortened, pfx, oldByte) ← drainBranchPlace state others sharedLength
  match fresh.key[depth + sharedLength]? with
  | none => wrapCurrentPlace shortened others pfx (some fresh) oldByte
  | some newByte => wrapCurrentPairPlace shortened others pfx oldByte newByte fresh

theorem split_branch_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (depth sharedLength : Nat) (fresh : Entry) (replacement : Tree)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (result : splitBranchAt depth sharedLength pfx terminal children fresh = some replacement) :
    ∃ next, splitBranchPlace state others depth sharedLength fresh = some next ∧
      MutationResult state others frames replacement next := by
  cases oldByte : pfx[sharedLength]? with
  | none => simp [splitBranchAt, oldByte] at result
  | some oldByteValue =>
      obtain ⟨shortened, address, shortenedFrames, drained, shortenedInv, values, preserve⟩ :=
        drain_branch_place_refines state others focus pfx terminal children frames sharedLength oldByteValue inv oldByte
      cases newByte : fresh.key[depth + sharedLength]? with
      | none =>
          simp only [splitBranchAt, oldByte, newByte, Option.some.injEq] at result
          subst replacement
          obtain ⟨next, completed, wrapped⟩ := wrap_current_place_result shortened others address
            (.branch (pfx.drop (sharedLength + 1)) terminal children) shortenedFrames (pfx.take sharedLength) (some fresh) oldByteValue shortenedInv
          refine ⟨next, ?_, ⟨wrapped.owned, ?_, ?_⟩⟩
          · simp [splitBranchPlace, drained, newByte, completed]
          · simpa only [values] using wrapped.rootValue
          · intro saved value member original
            exact wrapped.saved saved value member (preserve saved value member original)
      | some newByteValue =>
          simp only [splitBranchAt, oldByte, newByte, Option.some.injEq] at result
          subst replacement
          obtain ⟨next, completed, wrapped⟩ := wrap_current_pair_result shortened others address
            (.branch (pfx.drop (sharedLength + 1)) terminal children) shortenedFrames (pfx.take sharedLength) oldByteValue newByteValue fresh shortenedInv
          refine ⟨next, ?_, ⟨wrapped.owned, ?_, ?_⟩⟩
          · simp [splitBranchPlace, drained, newByte, completed]
          · simpa only [values] using wrapped.rootValue
          · intro saved value member original
            exact wrapped.saved saved value member (preserve saved value member original)

theorem split_branch_place_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (depth sharedLength : Nat) (fresh : Entry) (replacement : Tree)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (result : splitBranchAt depth sharedLength pfx terminal children fresh = some replacement) :
    executeInsert depth fresh (.branch pfx terminal children) (.splitBranch sharedLength) = .done replacement true ∧
      ∃ next, splitBranchPlace state others depth sharedLength fresh = some next ∧ MutationResult state others frames replacement next :=
  ⟨by simp [executeInsert, result], split_branch_place_refines state others focus pfx terminal children frames depth sharedLength fresh replacement inv result⟩

end Kv9.Radix
