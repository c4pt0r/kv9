import HeapAcyclic

set_option autoImplicit false

namespace Kv9.Radix

def cellTargets : Option StoredNode → List NodeId
  | none => []
  | some node => (storedChildren node).map Prod.snd

def heapTargets (heap : NodeHeap) : List NodeId := heap.flatMap cellTargets

theorem node_handles_targets (parent : NodeId) (node : StoredNode) :
    (nodeHandles parent node).map Prod.snd = (storedChildren node).map Prod.snd := by
  simp only [nodeHandles, List.map_map]
  change (storedChildren node).zipIdx.map (Prod.snd ∘ Prod.fst) = _
  rw [← List.map_map, List.zipIdx_map_fst]

theorem indexed_flatmap_values {A B : Type} (values : List A) (offset : Nat) (f : A → List B) :
    (values.zipIdx offset).flatMap (fun pair => f pair.1) = values.flatMap f := by
  induction values generalizing offset with
  | nil => rfl
  | cons head tail ih => simp [ih]

theorem arc_handles_targets (heap : NodeHeap) (roots : List NodeId) :
    (arcHandles heap roots).map Prod.snd = roots ++ heapTargets heap := by
  simp only [arcHandles, List.map_append, List.map_map, List.map_flatMap, Function.comp_def]
  rw [List.zipIdx_map_fst]
  apply congrArg (List.append roots)
  calc
    _ = heap.zipIdx.flatMap (fun pair => cellTargets pair.1) := by
      apply congrArg (fun f => heap.zipIdx.flatMap f)
      funext pair
      cases cell : pair.1 with
      | none => rfl
      | some node => exact node_handles_targets pair.2 node
    _ = heapTargets heap := indexed_flatmap_values heap 0 cellTargets

theorem strong_count_values (heap : NodeHeap) (roots : List NodeId) (address : NodeId) :
    strongCount heap roots address = roots.count address + (heapTargets heap).count address := by
  rw [strongCount, ← List.countP_eq_length_filter]
  have mapped : List.countP (fun pair : ArcSlot × NodeId => pair.2 == address) (arcHandles heap roots) =
      ((arcHandles heap roots).map Prod.snd).count address := by
    simp only [List.count_eq_countP, List.countP_map, Function.comp_def]
  rw [mapped, arc_handles_targets, List.count_append]

def HeapClosed (heap : NodeHeap) : Prop :=
  ∀ node, some node ∈ heap → ∀ edge ∈ storedChildren node, ∃ child, heapRead heap edge.2 = some child

theorem heap_targets_bound (heap : NodeHeap) (closed : HeapClosed heap) (address : NodeId)
    (member : address ∈ heapTargets heap) : address < heap.length := by
  obtain ⟨cell, present, target⟩ := List.mem_flatMap.mp member
  cases cell with
  | none => simp [cellTargets] at target
  | some node =>
      obtain ⟨edge, edgeMember, same⟩ := List.mem_map.mp target
      obtain ⟨child, read⟩ := closed node present edge edgeMember
      rw [← same]
      exact heap_read_bound heap edge.2 child read

theorem represented_heap_closed (heap : NodeHeap)
    (represented : ∀ address node, heapRead heap address = some node → ∃ tree, NodeRep heap address tree) :
    HeapClosed heap := by
  intro node present edge member
  obtain ⟨address, access⟩ := List.mem_iff_getElem?.mp present
  have read : heapRead heap address = some node := by simp only [heapRead, access, Option.join_some]
  obtain ⟨tree, rep⟩ := represented address node read
  obtain ⟨childTree, childRep, _⟩ := node_rep_child_work heap address tree node edge.1 edge.2 rep read member
  exact node_rep_has_cell heap edge.2 childTree childRep

theorem heap_targets_alloc (heap : NodeHeap) (node : StoredNode) :
    heapTargets (heapAlloc heap node) = heapTargets heap ++ (storedChildren node).map Prod.snd := by
  simp [heapTargets, heapAlloc, cellTargets]

theorem cloned_root_count_one (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (node : StoredNode) (closed : HeapClosed heap) (rep : NodeRep heap focus tree)
    (read : heapRead heap focus = some node) (represented : ∀ root ∈ others, ∃ old, NodeRep heap root old) :
    strongCount (heapAlloc heap node) (heap.length :: others) heap.length = 1 := by
  have noRoots : heap.length ∉ others := by
    intro member
    obtain ⟨old, oldRep⟩ := represented heap.length member
    obtain ⟨stored, access⟩ := node_rep_has_cell heap heap.length old oldRep
    exact Nat.lt_irrefl heap.length (heap_read_bound heap heap.length stored access)
  have noOld : heap.length ∉ heapTargets heap := fun member =>
    Nat.lt_irrefl heap.length (heap_targets_bound heap closed heap.length member)
  have noNew : heap.length ∉ (storedChildren node).map Prod.snd := by
    intro member
    obtain ⟨edge, edgeMember, same⟩ := List.mem_map.mp member
    obtain ⟨childTree, childRep, _⟩ := node_rep_child_work heap focus tree node edge.1 edge.2 rep read edgeMember
    obtain ⟨stored, access⟩ := node_rep_has_cell heap edge.2 childTree childRep
    have bound := heap_read_bound heap edge.2 stored access
    rw [same] at bound
    exact Nat.lt_irrefl heap.length bound
  simp only [strong_count_values, heap_targets_alloc, List.count_cons, beq_self_eq_true, if_true,
    List.count_append, List.count_eq_zero_of_not_mem noRoots, List.count_eq_zero_of_not_mem noOld,
    List.count_eq_zero_of_not_mem noNew, Nat.zero_add, Nat.add_zero]

theorem make_root_unique_count (heap next : NodeHeap) (focus address : NodeId) (others : List NodeId) (tree : Tree)
    (closed : HeapClosed heap) (rep : NodeRep heap focus tree)
    (represented : ∀ root ∈ others, ∃ old, NodeRep heap root old)
    (completed : makeRootUnique heap focus others = some (next, address)) :
    strongCount next (address :: others) address = 1 := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  by_cases unique : strongCount heap (focus :: others) focus = 1
  · have same : (heap, focus) = (next, address) := by simpa [makeRootUnique, read, unique] using completed
    cases same
    exact unique
  · have same : (heapAlloc heap node, heap.length) = (next, address) := by
      simpa [makeRootUnique, read, unique] using completed
    cases same
    exact cloned_root_count_one heap focus others tree node closed rep read represented

end Kv9.Radix
