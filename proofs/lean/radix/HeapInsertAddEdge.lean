import HeapEdgeInsert

set_option autoImplicit false

namespace Kv9.Radix

def addEdgePlace (state : MutHeap) (others : List NodeId) (depth index : Nat) (fresh : Entry) : Option (MutHeap × Bool) := do
  let (next, focus) ← makePlaceUnique state others
  match heapRead next.heap focus with
  | some (.branch pfx terminal edges) =>
      let byte ← fresh.key[depth + pfx.length]?
      if index ≤ edges.length then
        let allocated := heapAlloc next.heap (.leaf fresh)
        let node := StoredNode.branch pfx terminal (storedInsert edges index byte next.heap.length)
        some ({ next with heap := heapWrite allocated focus (some node) }, true)
      else none
  | _ => none

theorem add_edge_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (depth index : Nat) (byte : UInt8) (fresh : Entry)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (access : fresh.key[depth + pfx.length]? = some byte) (bound : index ≤ edgeCount children) :
    ∃ next, addEdgePlace state others depth index fresh = some (next, true) ∧
      MutationResult state others frames (.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)) next := by
  obtain ⟨unique, parent, uniqueFrames, completed, uniqueInv, one, separate, values, preserve⟩ :=
    make_place_unique_invariant state others focus (.branch pfx terminal children) frames inv
  have stored : ∃ edges, heapRead unique.heap parent = some (.branch pfx terminal edges) ∧ EdgesRep unique.heap edges children := by
    cases uniqueInv.focused with
    | branch _ _ _ edges _ read descendants => exact ⟨edges, read, descendants⟩
  obtain ⟨edges, read, descendants⟩ := stored
  have indexBound : index ≤ edges.length := by rw [edges_rep_length unique.heap edges children descendants]; exact bound
  let allocated := heapAlloc unique.heap (.leaf fresh)
  let node := StoredNode.branch pfx terminal (storedInsert edges index byte unique.heap.length)
  let replacement := Tree.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)
  have allocatedOwned := heap_alloc_leaf_owned unique.heap (unique.root :: others) fresh uniqueInv.owned
  have parentRead := heap_alloc_preserves_read unique.heap parent (.branch pfx terminal edges) (.leaf fresh) read
  have parentRep := node_rep_alloc unique.heap parent (.branch pfx terminal children) (.leaf fresh) uniqueInv.focused
  have freshRead := heap_alloc_fresh unique.heap (.leaf fresh)
  have freshRep : NodeRep allocated unique.heap.length (.leaf fresh) := .leaf unique.heap.length fresh freshRead
  have different := Nat.ne_of_lt (heap_read_bound unique.heap parent _ read)
  have noReturn : ¬ HeapReach allocated unique.heap.length parent := fun path =>
    different (leaf_reach_self allocated unique.heap.length fresh freshRead parent path)
  have nextOwned := insert_edge_token_owned allocated (unique.root :: others) parent unique.heap.length pfx terminal edges children
    index byte (.leaf fresh) allocatedOwned parentRep parentRead freshRep noReturn
  have focused : NodeRep (heapWrite allocated parent (some node)) parent replacement :=
    insert_edge_refines allocated parent unique.heap.length pfx terminal edges children index byte (.leaf fresh) parentRep parentRead freshRep noReturn
  have context := heap_context_alloc unique.heap parent unique.root uniqueFrames (.leaf fresh) uniqueInv.context
  have allocatedOne : strongCount allocated (unique.root :: others) parent = 1 := by
    simpa only [allocated, strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using one
  have privateParents : ContextPrivate allocated (unique.root :: others) uniqueFrames := by
    intro frame member
    simpa only [allocated, strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using uniqueInv.privateParents frame member
  have safe := heap_context_private_safe allocated (unique.root :: others) parent unique.root uniqueFrames context
    (.branch pfx terminal children) parentRep allocatedOne privateParents
  have rootValue := heap_context_write_value allocated parent unique.root uniqueFrames (some node) replacement context safe focused
  refine ⟨{ unique with heap := heapWrite allocated parent (some node) }, ?_, ⟨nextOwned, ?_, ?_⟩⟩
  · simp [addEdgePlace, completed, read, access, indexBound, allocated, node]
  · simpa only [values] using rootValue
  · intro saved value member original
    have previous := preserve saved value member original
    have allocatedSeparate : ¬ HeapReach allocated saved parent := fun path =>
      separate saved member (heap_reach_alloc_reflects unique.heap saved value (.leaf fresh) previous parent path)
    exact node_rep_write_frame allocated saved parent value (some node)
      (node_rep_alloc unique.heap saved value (.leaf fresh) previous) allocatedSeparate

theorem add_edge_place_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (depth index : Nat) (byte : UInt8) (fresh : Entry)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (access : fresh.key[depth + pfx.length]? = some byte) (bound : index ≤ edgeCount children) :
    executeInsert depth fresh (.branch pfx terminal children) (.addEdge index) =
      .done (.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)) true ∧
    ∃ next, addEdgePlace state others depth index fresh = some (next, true) ∧
      MutationResult state others frames (.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)) next :=
  ⟨by simp [executeInsert, access, bound], add_edge_place_refines state others focus pfx terminal children frames depth index byte fresh inv access bound⟩

end Kv9.Radix
