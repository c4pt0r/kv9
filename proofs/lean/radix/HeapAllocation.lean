import HeapAssign

set_option autoImplicit false

namespace Kv9.Radix

-- An owned payload before Arc::new: its child Arcs are external temporary
-- tokens until moved into the new node's edges. Entry/Box/Vec payload storage
-- remains subject to the separate native storage correspondence.
inductive PayloadRep (heap : NodeHeap) : StoredNode → Tree → Prop where
  | leaf (entry : Entry) : PayloadRep heap (.leaf entry) (.leaf entry)
  | branch (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
      (rep : EdgesRep heap edges children) : PayloadRep heap (.branch pfx terminal edges) (.branch pfx terminal children)

theorem payload_alloc_rep (heap : NodeHeap) (node : StoredNode) (tree : Tree) (rep : PayloadRep heap node tree) :
    NodeRep (heapAlloc heap node) heap.length tree := by
  cases rep with
  | leaf entry => exact .leaf heap.length entry (heap_alloc_fresh heap (.leaf entry))
  | branch pfx terminal edges children descendants =>
      exact .branch heap.length pfx terminal edges children (heap_alloc_fresh heap (.branch pfx terminal edges))
        (edges_rep_alloc heap edges children (.branch pfx terminal edges) descendants)

theorem payload_allocation_count (heap : NodeHeap) (roots : List NodeId) (node : StoredNode) (address : NodeId) :
    strongCount (heapAlloc heap node) (heap.length :: roots) address =
      strongCount heap ((storedChildren node).map Prod.snd ++ roots) address + referenceHit heap.length address := by
  rw [strong_count_root_add, strong_count_alloc, strong_count_roots_append]
  omega

theorem payload_allocation_owned (heap : NodeHeap) (roots : List NodeId) (node : StoredNode) (tree : Tree)
    (owned : HeapOwned heap ((storedChildren node).map Prod.snd ++ roots)) (rep : PayloadRep heap node tree) :
    HeapOwned (heapAlloc heap node) (heap.length :: roots) := by
  have freshRep := payload_alloc_rep heap node tree rep
  constructor
  · intro address stored read
    rcases heap_alloc_read_cases heap node address stored read with ⟨fresh, _⟩ | previous
    · subst address
      exact ⟨tree, freshRep⟩
    · obtain ⟨oldTree, oldRep⟩ := owned.modelled address stored previous
      exact ⟨oldTree, node_rep_alloc heap address oldTree node oldRep⟩
  · intro root member
    rcases List.mem_cons.mp member with fresh | previous
    · subst root
      exact ⟨tree, freshRep⟩
    · obtain ⟨oldTree, oldRep⟩ := owned.rootsRepresented root (List.mem_append_right _ previous)
      exact ⟨oldTree, node_rep_alloc heap root oldTree node oldRep⟩
  · intro address stored read
    rw [payload_allocation_count]
    rcases heap_alloc_read_cases heap node address stored read with ⟨fresh, _⟩ | previous
    · subst address
      simp [referenceHit]
    · have positive := owned.live address stored previous
      omega

theorem payload_allocation_unique (heap : NodeHeap) (roots : List NodeId) (node : StoredNode)
    (owned : HeapOwned heap ((storedChildren node).map Prod.snd ++ roots)) :
    strongCount (heapAlloc heap node) (heap.length :: roots) heap.length = 1 := by
  rw [payload_allocation_count,
    fresh_identity_unreferenced heap _ (heap_modelled_closed heap owned.modelled) owned.rootsRepresented]
  simp [referenceHit]

end Kv9.Radix
