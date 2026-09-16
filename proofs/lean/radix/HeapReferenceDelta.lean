import HeapExamples

set_option autoImplicit false

namespace Kv9.Radix

-- Additive conservation avoids truncated subtraction when a selected reference
-- is removed. Indicators describe actual owning slots, not mutable borrows.
def referenceHit (target address : NodeId) : Nat := if target = address then 1 else 0

theorem strong_count_root_add (heap : NodeHeap) (roots : List NodeId) (target address : NodeId) :
    strongCount heap (target :: roots) address = strongCount heap roots address + referenceHit target address := by
  simp [strong_count_values, referenceHit, List.count_cons, Nat.add_assoc, Nat.add_comm, Nat.add_left_comm]

theorem strong_count_alloc (heap : NodeHeap) (roots : List NodeId) (node : StoredNode) (address : NodeId) :
    strongCount (heapAlloc heap node) roots address =
      strongCount heap roots address + ((storedChildren node).map Prod.snd).count address := by
  simp only [strong_count_values, heap_targets_alloc, List.count_append, Nat.add_assoc]

theorem heap_targets_write_delta (heap : NodeHeap) (index : Nat) (old replacement : Option StoredNode)
    (address : NodeId) (access : heap[index]? = some old) :
    (heapTargets (heapWrite heap index replacement)).count address + (cellTargets old).count address =
      (heapTargets heap).count address + (cellTargets replacement).count address := by
  induction index generalizing heap with
  | zero =>
      cases heap with
      | nil => simp at access
      | cons head tail =>
          have same : head = old := Option.some.inj access
          subst head
          simp [heapWrite, heapTargets, Nat.add_comm, Nat.add_left_comm, Nat.add_assoc]
  | succ index ih =>
      cases heap with
      | nil => simp at access
      | cons head tail =>
          have step := ih tail access
          change (cellTargets head ++ heapTargets (heapWrite tail index replacement)).count address + _ =
            (cellTargets head ++ heapTargets tail).count address + _
          simp only [List.count_append]
          omega

theorem strong_count_write_delta (heap : NodeHeap) (roots : List NodeId) (index : Nat)
    (old replacement : Option StoredNode) (address : NodeId) (access : heap[index]? = some old) :
    strongCount (heapWrite heap index replacement) roots address + (cellTargets old).count address =
      strongCount heap roots address + (cellTargets replacement).count address := by
  have balance := heap_targets_write_delta heap index old replacement address access
  simp only [strong_count_values]
  omega

theorem edge_repoint_count_delta (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (old fresh address : NodeId) (access : edges[index]? = some (byte, old)) :
    ((edges.set index (byte, fresh)).map Prod.snd).count address + referenceHit old address =
      (edges.map Prod.snd).count address + referenceHit fresh address := by
  induction index generalizing edges with
  | zero =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have same : head = (byte, old) := Option.some.inj access
          subst head
          simp [referenceHit, List.count_cons, Nat.add_comm, Nat.add_left_comm]
  | succ index ih =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have step := ih tail access
          simp only [List.set_cons_succ, List.map_cons, List.count_cons]
          split <;> omega

theorem cloned_root_count_delta (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (node : StoredNode) (address : NodeId) :
    strongCount (heapAlloc heap node) (heap.length :: others) address + referenceHit focus address =
      strongCount heap (focus :: others) address +
        ((storedChildren node).map Prod.snd).count address + referenceHit heap.length address := by
  rw [strong_count_root_add, strong_count_alloc, strong_count_root_add]
  omega

theorem cloned_child_count_delta (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (node : StoredNode) (address : NodeId)
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) :
    strongCount (cloneChildAt heap parent pfx terminal edges index byte node) roots address + referenceHit child address =
      strongCount heap roots address + ((storedChildren node).map Prod.snd).count address + referenceHit heap.length address := by
  have parentKept := heap_alloc_preserves_read heap parent (.branch pfx terminal edges) node parentRead
  have write := strong_count_write_delta (heapAlloc heap node) roots parent
    (some (.branch pfx terminal edges)) (some (.branch pfx terminal (edges.set index (byte, heap.length))))
    address (heap_read_cell _ _ _ parentKept)
  have edge := edge_repoint_count_delta edges index byte child heap.length address access
  rw [strong_count_alloc] at write
  simp only [cellTargets, storedChildren] at write
  change strongCount (heapWrite (heapAlloc heap node) parent
    (some (.branch pfx terminal (edges.set index (byte, heap.length))))) roots address + _ = _
  simp only [storedChildren]
  omega

theorem root_reference_positive (heap : NodeHeap) (focus : NodeId) (others : List NodeId) :
    0 < strongCount heap (focus :: others) focus := by
  rw [strong_count_root_add]
  simp [referenceHit]

end Kv9.Radix
