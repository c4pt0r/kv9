import HeapInsertAddEdge

set_option autoImplicit false

namespace Kv9.Radix

theorem assign_place_subtree_result (state : MutHeap) (others : List NodeId) (old fresh : NodeId)
    (oldTree replacement : Tree) (frames : List HeapFrame)
    (owned : HeapOwned state.heap (fresh :: state.root :: others))
    (context : HeapContext state.heap old state.root frames) (focused : NodeRep state.heap old oldTree)
    (privateParents : ContextPrivate state.heap (state.root :: others) frames)
    (savedParents : ContextSaved state.heap others frames) (aligned : PlaceAligned state.place frames)
    (newRep : NodeRep state.heap fresh replacement)
    (avoid : ∀ frame ∈ frames, ¬ HeapReach state.heap fresh frame.parent) :
    ∃ next, assignPlace state others fresh = some next ∧ MutationResult state others frames replacement next := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          cases frames with
          | cons _ _ => exact False.elim aligned
          | nil =>
              obtain ⟨next, completed, nextOwned, newRoot, preserve⟩ := assign_root_refines heap root fresh others replacement owned newRep
              exact ⟨⟨next, fresh, .root⟩, by simp [assignPlace, completed], ⟨nextOwned, newRoot, preserve⟩⟩
      | edge parent index =>
          cases frames with
          | nil => exact False.elim aligned
          | cons frame tail =>
              obtain ⟨parentEq, indexEq⟩ := aligned
              have details : ∃ byte, frame.edges[frame.value.index]? = some (byte, old) ∧
                  heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges) ∧
                  NodeRep heap frame.parent (plugInsert frame.value oldTree) ∧ HeapReach heap root frame.parent := by
                cases context with
                | up _ _ _ _ read siblings outer =>
                    obtain ⟨byte, access⟩ := edge_hole_read heap old frame.edges frame.value.index frame.value.children siblings
                    exact ⟨byte, access, read, .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
                      (edge_hole_plug heap old frame.edges frame.value.index frame.value.children siblings oldTree focused),
                      heap_context_reach heap frame.parent root tail outer⟩
              obtain ⟨byte, access, read, parentRep, reachable⟩ := details
              have noReturn := avoid frame (by simp)
              obtain ⟨next, completed, nextOwned, _, _, _, preserve⟩ := assign_edge_refines heap (root :: others)
                frame.parent old fresh root frame.value.pfx frame.value.terminal frame.edges
                (edgeSet frame.value.index oldTree frame.value.children) frame.value.index byte replacement
                owned parentRep read access newRep noReturn (by simp) reachable
              refine ⟨⟨next, root, .edge parent index⟩, ?_, ⟨nextOwned, ?_, ?_⟩⟩
              · simp only [assignPlace, ← parentEq, ← indexEq, completed, Option.map_some]
              · exact assign_edge_context_root heap next (root :: others) old fresh root frame tail oldTree replacement
                  owned (by simp) context focused privateParents newRep noReturn completed
              · intro saved value member original
                exact preserve saved value (by simp [member]) (savedParents frame (by simp) saved member) original

theorem payload_child_rep (heap : NodeHeap) (node : StoredNode) (tree : Tree) (rep : PayloadRep heap node tree)
    (edge : StoredEdge) (member : edge ∈ storedChildren node) : ∃ child, NodeRep heap edge.2 child := by
  cases rep with
  | leaf => simp [storedChildren] at member
  | branch pfx terminal edges children descendants => exact edges_rep_member heap edges children descendants edge member

-- A newly allocated branch reaches only itself or an old descendant of one
-- of its owned child slots. In particular, wrapping a focus creates no edge
-- back to any of the existing mutable-place ancestors.
theorem payload_alloc_reach (heap : NodeHeap) (node : StoredNode) (tree : Tree) (rep : PayloadRep heap node tree)
    (address : NodeId) (reachable : HeapReach (heapAlloc heap node) heap.length address) :
    address = heap.length ∨ ∃ edge ∈ storedChildren node, HeapReach heap edge.2 address := by
  induction reachable with
  | root => exact Or.inl rfl
  | child parent address stored byte _ read member ih =>
      apply Or.inr
      rcases ih with fresh | ⟨edge, childMember, previous⟩
      · rw [fresh, heap_alloc_fresh] at read
        have same := Option.some.inj read
        subst stored
        exact ⟨(byte, address), member, .root⟩
      · obtain ⟨child, childRep⟩ := payload_child_rep heap node tree rep edge childMember
        obtain ⟨parentTree, parentRep⟩ := node_rep_reachable heap edge.2 child childRep parent previous
        obtain ⟨old, oldRead⟩ := node_rep_has_cell heap parent parentTree parentRep
        have same := Option.some.inj ((heap_alloc_preserves_read heap parent old node oldRead).symm.trans read)
        subst old
        exact ⟨edge, childMember, .child parent address stored byte previous oldRead member⟩

theorem payload_alloc_avoids (heap : NodeHeap) (node : StoredNode) (tree : Tree) (rep : PayloadRep heap node tree)
    (ancestor : NodeId) (bound : ancestor < heap.length)
    (childrenAvoid : ∀ edge ∈ storedChildren node, ¬ HeapReach heap edge.2 ancestor) :
    ¬ HeapReach (heapAlloc heap node) heap.length ancestor := by
  intro reachable
  rcases payload_alloc_reach heap node tree rep ancestor reachable with fresh | ⟨edge, member, path⟩
  · exact (Nat.ne_of_lt bound) fresh
  · exact childrenAvoid edge member path

theorem payload_children_avoid_count (heap : NodeHeap) (node : StoredNode) (ancestor : NodeId)
    (avoid : ∀ edge ∈ storedChildren node, ¬ HeapReach heap edge.2 ancestor) :
    ((storedChildren node).map Prod.snd).count ancestor = 0 := by
  apply List.count_eq_zero_of_not_mem
  intro member
  obtain ⟨edge, edgeMember, same⟩ := List.mem_map.mp member
  exact avoid edge edgeMember (same ▸ HeapReach.root)

theorem allocate_assign_payload_result (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree replacement : Tree) (frames : List HeapFrame) (node : StoredNode)
    (owned : HeapOwned state.heap ((storedChildren node).map Prod.snd ++ state.root :: others))
    (context : HeapContext state.heap focus state.root frames) (focused : NodeRep state.heap focus tree)
    (privateParents : ContextPrivate state.heap (state.root :: others) frames)
    (savedParents : ContextSaved state.heap others frames) (aligned : PlaceAligned state.place frames)
    (payload : PayloadRep state.heap node replacement)
    (avoid : ∀ frame ∈ frames, ∀ edge ∈ storedChildren node, ¬ HeapReach state.heap edge.2 frame.parent) :
    ∃ next, assignPlace { state with heap := heapAlloc state.heap node } others state.heap.length = some next ∧
      MutationResult state others frames replacement next := by
  have allocatedOwned := payload_allocation_owned state.heap (state.root :: others) node replacement owned payload
  have nextContext := heap_context_alloc state.heap focus state.root frames node context
  have nextFocus := node_rep_alloc state.heap focus tree node focused
  have nextPrivate : ContextPrivate (heapAlloc state.heap node) (state.root :: others) frames := by
    intro frame member
    rw [strong_count_alloc, payload_children_avoid_count state.heap node frame.parent (avoid frame member), Nat.add_zero]
    exact privateParents frame member
  have nextSaved : ContextSaved (heapAlloc state.heap node) others frames := by
    intro frame member saved included reachable
    obtain ⟨old, rep⟩ := owned.rootsRepresented saved (List.mem_append_right _ (by simp [included]))
    exact savedParents frame member saved included (heap_reach_alloc_reflects state.heap saved old node rep frame.parent reachable)
  have freshAvoid : ∀ frame ∈ frames, ¬ HeapReach (heapAlloc state.heap node) state.heap.length frame.parent := by
    intro frame member
    exact payload_alloc_avoids state.heap node replacement payload frame.parent
      (heap_read_bound state.heap frame.parent _ (heap_context_frame_read state.heap focus state.root frames context frame member)) (avoid frame member)
  obtain ⟨next, completed, result⟩ := assign_place_subtree_result { state with heap := heapAlloc state.heap node } others
    focus state.heap.length tree replacement frames allocatedOwned nextContext nextFocus nextPrivate nextSaved aligned
    (payload_alloc_rep state.heap node replacement payload) freshAvoid
  exact ⟨next, completed, ⟨result.owned, result.rootValue,
    fun saved old included original => result.saved saved old included (node_rep_alloc state.heap saved old node original)⟩⟩

end Kv9.Radix
