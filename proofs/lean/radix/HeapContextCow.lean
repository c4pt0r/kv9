import HeapContextUpdate

set_option autoImplicit false

namespace Kv9.Radix

theorem no_reach_children_count_zero (heap : NodeHeap) (child ancestor : NodeId) (node : StoredNode)
    (read : heapRead heap child = some node) (noReturn : ¬ HeapReach heap child ancestor) :
    ((storedChildren node).map Prod.snd).count ancestor = 0 := by
  apply List.count_eq_zero_of_not_mem
  intro member
  obtain ⟨edge, edgeMember, same⟩ := List.mem_map.mp member
  exact noReturn (same ▸ HeapReach.child child edge.2 node edge.1 .root read edgeMember)

-- Copying the focused payload adds references only to its descendants. It
-- cannot make any private ancestor shared, even when descendants are shared.
theorem cloned_child_ancestor_count (heap : NodeHeap) (roots : List NodeId) (parent child ancestor : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (node : StoredNode) (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (read : heapRead heap child = some node)
    (bound : ancestor < heap.length) (noReturn : ¬ HeapReach heap child ancestor) :
    strongCount (cloneChildAt heap parent pfx terminal edges index byte node) roots ancestor =
      strongCount heap roots ancestor := by
  have different : child ≠ ancestor := fun same => noReturn (same ▸ HeapReach.root)
  have freshDifferent : heap.length ≠ ancestor := Nat.ne_of_gt bound
  have none := no_reach_children_count_zero heap child ancestor node read noReturn
  have balance := cloned_child_count_delta heap roots parent child pfx terminal edges index byte node ancestor parentRead access
  simpa only [referenceHit, if_neg different, if_neg freshDifferent, none, Nat.add_zero] using balance

theorem heap_context_alloc_private (heap : NodeHeap) (roots : List NodeId) (hole root : NodeId)
    (frames : List HeapFrame) (tree : Tree) (node : StoredNode)
    (context : HeapContext heap hole root frames) (focused : NodeRep heap hole tree)
    (read : heapRead heap hole = some node) (privateParents : ContextPrivate heap roots frames) :
    ContextPrivate (heapAlloc heap node) roots frames := by
  intro frame member
  rw [strong_count_alloc, no_reach_children_count_zero heap hole frame.parent node read
    (heap_context_ancestor_no_return heap hole root frames context tree focused frame member), Nat.add_zero]
  exact privateParents frame member

theorem heap_context_clone_private (heap : NodeHeap) (roots : List NodeId) (hole root : NodeId)
    (frame : HeapFrame) (tail : List HeapFrame) (tree : Tree) (node : StoredNode) (byte : UInt8)
    (context : HeapContext heap hole root (frame :: tail)) (focused : NodeRep heap hole tree)
    (read : heapRead heap hole = some node) (access : frame.edges[frame.value.index]? = some (byte, hole))
    (privateParents : ContextPrivate heap roots (frame :: tail)) :
    ContextPrivate (cloneChildAt heap frame.parent frame.value.pfx frame.value.terminal frame.edges
      frame.value.index byte node) roots (repointedFrame frame byte heap.length :: tail) := by
  have parentRead := heap_context_frame_read heap hole root (frame :: tail) context frame (by simp)
  have unchanged : ∀ item ∈ frame :: tail,
      strongCount (cloneChildAt heap frame.parent frame.value.pfx frame.value.terminal frame.edges
        frame.value.index byte node) roots item.parent = strongCount heap roots item.parent := by
    intro item member
    exact cloned_child_ancestor_count heap roots frame.parent hole item.parent frame.value.pfx frame.value.terminal
      frame.edges frame.value.index byte node parentRead access read
      (heap_read_bound heap item.parent _ (heap_context_frame_read heap hole root (frame :: tail) context item member))
      (heap_context_ancestor_no_return heap hole root (frame :: tail) context tree focused item member)
  intro item member
  rcases List.mem_cons.mp member with same | later
  · subst item
    change strongCount _ roots frame.parent = 1
    rw [unchanged frame (by simp)]
    exact privateParents frame (by simp)
  · rw [unchanged item (by simp [later])]
    exact privateParents item (by simp [later])

theorem heap_context_clone (heap : NodeHeap) (roots : List NodeId) (hole root : NodeId)
    (frame : HeapFrame) (tail : List HeapFrame) (tree : Tree) (node : StoredNode) (byte : UInt8)
    (context : HeapContext heap hole root (frame :: tail)) (focused : NodeRep heap hole tree)
    (read : heapRead heap hole = some node) (access : frame.edges[frame.value.index]? = some (byte, hole))
    (privateParents : ContextPrivate heap roots (frame :: tail)) :
    let next := cloneChildAt heap frame.parent frame.value.pfx frame.value.terminal frame.edges frame.value.index byte node
    HeapContext next heap.length root (repointedFrame frame byte heap.length :: tail) ∧
      NodeRep next heap.length tree := by
  have allocatedContext := heap_context_alloc heap hole root (frame :: tail) node context
  have allocatedPrivate := heap_context_alloc_private heap roots hole root (frame :: tail) tree node context focused read privateParents
  have allocatedFocus := node_rep_alloc heap hole tree node focused
  have parentRead := heap_context_frame_read heap hole root (frame :: tail) context frame (by simp)
  have parentBound := heap_read_bound heap frame.parent _ parentRead
  have noReturn := heap_context_ancestor_no_return heap hole root (frame :: tail) context tree focused frame (by simp)
  have newFocus := node_rep_write_frame (heapAlloc heap node) heap.length frame.parent tree
    (some (.branch frame.value.pfx frame.value.terminal (repointedFrame frame byte heap.length).edges))
    (clone_node_rep heap hole tree node focused read)
    (clone_separates_ancestor heap hole frame.parent tree node focused read parentBound noReturn)
  have outerSafe : ContextSafe (heapAlloc heap node) frame.parent tail := by
    cases allocatedContext with
    | up _ _ _ _ parent siblings outer =>
        have parentRep : NodeRep (heapAlloc heap node) frame.parent (plugInsert frame.value tree) :=
          .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ parent
            (edge_hole_plug _ hole frame.edges frame.value.index frame.value.children siblings tree allocatedFocus)
        exact heap_context_private_safe _ roots frame.parent root tail outer (plugInsert frame.value tree) parentRep
          (allocatedPrivate frame (by simp)) (fun item member => allocatedPrivate item (by simp [member]))
  exact ⟨heap_context_repoint (heapAlloc heap node) hole heap.length root frame tail tree byte
    allocatedContext allocatedFocus access outerSafe, newFocus⟩

-- This follows the actual makeChildUnique branch. The ghost context is merely
-- repointed when COW allocates; its abstract insertion frames do not change.
theorem make_child_unique_context (heap : NodeHeap) (roots : List NodeId) (hole root : NodeId)
    (frame : HeapFrame) (tail : List HeapFrame) (tree : Tree)
    (context : HeapContext heap hole root (frame :: tail)) (focused : NodeRep heap hole tree)
    (privateParents : ContextPrivate heap roots (frame :: tail)) :
    ∃ next address newFrame, makeChildUnique heap roots frame.parent frame.value.index = some (next, address) ∧
      HeapContext next address root (newFrame :: tail) ∧ NodeRep next address tree ∧
      ContextPrivate next roots (newFrame :: tail) ∧ newFrame.parent = frame.parent ∧ newFrame.value = frame.value := by
  have parentRead := heap_context_frame_read heap hole root (frame :: tail) context frame (by simp)
  have parentOne := privateParents frame (by simp)
  have access : ∃ byte, frame.edges[frame.value.index]? = some (byte, hole) := by
    cases context with
    | up _ _ _ _ _ siblings _ => exact edge_hole_read heap hole frame.edges frame.value.index frame.value.children siblings
  obtain ⟨byte, edgeRead⟩ := access
  obtain ⟨node, read⟩ := node_rep_has_cell heap hole tree focused
  by_cases one : strongCount heap roots hole = 1
  · exact ⟨heap, hole, frame, by simp [makeChildUnique, parentOne, parentRead, edgeRead, read, one],
      context, focused, privateParents, rfl, rfl⟩
  · obtain ⟨newContext, newFocus⟩ := heap_context_clone heap roots hole root frame tail tree node byte
      context focused read edgeRead privateParents
    exact ⟨cloneChildAt heap frame.parent frame.value.pfx frame.value.terminal frame.edges frame.value.index byte node,
      heap.length, repointedFrame frame byte heap.length,
      by simp [makeChildUnique, parentOne, parentRead, edgeRead, read, one], newContext, newFocus,
      heap_context_clone_private heap roots hole root frame tail tree node byte context focused read edgeRead privateParents,
      rfl, rfl⟩

end Kv9.Radix
