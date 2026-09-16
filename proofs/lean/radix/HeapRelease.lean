import HeapLive

set_option autoImplicit false

namespace Kv9.Radix

theorem strong_count_roots_append (heap : NodeHeap) (left right : List NodeId) (address : NodeId) :
    strongCount heap (left ++ right) address = left.count address + strongCount heap right address := by
  simp only [strong_count_values, List.count_append, Nat.add_assoc]

theorem unique_external_separates_node (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (root : NodeId) (unique : strongCount heap (focus :: others) focus = 1) (different : root ≠ focus) :
    ¬ HeapReach heap root focus := by
  intro reachable
  cases reachable with
  | root => exact different rfl
  | child parent _ node byte _ read edge =>
      obtain ⟨index, access⟩ := List.mem_iff_getElem?.mp edge
      have owner : arcSlotRead heap (focus :: others) (.edge parent index) = some focus := by
        simp only [arcSlotRead, read, Option.bind_some, access, Option.map_some]
      have impossible := strong_one_unique_slot heap (focus :: others) focus (.root 0) (.edge parent index) unique rfl owner
      cases impossible

-- One owned Arc token is consumed. A non-final release keeps the node. A final
-- release transfers its outgoing tokens to the caller's owned-reference list.
-- Worklist order and native atomic handoff are composed separately; this does
-- not recursively collect descendants or assume their final ownership.
def releaseExternal (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × List NodeId) :=
  (heapRead heap focus).map (fun node =>
    if strongCount heap (focus :: others) focus = 1 then
      (heapWrite heap focus none, (storedChildren node).map Prod.snd ++ others)
    else (heap, others))

theorem released_last_count_delta (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (node : StoredNode) (address : NodeId) (read : heapRead heap focus = some node) :
    strongCount (heapWrite heap focus none) ((storedChildren node).map Prod.snd ++ others) address + referenceHit focus address =
      strongCount heap (focus :: others) address := by
  have balance := strong_count_write_delta heap others focus (some node) none address (heap_read_cell heap focus node read)
  rw [strong_count_roots_append, strong_count_root_add]
  simp only [cellTargets, List.count_nil, Nat.add_zero] at balance
  omega

theorem released_last_modelled (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (node : StoredNode)
    (modelled : HeapModelled heap) (read : heapRead heap focus = some node)
    (unique : strongCount heap (focus :: others) focus = 1) : HeapModelled (heapWrite heap focus none) := by
  intro address stored access
  by_cases same : address = focus
  · subst address
    rw [heap_write_here heap focus none (heap_read_bound heap focus node read)] at access
    contradiction
  · rw [heap_write_elsewhere heap focus address none same] at access
    obtain ⟨tree, rep⟩ := modelled address stored access
    exact ⟨tree, node_rep_write_frame heap address focus tree none rep
      (unique_external_separates_node heap focus others address unique same)⟩

theorem released_last_roots_modelled (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (node : StoredNode)
    (owned : HeapOwned heap (focus :: others)) (read : heapRead heap focus = some node)
    (unique : strongCount heap (focus :: others) focus = 1) :
    RootsModelled (heapWrite heap focus none) ((storedChildren node).map Prod.snd ++ others) := by
  intro root member
  rcases List.mem_append.mp member with child | other
  · obtain ⟨edge, edgeMember, rootEq⟩ := List.mem_map.mp child
    obtain ⟨tree, rep⟩ := owned.modelled focus node read
    obtain ⟨childTree, childRep, _⟩ := node_rep_child_work heap focus tree node edge.1 edge.2 rep read edgeMember
    have separate := represented_edge_no_return heap focus tree node edge.1 edge.2 rep read edgeMember
    rw [← rootEq]
    exact ⟨childTree, node_rep_write_frame heap edge.2 focus childTree none childRep separate⟩
  · obtain ⟨tree, rep⟩ := owned.rootsRepresented root (by simp [other])
    exact ⟨tree, node_rep_write_frame heap root focus tree none rep
      (unique_root_separates heap focus others unique root other)⟩

theorem released_last_live (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (node : StoredNode)
    (live : HeapLive heap (focus :: others)) (read : heapRead heap focus = some node) :
    HeapLive (heapWrite heap focus none) ((storedChildren node).map Prod.snd ++ others) := by
  intro address stored access
  by_cases same : address = focus
  · subst address
    rw [heap_write_here heap focus none (heap_read_bound heap focus node read)] at access
    contradiction
  · rw [heap_write_elsewhere heap focus address none same] at access
    have positive := live address stored access
    have balance := released_last_count_delta heap focus others node address read
    simp only [referenceHit, if_neg (Ne.symm same), Nat.add_zero] at balance
    omega

theorem release_external_owned (heap next : NodeHeap) (focus : NodeId) (others remaining : List NodeId)
    (owned : HeapOwned heap (focus :: others))
    (completed : releaseExternal heap focus others = some (next, remaining)) : HeapOwned next remaining := by
  obtain ⟨tree, rep⟩ := owned.rootsRepresented focus (by simp)
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  by_cases unique : strongCount heap (focus :: others) focus = 1
  · have same : (heapWrite heap focus none, (storedChildren node).map Prod.snd ++ others) = (next, remaining) := by
      simpa [releaseExternal, read, unique] using completed
    cases same
    exact ⟨released_last_modelled heap focus others node owned.modelled read unique,
      released_last_roots_modelled heap focus others node owned read unique,
      released_last_live heap focus others node owned.live read⟩
  · have same : (heap, others) = (next, remaining) := by simpa [releaseExternal, read, unique] using completed
    cases same
    refine ⟨owned.modelled, fun root member => owned.rootsRepresented root (by simp [member]), ?_⟩
    intro address stored access
    have positive := owned.live address stored access
    rw [strong_count_root_add] at positive
    by_cases atFocus : focus = address
    · have notOne : strongCount heap (focus :: others) address ≠ 1 := by simpa only [atFocus] using unique
      rw [strong_count_root_add] at notOne
      simp only [referenceHit, if_pos atFocus] at notOne
      omega
    · simp only [referenceHit, if_neg atFocus, Nat.add_zero] at positive
      exact positive

theorem release_external_no_failure (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (represented : RootsModelled heap (focus :: others)) : releaseExternal heap focus others ≠ none := by
  obtain ⟨tree, rep⟩ := represented focus (by simp)
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  simp [releaseExternal, read]

theorem release_external_count_delta (heap next : NodeHeap) (focus : NodeId) (others remaining : List NodeId)
    (address : NodeId) (completed : releaseExternal heap focus others = some (next, remaining)) :
    strongCount next remaining address + referenceHit focus address = strongCount heap (focus :: others) address := by
  cases read : heapRead heap focus with
  | none => simp [releaseExternal, read] at completed
  | some node =>
      by_cases unique : strongCount heap (focus :: others) focus = 1
      · have same : (heapWrite heap focus none, (storedChildren node).map Prod.snd ++ others) = (next, remaining) := by
          simpa [releaseExternal, read, unique] using completed
        cases same
        exact released_last_count_delta heap focus others node address read
      · have same : (heap, others) = (next, remaining) := by simpa [releaseExternal, read, unique] using completed
        cases same
        exact (strong_count_root_add heap others focus address).symm

theorem release_external_preserves_remaining (heap next : NodeHeap) (focus : NodeId) (others remaining : List NodeId)
    (root : NodeId) (tree : Tree) (member : root ∈ others) (rep : NodeRep heap root tree)
    (completed : releaseExternal heap focus others = some (next, remaining)) : NodeRep next root tree := by
  cases read : heapRead heap focus with
  | none => simp [releaseExternal, read] at completed
  | some node =>
      by_cases unique : strongCount heap (focus :: others) focus = 1
      · have same : (heapWrite heap focus none, (storedChildren node).map Prod.snd ++ others) = (next, remaining) := by
          simpa [releaseExternal, read, unique] using completed
        cases same
        exact node_rep_write_frame heap root focus tree none rep (unique_root_separates heap focus others unique root member)
      · have same : (heap, others) = (next, remaining) := by simpa [releaseExternal, read, unique] using completed
        cases same
        exact rep

theorem heap_targets_write_length_delta (heap : NodeHeap) (index : Nat) (old replacement : Option StoredNode)
    (access : heap[index]? = some old) :
    (heapTargets (heapWrite heap index replacement)).length + (cellTargets old).length =
      (heapTargets heap).length + (cellTargets replacement).length := by
  induction index generalizing heap with
  | zero =>
      cases heap with
      | nil => simp at access
      | cons head tail =>
          have same : head = old := Option.some.inj access
          subst head
          simp [heapWrite, heapTargets, Nat.add_comm, Nat.add_left_comm]
  | succ index ih =>
      cases heap with
      | nil => simp at access
      | cons head tail =>
          have step := ih tail access
          change (cellTargets head ++ heapTargets (heapWrite tail index replacement)).length + _ =
            (cellTargets head ++ heapTargets tail).length + _
          simp only [List.length_append]
          omega

theorem release_external_potential (heap next : NodeHeap) (focus : NodeId) (others remaining : List NodeId)
    (completed : releaseExternal heap focus others = some (next, remaining)) :
    (heapTargets next).length + remaining.length + 1 = (heapTargets heap).length + (focus :: others).length := by
  cases read : heapRead heap focus with
  | none => simp [releaseExternal, read] at completed
  | some node =>
      by_cases unique : strongCount heap (focus :: others) focus = 1
      · have same : (heapWrite heap focus none, (storedChildren node).map Prod.snd ++ others) = (next, remaining) := by
          simpa [releaseExternal, read, unique] using completed
        cases same
        have balance := heap_targets_write_length_delta heap focus (some node) none (heap_read_cell heap focus node read)
        simp only [cellTargets, List.length_nil, List.length_map, Nat.add_zero] at balance
        simp only [List.length_append, List.length_map, List.length_cons]
        omega
      · have same : (heap, others) = (next, remaining) := by simpa [releaseExternal, read, unique] using completed
        cases same
        rfl

end Kv9.Radix
