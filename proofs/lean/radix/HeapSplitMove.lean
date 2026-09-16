import HeapSplitBuild

set_option autoImplicit false

namespace Kv9.Radix

def movedEmptyEntry : Entry := ⟨[], []⟩

-- The two mem::take operations move the owned key/value into the returned
-- Entry. The old leaf really contains empty buffers before slot assignment.
-- Native buffer ownership is a separate correspondence, not an Arc token.
def takeLeafEntryPlace (state : MutHeap) (others : List NodeId) : Option (MutHeap × Entry) := do
  let (next, focus) ← makePlaceUnique state others
  match heapRead next.heap focus with
  | some (.leaf entry) => some ({ next with heap := heapWrite next.heap focus (some (.leaf movedEmptyEntry)) }, entry)
  | _ => none

theorem take_leaf_entry_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (entry : Entry) (frames : List HeapFrame) (inv : MutationInvariant state others focus (.leaf entry) frames) :
    ∃ next address nextFrames, takeLeafEntryPlace state others = some (next, entry) ∧
      MutationInvariant next others address (.leaf movedEmptyEntry) nextFrames ∧
      nextFrames.map HeapFrame.value = frames.map HeapFrame.value ∧
      (∀ saved value, saved ∈ others → NodeRep state.heap saved value → NodeRep next.heap saved value) := by
  obtain ⟨unique, address, uniqueFrames, completed, uniqueInv, one, _, values, preserve⟩ :=
    make_place_unique_invariant state others focus (.leaf entry) frames inv
  have read : heapRead unique.heap address = some (.leaf entry) := by cases uniqueInv.focused; assumption
  have focused := write_leaf_payload_refines unique.heap address entry movedEmptyEntry read
  obtain ⟨nextInv, _, keep⟩ := mutation_same_children_write unique others address (.leaf entry) (.leaf movedEmptyEntry)
    uniqueFrames (.leaf entry) (.leaf movedEmptyEntry) uniqueInv one read rfl focused
  exact ⟨{ unique with heap := heapWrite unique.heap address (some (.leaf movedEmptyEntry)) }, address, uniqueFrames,
    by simp [takeLeafEntryPlace, completed, read], nextInv, values,
    fun saved value member original => keep saved value member (preserve saved value member original)⟩

def wrapFreshLeafPlace (state : MutHeap) (others : List NodeId) (pfx : Key) (terminal : Option Entry)
    (byte : UInt8) (fresh : Entry) : Option MutHeap :=
  let allocated := heapAlloc state.heap (.leaf fresh)
  let node := StoredNode.branch pfx terminal [(byte, state.heap.length)]
  assignPlace { state with heap := heapAlloc allocated node } others allocated.length

theorem wrap_fresh_leaf_result (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (pfx : Key) (terminal : Option Entry) (byte : UInt8) (fresh : Entry)
    (inv : MutationInvariant state others focus tree frames) :
    ∃ next, wrapFreshLeafPlace state others pfx terminal byte fresh = some next ∧
      MutationResult state others frames (.branch pfx terminal (.cons byte (.leaf fresh) .nil)) next := by
  let allocated := heapAlloc state.heap (.leaf fresh)
  let node := StoredNode.branch pfx terminal [(byte, state.heap.length)]
  have allocatedOwned := heap_alloc_leaf_owned state.heap (state.root :: others) fresh inv.owned
  have leafRead := heap_alloc_fresh state.heap (.leaf fresh)
  have leafRep : NodeRep allocated state.heap.length (.leaf fresh) := .leaf state.heap.length fresh leafRead
  have payload : PayloadRep allocated node (.branch pfx terminal (.cons byte (.leaf fresh) .nil)) :=
    .branch pfx terminal [(byte, state.heap.length)] (.cons byte (.leaf fresh) .nil)
      (.cons byte state.heap.length [] (.leaf fresh) .nil leafRep .nil)
  have privateParents : ContextPrivate allocated (state.root :: others) frames := by
    intro frame member
    simpa only [allocated, strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using inv.privateParents frame member
  have savedParents : ContextSaved allocated others frames := by
    intro frame member saved included path
    obtain ⟨old, rep⟩ := inv.owned.rootsRepresented saved (by simp [included])
    exact inv.savedParents frame member saved included (heap_reach_alloc_reflects state.heap saved old (.leaf fresh) rep frame.parent path)
  have avoid : ∀ frame ∈ frames, ∀ edge ∈ storedChildren node, ¬ HeapReach allocated edge.2 frame.parent := by
    intro frame member edge included
    have same : edge = (byte, state.heap.length) := by simpa only [node, storedChildren, List.mem_cons, List.not_mem_nil, or_false] using included
    subst edge
    intro path
    exact (Nat.ne_of_lt (heap_read_bound state.heap frame.parent _
      (heap_context_frame_read state.heap focus state.root frames inv.context frame member)))
      (leaf_reach_self allocated state.heap.length fresh leafRead frame.parent path)
  obtain ⟨next, completed, result⟩ := allocate_assign_payload_result { state with heap := allocated } others focus tree _ frames node
    allocatedOwned (heap_context_alloc state.heap focus state.root frames (.leaf fresh) inv.context)
    (node_rep_alloc state.heap focus tree (.leaf fresh) inv.focused) privateParents savedParents inv.aligned payload avoid
  exact ⟨next, completed, ⟨result.owned, result.rootValue,
    fun saved value member original => result.saved saved value member (node_rep_alloc state.heap saved value (.leaf fresh) original)⟩⟩

end Kv9.Radix
