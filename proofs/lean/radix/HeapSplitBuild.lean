import HeapAssignSubtree

set_option autoImplicit false

namespace Kv9.Radix

def storedTwo (a : UInt8) (left : NodeId) (b : UInt8) (right : NodeId) : List StoredEdge :=
  if a < b then [(a, left), (b, right)] else [(b, right), (a, left)]

theorem stored_two_rep (heap : NodeHeap) (a b : UInt8) (left right : NodeId) (leftTree rightTree : Tree)
    (leftRep : NodeRep heap left leftTree) (rightRep : NodeRep heap right rightTree) :
    EdgesRep heap (storedTwo a left b right) (twoEdges a leftTree b rightTree) := by
  by_cases less : a < b
  · simp only [storedTwo, twoEdges, if_pos less]
    exact .cons a left [(b, right)] leftTree (.cons b rightTree .nil) leftRep
      (.cons b right [] rightTree .nil rightRep .nil)
  · simp only [storedTwo, twoEdges, if_neg less]
    exact .cons b right [(a, left)] rightTree (.cons a leftTree .nil) rightRep
      (.cons a left [] leftTree .nil leftRep .nil)

theorem stored_two_member (a b : UInt8) (left right : NodeId) (edge : StoredEdge)
    (member : edge ∈ storedTwo a left b right) : edge = (a, left) ∨ edge = (b, right) := by
  by_cases less : a < b <;> simp only [storedTwo, less, if_true, if_false, List.mem_cons, List.not_mem_nil, or_false] at member
  · exact member
  · exact member.symm

-- Arc::clone(slot) adds an external child token without changing the heap.
-- Branch allocation moves that token into its stored edge before assignment
-- releases the previous slot token. The focused payload is never cloned here.
def wrapCurrentPlace (state : MutHeap) (others : List NodeId) (pfx : Key) (terminal : Option Entry)
    (byte : UInt8) : Option MutHeap := do
  let focus ← placeTarget state
  let node := StoredNode.branch pfx terminal [(byte, focus)]
  assignPlace { state with heap := heapAlloc state.heap node } others state.heap.length

theorem wrap_current_place_result (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (pfx : Key) (terminal : Option Entry) (byte : UInt8)
    (inv : MutationInvariant state others focus tree frames) :
    ∃ next, wrapCurrentPlace state others pfx terminal byte = some next ∧
      MutationResult state others frames (.branch pfx terminal (.cons byte tree .nil)) next := by
  have owned := heap_owned_add_reference state.heap (state.root :: others) focus tree inv.owned inv.focused
  have payload : PayloadRep state.heap (.branch pfx terminal [(byte, focus)]) (.branch pfx terminal (.cons byte tree .nil)) :=
    .branch pfx terminal [(byte, focus)] (.cons byte tree .nil) (.cons byte focus [] tree .nil inv.focused .nil)
  have avoid : ∀ frame ∈ frames, ∀ edge ∈ storedChildren (.branch pfx terminal [(byte, focus)]),
      ¬ HeapReach state.heap edge.2 frame.parent := by
    intro frame member edge included
    have same : edge = (byte, focus) := by simpa only [storedChildren, List.mem_cons, List.not_mem_nil, or_false] using included
    subst edge
    exact heap_context_ancestor_no_return state.heap focus state.root frames inv.context tree inv.focused frame member
  obtain ⟨next, completed, result⟩ := allocate_assign_payload_result state others focus tree _ frames
    (.branch pfx terminal [(byte, focus)]) owned inv.context inv.focused inv.privateParents inv.savedParents inv.aligned payload avoid
  exact ⟨next, by simpa [wrapCurrentPlace, mutation_invariant_target state others focus tree frames inv] using completed, result⟩

-- Source order: retain old child, allocate the new leaf, sort the two distinct
-- byte edges, allocate their parent, then replace the old mutable slot.
def wrapCurrentPairPlace (state : MutHeap) (others : List NodeId) (pfx : Key)
    (oldByte newByte : UInt8) (fresh : Entry) : Option MutHeap := do
  let focus ← placeTarget state
  let allocated := heapAlloc state.heap (.leaf fresh)
  let node := StoredNode.branch pfx none (storedTwo oldByte focus newByte state.heap.length)
  assignPlace { state with heap := heapAlloc allocated node } others allocated.length

theorem wrap_current_pair_result (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (pfx : Key) (oldByte newByte : UInt8) (fresh : Entry)
    (inv : MutationInvariant state others focus tree frames) :
    ∃ next, wrapCurrentPairPlace state others pfx oldByte newByte fresh = some next ∧
      MutationResult state others frames (.branch pfx none (twoEdges oldByte tree newByte (.leaf fresh))) next := by
  let allocated := heapAlloc state.heap (.leaf fresh)
  let node := StoredNode.branch pfx none (storedTwo oldByte focus newByte state.heap.length)
  have cloned := heap_owned_add_reference state.heap (state.root :: others) focus tree inv.owned inv.focused
  have allocatedOwned := heap_alloc_leaf_owned state.heap (focus :: state.root :: others) fresh cloned
  have childRep := node_rep_alloc state.heap focus tree (.leaf fresh) inv.focused
  have leafRead := heap_alloc_fresh state.heap (.leaf fresh)
  have leafRep : NodeRep allocated state.heap.length (.leaf fresh) := .leaf state.heap.length fresh leafRead
  have owned : HeapOwned allocated ((storedChildren node).map Prod.snd ++ state.root :: others) := by
    by_cases less : oldByte < newByte
    · simpa only [node, storedChildren, storedTwo, if_pos less, List.map_cons, List.map_nil, List.cons_append, List.nil_append]
        using heap_owned_permute allocated (state.heap.length :: focus :: state.root :: others)
          (focus :: state.heap.length :: state.root :: others) (List.Perm.swap _ _ _) allocatedOwned
    · simpa only [node, storedChildren, storedTwo, if_neg less, List.map_cons, List.map_nil, List.cons_append, List.nil_append]
        using allocatedOwned
  have payload : PayloadRep allocated node (.branch pfx none (twoEdges oldByte tree newByte (.leaf fresh))) :=
    .branch pfx none (storedTwo oldByte focus newByte state.heap.length) _
      (stored_two_rep allocated oldByte newByte focus state.heap.length tree (.leaf fresh) childRep leafRep)
  have privateParents : ContextPrivate allocated (state.root :: others) frames := by
    intro frame member
    simpa only [allocated, strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using inv.privateParents frame member
  have savedParents : ContextSaved allocated others frames := by
    intro frame member saved included path
    obtain ⟨old, rep⟩ := inv.owned.rootsRepresented saved (by simp [included])
    exact inv.savedParents frame member saved included (heap_reach_alloc_reflects state.heap saved old (.leaf fresh) rep frame.parent path)
  have avoid : ∀ frame ∈ frames, ∀ edge ∈ storedChildren node, ¬ HeapReach allocated edge.2 frame.parent := by
    intro frame member edge included
    rcases stored_two_member oldByte newByte focus state.heap.length edge included with old | new
    · subst edge
      intro path
      exact heap_context_ancestor_no_return state.heap focus state.root frames inv.context tree inv.focused frame member
        (heap_reach_alloc_reflects state.heap focus tree (.leaf fresh) inv.focused frame.parent path)
    · subst edge
      intro path
      exact (Nat.ne_of_lt (heap_read_bound state.heap frame.parent _
        (heap_context_frame_read state.heap focus state.root frames inv.context frame member)))
        (leaf_reach_self allocated state.heap.length fresh leafRead frame.parent path)
  obtain ⟨next, completed, result⟩ := allocate_assign_payload_result { state with heap := allocated } others focus tree _ frames node owned
    (heap_context_alloc state.heap focus state.root frames (.leaf fresh) inv.context) childRep privateParents savedParents inv.aligned payload avoid
  refine ⟨next, ?_, ⟨result.owned, result.rootValue, ?_⟩⟩
  · simpa [wrapCurrentPairPlace, mutation_invariant_target state others focus tree frames inv, allocated, node] using completed
  · intro saved value member original
    exact result.saved saved value member (node_rep_alloc state.heap saved value (.leaf fresh) original)

end Kv9.Radix
