import HeapChildCounts

set_option autoImplicit false

namespace Kv9.Radix

mutual
  theorem node_rep_rewrite (heap next : NodeHeap) (focus : NodeId) (focused : Tree)
      (oldFocus : NodeRep heap focus focused) (newFocus : NodeRep next focus focused)
      (elsewhere : ∀ address node, address ≠ focus → heapRead heap address = some node → heapRead next address = some node)
      (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree) : NodeRep next root tree := by
    by_cases same : root = focus
    · subst root
      have value := node_rep_unique heap focus tree focused rep oldFocus
      rw [value]
      exact newFocus
    · cases rep with
      | leaf _ entry read => exact .leaf root entry (elsewhere root (.leaf entry) same read)
      | branch _ pfx terminal edges children read descendants =>
          exact .branch root pfx terminal edges children (elsewhere root (.branch pfx terminal edges) same read)
            (edges_rep_rewrite heap next focus focused oldFocus newFocus elsewhere edges children descendants)
  theorem edges_rep_rewrite (heap next : NodeHeap) (focus : NodeId) (focused : Tree)
      (oldFocus : NodeRep heap focus focused) (newFocus : NodeRep next focus focused)
      (elsewhere : ∀ address node, address ≠ focus → heapRead heap address = some node → heapRead next address = some node)
      (edges : List StoredEdge) (children : Forest) (rep : EdgesRep heap edges children) : EdgesRep next edges children := by
    cases rep with
    | nil => exact .nil
    | cons byte address tail child rest node remaining =>
        exact .cons byte address tail child rest
          (node_rep_rewrite heap next focus focused oldFocus newFocus elsewhere address child node)
          (edges_rep_rewrite heap next focus focused oldFocus newFocus elsewhere tail rest remaining)
end

def HeapModelled (heap : NodeHeap) : Prop :=
  ∀ address node, heapRead heap address = some node → ∃ tree, NodeRep heap address tree

theorem heap_modelled_closed (heap : NodeHeap) (modelled : HeapModelled heap) : HeapClosed heap :=
  represented_heap_closed heap modelled

theorem heap_alloc_read_cases (heap : NodeHeap) (fresh : StoredNode) (address : NodeId) (node : StoredNode)
    (read : heapRead (heapAlloc heap fresh) address = some node) :
    (address = heap.length ∧ node = fresh) ∨ heapRead heap address = some node := by
  by_cases old : address < heap.length
  · apply Or.inr
    simpa only [heapRead, heapAlloc, List.getElem?_append_left old] using read
  · have bound := heap_read_bound (heapAlloc heap fresh) address node read
    have within : address ≤ heap.length := Nat.le_of_lt_succ (by simpa [heapAlloc] using bound)
    have atEnd : address = heap.length := Nat.le_antisymm within (Nat.le_of_not_gt old)
    rw [atEnd, heap_alloc_fresh] at read
    exact Or.inl ⟨atEnd, (Option.some.inj read).symm⟩

theorem heap_modelled_clone (heap : NodeHeap) (focus : NodeId) (tree : Tree) (node : StoredNode)
    (modelled : HeapModelled heap) (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) :
    HeapModelled (heapAlloc heap node) := by
  intro address stored access
  rcases heap_alloc_read_cases heap node address stored access with ⟨fresh, _⟩ | previous
  · subst address
    exact ⟨tree, clone_node_rep heap focus tree node rep read⟩
  · obtain ⟨old, original⟩ := modelled address stored previous
    exact ⟨old, node_rep_alloc heap address old node original⟩

theorem make_root_unique_modelled (heap next : NodeHeap) (focus address : NodeId) (others : List NodeId) (tree : Tree)
    (modelled : HeapModelled heap) (rep : NodeRep heap focus tree)
    (completed : makeRootUnique heap focus others = some (next, address)) : HeapModelled next := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  by_cases unique : strongCount heap (focus :: others) focus = 1
  · have same : (heap, focus) = (next, address) := by simpa [makeRootUnique, read, unique] using completed
    cases same
    exact modelled
  · have same : (heapAlloc heap node, heap.length) = (next, address) := by
      simpa [makeRootUnique, read, unique] using completed
    cases same
    exact heap_modelled_clone heap focus tree node modelled rep read

theorem clone_child_preserves_all_roots (heap : NodeHeap) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (node : StoredNode) (childTree : Tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (pureAccess : edgeGet index children = some (byte, childTree))
    (childRep : NodeRep heap child childTree) (childRead : heapRead heap child = some node)
    (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree) :
    NodeRep (cloneChildAt heap parent pfx terminal edges index byte node) root tree := by
  have newParent := (clone_child_at_refines heap [] parent child pfx terminal edges children index byte node childTree
    parentRep parentRead access pureAccess childRep childRead (by simp) (by simp [HeapSeparated])).1
  apply node_rep_rewrite heap (cloneChildAt heap parent pfx terminal edges index byte node)
    parent (.branch pfx terminal children) parentRep newParent
  · intro address stored different read
    rw [cloneChildAt, heap_write_elsewhere _ parent address _ different]
    exact heap_alloc_preserves_read heap address stored node read
  · exact rep

theorem cloned_child_modelled (heap : NodeHeap) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (node : StoredNode) (childTree : Tree) (modelled : HeapModelled heap)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (pureAccess : edgeGet index children = some (byte, childTree))
    (childRep : NodeRep heap child childTree) (childRead : heapRead heap child = some node) :
    HeapModelled (cloneChildAt heap parent pfx terminal edges index byte node) := by
  have copies := clone_child_at_refines heap [] parent child pfx terminal edges children index byte node childTree
    parentRep parentRead access pureAccess childRep childRead (by simp) (by simp [HeapSeparated])
  intro address stored read
  by_cases isParent : address = parent
  · subst address
    exact ⟨.branch pfx terminal children, copies.1⟩
  · rw [cloneChildAt, heap_write_elsewhere _ parent address _ isParent] at read
    rcases heap_alloc_read_cases heap node address stored read with ⟨fresh, _⟩ | previous
    · subst address
      exact ⟨childTree, copies.2.1⟩
    · obtain ⟨tree, rep⟩ := modelled address stored previous
      exact ⟨tree, clone_child_preserves_all_roots heap parent child pfx terminal edges children index byte node childTree
        parentRep parentRead access pureAccess childRep childRead address tree rep⟩

theorem make_child_unique_modelled (heap next : NodeHeap) (roots : List NodeId) (parent child address : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (parentOne : strongCount heap roots parent = 1) (modelled : HeapModelled heap)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child))
    (completed : makeChildUnique heap roots parent index = some (next, address)) : HeapModelled next := by
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep parentRead
  obtain ⟨childTree, pureAccess, childRep⟩ := edges_rep_get heap edges children index byte child descendants access
  obtain ⟨node, childRead⟩ := node_rep_has_cell heap child childTree childRep
  by_cases unique : strongCount heap roots child = 1
  · have same : (heap, child) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    exact modelled
  · have same : (cloneChildAt heap parent pfx terminal edges index byte node, heap.length) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    exact cloned_child_modelled heap parent child pfx terminal edges children index byte node childTree modelled
      parentRep parentRead access pureAccess childRep childRead

end Kv9.Radix
