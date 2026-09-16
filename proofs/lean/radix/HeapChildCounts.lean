import HeapChild

set_option autoImplicit false

namespace Kv9.Radix

theorem heap_targets_write_fresh_count (heap : NodeHeap) (index : Nat) (replacement : Option StoredNode)
    (address : NodeId) (bound : index < heap.length) (absent : (heapTargets heap).count address = 0) :
    (heapTargets (heapWrite heap index replacement)).count address = (cellTargets replacement).count address := by
  induction index generalizing heap with
  | zero =>
      cases heap with
      | nil => simp at bound
      | cons head tail =>
          have parts : (cellTargets head).count address = 0 ∧ (heapTargets tail).count address = 0 :=
            Nat.add_eq_zero_iff.mp (by simpa only [heapTargets, List.flatMap_cons, List.count_append] using absent)
          change (cellTargets replacement ++ heapTargets tail).count address = _
          rw [List.count_append, parts.2, Nat.add_zero]
  | succ n ih =>
      cases heap with
      | nil => simp at bound
      | cons head tail =>
          have parts : (cellTargets head).count address = 0 ∧ (heapTargets tail).count address = 0 :=
            Nat.add_eq_zero_iff.mp (by simpa only [heapTargets, List.flatMap_cons, List.count_append] using absent)
          change (cellTargets head ++ heapTargets (heapWrite tail n replacement)).count address = _
          rw [List.count_append, parts.1, ih tail (Nat.lt_of_succ_lt_succ bound) parts.2, Nat.zero_add]

theorem repoint_fresh_count (edges : List StoredEdge) (index : Nat) (byte : UInt8) (old fresh : NodeId)
    (access : edges[index]? = some (byte, old)) (absent : fresh ∉ edges.map Prod.snd) :
    ((edges.set index (byte, fresh)).map Prod.snd).count fresh = 1 := by
  induction index generalizing edges with
  | zero =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have noTail : fresh ∉ tail.map Prod.snd := fun member => absent (by simp [member])
          simp [List.set, List.count_eq_zero_of_not_mem noTail]
  | succ n ih =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have different : head.2 ≠ fresh := fun same => absent (by simp [same])
          have noTail : fresh ∉ tail.map Prod.snd := fun member => absent (by simp [member])
          simpa [List.set, different] using ih tail access noTail

theorem represented_roots_avoid_fresh (heap : NodeHeap) (roots : List NodeId)
    (represented : ∀ root ∈ roots, ∃ tree, NodeRep heap root tree) : heap.length ∉ roots := by
  intro member
  obtain ⟨tree, rep⟩ := represented heap.length member
  obtain ⟨node, read⟩ := node_rep_has_cell heap heap.length tree rep
  exact Nat.lt_irrefl heap.length (heap_read_bound heap heap.length node read)

theorem represented_children_avoid_fresh (heap : NodeHeap) (focus : NodeId) (tree : Tree) (node : StoredNode)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) :
    heap.length ∉ (storedChildren node).map Prod.snd := by
  intro member
  obtain ⟨edge, edgeMember, same⟩ := List.mem_map.mp member
  obtain ⟨childTree, childRep, _⟩ := node_rep_child_work heap focus tree node edge.1 edge.2 rep read edgeMember
  obtain ⟨stored, access⟩ := node_rep_has_cell heap edge.2 childTree childRep
  have bound := heap_read_bound heap edge.2 stored access
  rw [same] at bound
  exact Nat.lt_irrefl heap.length bound

theorem allocated_clone_no_incoming (heap : NodeHeap) (focus : NodeId) (tree : Tree) (node : StoredNode)
    (closed : HeapClosed heap) (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) :
    (heapTargets (heapAlloc heap node)).count heap.length = 0 := by
  have noOld : heap.length ∉ heapTargets heap := fun member =>
    Nat.lt_irrefl heap.length (heap_targets_bound heap closed heap.length member)
  have noNew := represented_children_avoid_fresh heap focus tree node rep read
  rw [heap_targets_alloc, List.count_append, List.count_eq_zero_of_not_mem noOld,
    List.count_eq_zero_of_not_mem noNew]

theorem cloned_child_count_one (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (node : StoredNode) (childTree : Tree) (closed : HeapClosed heap)
    (represented : ∀ root ∈ roots, ∃ tree, NodeRep heap root tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (childRep : NodeRep heap child childTree)
    (childRead : heapRead heap child = some node) :
    strongCount (cloneChildAt heap parent pfx terminal edges index byte node) roots heap.length = 1 := by
  have noRoots := represented_roots_avoid_fresh heap roots represented
  have noPrevious := allocated_clone_no_incoming heap child childTree node closed childRep childRead
  have parentBound := heap_read_bound heap parent (.branch pfx terminal edges) parentRead
  have bound : parent < (heapAlloc heap node).length := Nat.lt_of_lt_of_le parentBound (by simp [heapAlloc])
  have noEdges := represented_children_avoid_fresh heap parent (.branch pfx terminal children)
    (.branch pfx terminal edges) parentRep parentRead
  rw [strong_count_values, List.count_eq_zero_of_not_mem noRoots, Nat.zero_add, cloneChildAt,
    heap_targets_write_fresh_count (heapAlloc heap node) parent _ heap.length bound noPrevious]
  exact repoint_fresh_count edges index byte child heap.length access noEdges

theorem make_child_unique_count (heap next : NodeHeap) (roots : List NodeId) (parent child address : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (parentOne : strongCount heap roots parent = 1) (closed : HeapClosed heap)
    (represented : ∀ root ∈ roots, ∃ tree, NodeRep heap root tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child))
    (completed : makeChildUnique heap roots parent index = some (next, address)) : strongCount next roots address = 1 := by
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep parentRead
  obtain ⟨childTree, _, childRep⟩ := edges_rep_get heap edges children index byte child descendants access
  obtain ⟨node, childRead⟩ := node_rep_has_cell heap child childTree childRep
  by_cases unique : strongCount heap roots child = 1
  · have same : (heap, child) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    exact unique
  · have same : (cloneChildAt heap parent pfx terminal edges index byte node, heap.length) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, childRead, unique] using completed
    cases same
    exact cloned_child_count_one heap roots parent child pfx terminal edges children index byte node childTree closed
      represented parentRep parentRead access childRep childRead

end Kv9.Radix
