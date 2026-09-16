import HeapRelease

set_option autoImplicit false

namespace Kv9.Radix

theorem positive_count_owner (heap : NodeHeap) (roots : List NodeId) (address : NodeId)
    (positive : 0 < strongCount heap roots address) :
    address ∈ roots ∨ ∃ parent node byte, heapRead heap parent = some node ∧ (byte, address) ∈ storedChildren node := by
  rw [strong_count_values] at positive
  by_cases external : address ∈ roots
  · exact Or.inl external
  · have rootZero := List.count_eq_zero_of_not_mem external
    have edgePositive : 0 < (heapTargets heap).count address := by omega
    have edgeMember := List.count_pos_iff.mp edgePositive
    obtain ⟨cell, present, target⟩ := List.mem_flatMap.mp edgeMember
    cases cell with
    | none => simp [cellTargets] at target
    | some node =>
        obtain ⟨parent, access⟩ := List.mem_iff_getElem?.mp present
        obtain ⟨edge, member, same⟩ := List.mem_map.mp target
        refine Or.inr ⟨parent, node, edge.1, ?_, ?_⟩
        · simp only [heapRead, access, Option.join_some]
        · have pair : edge = (edge.1, address) := by rw [← same]
          simpa only [← pair] using member

theorem finite_function_bound (f : Nat → Nat) (size : Nat) : ∃ bound, ∀ index < size, f index < bound := by
  induction size with
  | zero => exact ⟨0, by intro index invalid; omega⟩
  | succ size ih =>
      obtain ⟨bound, previous⟩ := ih
      refine ⟨bound + f size + 1, ?_⟩
      intro index within
      by_cases last : index = size
      · subst index
        omega
      · have old := previous index (by omega)
        omega

-- Used only to select a finite proof bound, never by an executable transition.
noncomputable def representedWork (heap : NodeHeap) (address : NodeId) : Nat := by
  classical
  exact if represented : ∃ tree, NodeRep heap address tree then treeWork (Classical.choose represented) else 0

theorem represented_work_eq (heap : NodeHeap) (address : NodeId) (tree : Tree) (rep : NodeRep heap address tree) :
    representedWork heap address = treeWork tree := by
  classical
  have present : ∃ value, NodeRep heap address value := ⟨tree, rep⟩
  rw [representedWork, dif_pos present]
  rw [node_rep_unique heap address (Classical.choose present) tree (Classical.choose_spec present) rep]

theorem finite_heap_work_bound (heap : NodeHeap) :
    ∃ bound, ∀ address tree, NodeRep heap address tree → treeWork tree < bound := by
  obtain ⟨bound, limits⟩ := finite_function_bound (representedWork heap) heap.length
  refine ⟨bound, ?_⟩
  intro address tree rep
  obtain ⟨node, read⟩ := node_rep_has_cell heap address tree rep
  have within := limits address (heap_read_bound heap address node read)
  simpa only [represented_work_eq heap address tree rep] using within

theorem owned_node_reachable_bounded (heap : NodeHeap) (roots : List NodeId) (owned : HeapOwned heap roots)
    (bound : Nat) (limits : ∀ address tree, NodeRep heap address tree → treeWork tree < bound)
    (address : NodeId) (tree : Tree) (rep : NodeRep heap address tree) :
    ∃ root ∈ roots, HeapReach heap root address := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap address tree rep
  rcases positive_count_owner heap roots address (owned.live address node read) with external | ⟨parent, stored, byte, parentRead, edge⟩
  · exact ⟨address, external, .root⟩
  · obtain ⟨parentTree, parentRep⟩ := owned.modelled parent stored parentRead
    obtain ⟨childTree, childRep, smaller⟩ := node_rep_child_work heap parent parentTree stored byte address parentRep parentRead edge
    have same := node_rep_unique heap address childTree tree childRep rep
    subst childTree
    have parentBound := limits parent parentTree parentRep
    obtain ⟨root, member, reachable⟩ := owned_node_reachable_bounded heap roots owned bound limits parent parentTree parentRep
    exact ⟨root, member, .child parent address stored byte reachable parentRead edge⟩
termination_by bound - treeWork tree
decreasing_by omega

theorem owned_node_reachable (heap : NodeHeap) (roots : List NodeId) (owned : HeapOwned heap roots)
    (address : NodeId) (tree : Tree) (rep : NodeRep heap address tree) :
    ∃ root ∈ roots, HeapReach heap root address := by
  obtain ⟨bound, limits⟩ := finite_heap_work_bound heap
  exact owned_node_reachable_bounded heap roots owned bound limits address tree rep

theorem owned_cell_reachable (heap : NodeHeap) (roots : List NodeId) (owned : HeapOwned heap roots)
    (address : NodeId) (node : StoredNode) (read : heapRead heap address = some node) :
    ∃ root ∈ roots, HeapReach heap root address := by
  obtain ⟨tree, rep⟩ := owned.modelled address node read
  exact owned_node_reachable heap roots owned address tree rep

theorem owned_no_roots_no_cells (heap : NodeHeap) (owned : HeapOwned heap []) (address : NodeId) :
    heapRead heap address = none := by
  cases read : heapRead heap address with
  | none => rfl
  | some node =>
      obtain ⟨root, member, _⟩ := owned_cell_reachable heap [] owned address node read
      simp at member

end Kv9.Radix
