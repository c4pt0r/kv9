import HeapReferenceDelta

set_option autoImplicit false

namespace Kv9.Radix

def RootsModelled (heap : NodeHeap) (roots : List NodeId) : Prop :=
  ∀ root ∈ roots, ∃ tree, NodeRep heap root tree

def HeapLive (heap : NodeHeap) (roots : List NodeId) : Prop :=
  ∀ address node, heapRead heap address = some node → 0 < strongCount heap roots address

-- This is an ownership inventory invariant. Native Arc counters and buffers,
-- and the transitions that release the final owner, require separate proofs.
structure HeapOwned (heap : NodeHeap) (roots : List NodeId) : Prop where
  modelled : HeapModelled heap
  rootsRepresented : RootsModelled heap roots
  live : HeapLive heap roots

theorem fresh_identity_unreferenced (heap : NodeHeap) (roots : List NodeId)
    (closed : HeapClosed heap) (represented : RootsModelled heap roots) :
    strongCount heap roots heap.length = 0 := by
  have noRoots := represented_roots_avoid_fresh heap roots represented
  have noEdges : heap.length ∉ heapTargets heap := fun member =>
    Nat.lt_irrefl heap.length (heap_targets_bound heap closed heap.length member)
  rw [strong_count_values, List.count_eq_zero_of_not_mem noRoots, List.count_eq_zero_of_not_mem noEdges]

theorem cloned_root_live (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (node : StoredNode) (live : HeapLive heap (focus :: others)) (closed : HeapClosed heap)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node)
    (represented : RootsModelled heap others) (shared : strongCount heap (focus :: others) focus ≠ 1) :
    HeapLive (heapAlloc heap node) (heap.length :: others) := by
  intro address stored access
  rcases heap_alloc_read_cases heap node address stored access with ⟨fresh, _⟩ | previous
  · subst address
    rw [cloned_root_count_one heap focus others tree node closed rep read represented]
    decide
  · have positive := live address stored previous
    have balance := cloned_root_count_delta heap focus others node address
    by_cases same : focus = address
    · have notOne : strongCount heap (focus :: others) address ≠ 1 := by simpa only [same] using shared
      simp only [referenceHit, if_pos same] at balance
      omega
    · simp only [referenceHit, if_neg same] at balance
      omega

theorem make_root_unique_live (heap next : NodeHeap) (focus address : NodeId) (others : List NodeId)
    (tree : Tree) (live : HeapLive heap (focus :: others)) (closed : HeapClosed heap)
    (rep : NodeRep heap focus tree) (represented : RootsModelled heap others)
    (completed : makeRootUnique heap focus others = some (next, address)) : HeapLive next (address :: others) := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  by_cases unique : strongCount heap (focus :: others) focus = 1
  · have same : (heap, focus) = (next, address) := by simpa [makeRootUnique, read, unique] using completed
    cases same
    exact live
  · have same : (heapAlloc heap node, heap.length) = (next, address) := by
      simpa [makeRootUnique, read, unique] using completed
    cases same
    exact cloned_root_live heap focus others tree node live closed rep read represented unique

theorem make_root_unique_owned (heap next : NodeHeap) (focus address : NodeId) (others : List NodeId)
    (owned : HeapOwned heap (focus :: others))
    (completed : makeRootUnique heap focus others = some (next, address)) : HeapOwned next (address :: others) := by
  obtain ⟨tree, rep⟩ := owned.rootsRepresented focus (by simp)
  have represented : RootsModelled heap others := fun root member => owned.rootsRepresented root (by simp [member])
  obtain ⟨actualHeap, actualAddress, result, newRep, _, preserve⟩ :=
    make_root_unique_refines heap focus others tree rep represented
  have same := Option.some.inj (result.symm.trans completed)
  cases same
  refine ⟨make_root_unique_modelled heap next focus address others tree owned.modelled rep completed, ?_,
    make_root_unique_live heap next focus address others tree owned.live
      (heap_modelled_closed heap owned.modelled) rep represented completed⟩
  intro root member
  rcases List.mem_cons.mp member with atNew | old
  · subst root
    exact ⟨tree, newRep⟩
  · obtain ⟨oldTree, oldRep⟩ := represented root old
    exact ⟨oldTree, preserve root oldTree oldRep⟩

theorem clone_child_read_cases (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (index : Nat) (byte : UInt8) (node : StoredNode)
    (address : NodeId) (stored : StoredNode)
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (read : heapRead (cloneChildAt heap parent pfx terminal edges index byte node) address = some stored) :
    address = heap.length ∨ ∃ old, heapRead heap address = some old := by
  by_cases same : address = parent
  · exact Or.inr ⟨.branch pfx terminal edges, by simpa only [same] using parentRead⟩
  · rw [cloneChildAt, heap_write_elsewhere _ parent address _ same] at read
    rcases heap_alloc_read_cases heap node address stored read with ⟨fresh, _⟩ | previous
    · exact Or.inl fresh
    · exact Or.inr ⟨stored, previous⟩

theorem cloned_child_live (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (node : StoredNode) (childTree : Tree)
    (owned : HeapOwned heap roots) (shared : strongCount heap roots child ≠ 1)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (childRep : NodeRep heap child childTree)
    (childRead : heapRead heap child = some node) :
    HeapLive (cloneChildAt heap parent pfx terminal edges index byte node) roots := by
  intro address stored read
  rcases clone_child_read_cases heap parent pfx terminal edges index byte node address stored parentRead read with fresh | ⟨old, previous⟩
  · subst address
    rw [cloned_child_count_one heap roots parent child pfx terminal edges children index byte node childTree
      (heap_modelled_closed heap owned.modelled) owned.rootsRepresented parentRep parentRead access childRep childRead]
    decide
  · have positive := owned.live address old previous
    have balance := cloned_child_count_delta heap roots parent child pfx terminal edges index byte node address parentRead access
    by_cases same : child = address
    · have notOne : strongCount heap roots address ≠ 1 := by simpa only [same] using shared
      simp only [referenceHit, if_pos same] at balance
      omega
    · simp only [referenceHit, if_neg same] at balance
      omega

theorem make_child_unique_owned (heap next : NodeHeap) (roots : List NodeId) (parent child address : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (owned : HeapOwned heap roots) (parentOne : strongCount heap roots parent = 1)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child))
    (completed : makeChildUnique heap roots parent index = some (next, address)) : HeapOwned next roots := by
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep parentRead
  obtain ⟨childTree, pureAccess, childRep⟩ := edges_rep_get heap edges children index byte child descendants access
  obtain ⟨node, childRead⟩ := node_rep_has_cell heap child childTree childRep
  by_cases unique : strongCount heap roots child = 1
  · have same : (heap, child) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    exact owned
  · have same : (cloneChildAt heap parent pfx terminal edges index byte node, heap.length) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    refine ⟨cloned_child_modelled heap parent child pfx terminal edges children index byte node childTree owned.modelled
      parentRep parentRead access pureAccess childRep childRead, ?_,
      cloned_child_live heap roots parent child pfx terminal edges children index byte node childTree owned unique
        parentRep parentRead access childRep childRead⟩
    intro root member
    obtain ⟨tree, rep⟩ := owned.rootsRepresented root member
    exact ⟨tree, clone_child_preserves_all_roots heap parent child pfx terminal edges children index byte node childTree
      parentRep parentRead access pureAccess childRep childRead root tree rep⟩

theorem heap_owned_empty : HeapOwned [] [] := by
  constructor <;> simp [HeapModelled, RootsModelled, HeapLive, heapRead]

theorem heap_owned_add_reference (heap : NodeHeap) (roots : List NodeId) (focus : NodeId) (tree : Tree)
    (owned : HeapOwned heap roots) (rep : NodeRep heap focus tree) : HeapOwned heap (focus :: roots) := by
  refine ⟨owned.modelled, ?_, ?_⟩
  · intro root member
    rcases List.mem_cons.mp member with selected | previous
    · subst root
      exact ⟨tree, rep⟩
    · exact owned.rootsRepresented root previous
  · intro address node read
    have positive := owned.live address node read
    rw [strong_count_root_add]
    omega

theorem heap_alloc_leaf_owned (heap : NodeHeap) (roots : List NodeId) (entry : Entry)
    (owned : HeapOwned heap roots) : HeapOwned (heapAlloc heap (.leaf entry)) (heap.length :: roots) := by
  have freshRep : NodeRep (heapAlloc heap (.leaf entry)) heap.length (.leaf entry) :=
    .leaf heap.length entry (heap_alloc_fresh heap (.leaf entry))
  have freshZero := fresh_identity_unreferenced heap roots (heap_modelled_closed heap owned.modelled) owned.rootsRepresented
  constructor
  · intro address stored read
    rcases heap_alloc_read_cases heap (.leaf entry) address stored read with ⟨fresh, _⟩ | previous
    · subst address
      exact ⟨.leaf entry, freshRep⟩
    · obtain ⟨tree, rep⟩ := owned.modelled address stored previous
      exact ⟨tree, node_rep_alloc heap address tree (.leaf entry) rep⟩
  · intro root member
    rcases List.mem_cons.mp member with fresh | previous
    · subst root
      exact ⟨.leaf entry, freshRep⟩
    · obtain ⟨tree, rep⟩ := owned.rootsRepresented root previous
      exact ⟨tree, node_rep_alloc heap root tree (.leaf entry) rep⟩
  · intro address stored read
    rcases heap_alloc_read_cases heap (.leaf entry) address stored read with ⟨fresh, _⟩ | previous
    · subst address
      simp [strong_count_root_add, strong_count_alloc, storedChildren, referenceHit, freshZero]
    · have positive := owned.live address stored previous
      simp only [strong_count_root_add, strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero]
      omega

end Kv9.Radix
