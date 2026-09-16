import NativeObjectReclaim

set_option autoImplicit false

namespace Kv9.Radix

-- Unlike makeRootUnique, this operation does not recheck whether the source
-- is shared. The library may already have chosen its copying arm before an
-- external snapshot was released. Keep the old token until assignment, and
-- release it through the ordinary last-owner path, even if it is now unique.
def copyOwnedRoot (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId) := do
  let node ← heapRead heap focus
  let next ← assignRoot (heapAlloc heap node) focus heap.length others
  some (next, heap.length)

theorem copied_payload_count_one (heap : NodeHeap) (roots : List NodeId) (focus : NodeId)
    (tree : Tree) (node : StoredNode) (owned : HeapOwned heap roots)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) :
    strongCount (heapAlloc heap node) (heap.length :: roots) heap.length = 1 := by
  have zero := fresh_identity_unreferenced heap roots (heap_modelled_closed heap owned.modelled) owned.rootsRepresented
  have noChild := represented_children_avoid_fresh heap focus tree node rep read
  rw [strong_count_root_add, strong_count_alloc, zero, List.count_eq_zero_of_not_mem noChild]
  simp [referenceHit]

theorem copied_payload_owned (heap : NodeHeap) (roots : List NodeId) (focus : NodeId)
    (tree : Tree) (node : StoredNode) (owned : HeapOwned heap roots)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) :
    HeapOwned (heapAlloc heap node) (heap.length :: roots) := by
  have newRep := clone_node_rep heap focus tree node rep read
  refine ⟨?_, ?_, ?_⟩
  · intro address stored access
    rcases heap_alloc_read_cases heap node address stored access with ⟨fresh, _⟩ | previous
    · subst address
      exact ⟨tree, newRep⟩
    · obtain ⟨oldTree, oldRep⟩ := owned.modelled address stored previous
      exact ⟨oldTree, node_rep_alloc heap address oldTree node oldRep⟩
  · intro root member
    rcases List.mem_cons.mp member with fresh | previous
    · subst root
      exact ⟨tree, newRep⟩
    · obtain ⟨oldTree, oldRep⟩ := owned.rootsRepresented root previous
      exact ⟨oldTree, node_rep_alloc heap root oldTree node oldRep⟩
  · intro address stored access
    rcases heap_alloc_read_cases heap node address stored access with ⟨fresh, _⟩ | previous
    · subst address
      rw [copied_payload_count_one heap roots focus tree node owned rep read]
      decide
    · have positive := owned.live address stored previous
      rw [strong_count_root_add, strong_count_alloc]
      omega

theorem copy_owned_root_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus tree) :
    ∃ next, copyOwnedRoot heap focus others = some (next, heap.length) ∧ HeapOwned next (heap.length :: others) ∧
      NodeRep next heap.length tree ∧ strongCount next (heap.length :: others) heap.length = 1 ∧
      HeapSeparated next others heap.length ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  have copiedOwned := copied_payload_owned heap (focus :: others) focus tree node owned rep read
  have copiedRep := clone_node_rep heap focus tree node rep read
  obtain ⟨next, assigned, nextOwned, nextRep, preserve⟩ :=
    assign_root_refines (heapAlloc heap node) focus heap.length others tree copiedOwned copiedRep
  have reorder : (heap.length :: focus :: others).Perm ((heap.length :: others) ++ [focus]) :=
    List.Perm.cons heap.length (List.perm_append_singleton focus others).symm
  have smaller := drop_loop_count_le (heapAlloc heap node) next (heap.length :: others) [focus] assigned heap.length
  rw [← strong_count_permute _ _ _ reorder,
    copied_payload_count_one heap (focus :: others) focus tree node owned rep read] at smaller
  have positive := root_reference_positive next heap.length others
  have one : strongCount next (heap.length :: others) heap.length = 1 := by omega
  refine ⟨next, ?_, nextOwned, nextRep, one, unique_root_separates next heap.length others one, ?_⟩
  · simp [copyOwnedRoot, read, assigned]
  · intro saved value member original
    exact preserve saved value member (node_rep_alloc heap saved value node original)

theorem native_copy_owned_root (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (tree : Tree) (node : StoredNode) (layout : NativeObjectLayout) (blocks : ObjectBlocks)
    (limit : Nat) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus tree)
    (read : heapRead heap focus = some node) (objects : NativeObjects heap layout blocks limit)
    (ready : ObjectNodeReady layout (some node) localBlocks limit) (fresh : ObjectsFreshAt blocks heap.length localBlocks) :
    ∃ next, copyOwnedRoot heap focus others = some (next, heap.length) ∧
      NativeObjects next layout (survivingObjectBlocks next (overwriteObjectNode blocks heap.length localBlocks)) limit ∧
      HeapOwned next (heap.length :: others) ∧ NodeRep next heap.length tree ∧
      strongCount next (heap.length :: others) heap.length = 1 ∧ HeapSeparated next others heap.length ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨next, completed, nextOwned, nextRep, one, separate, preserve⟩ := copy_owned_root_refines heap focus others tree owned rep
  have assigned : assignRoot (heapAlloc heap node) focus heap.length others = some next := by
    cases result : assignRoot (heapAlloc heap node) focus heap.length others with
    | none => simp [copyOwnedRoot, read, result] at completed
    | some actual =>
        have same : actual = next := by simpa [copyOwnedRoot, read, result] using completed
        exact congrArg some same
  have allocated := native_objects_allocate heap layout blocks limit node localBlocks objects ready fresh
  exact ⟨next, completed, native_objects_read_back (heapAlloc heap node) next layout _ limit allocated
    (drop_loop_read_back (heapAlloc heap node) next (heap.length :: others) [focus] assigned),
    nextOwned, nextRep, one, separate, preserve⟩

end Kv9.Radix
