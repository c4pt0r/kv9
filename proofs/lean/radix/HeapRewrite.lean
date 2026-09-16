import HeapDropExamples

set_option autoImplicit false

namespace Kv9.Radix

-- Changing a private node can change its active ancestors' values. The earlier
-- equal-value substitution theorem is insufficient for those mutation states.
-- A represented replacement plus unchanged other cells preserves finite
-- interpretation, without assuming every old root keeps the same value.
mutual
  theorem node_rep_update_exists (heap next : NodeHeap) (focus : NodeId) (replacement : Tree)
      (focused : NodeRep next focus replacement)
      (elsewhere : ∀ address node, address ≠ focus → heapRead heap address = some node → heapRead next address = some node)
      (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree) : ∃ result, NodeRep next root result := by
    by_cases same : root = focus
    · subst root
      exact ⟨replacement, focused⟩
    · cases rep with
      | leaf _ entry read => exact ⟨.leaf entry, .leaf root entry (elsewhere root (.leaf entry) same read)⟩
      | branch _ pfx terminal edges children read descendants =>
          obtain ⟨updated, updatedRep⟩ := edges_rep_update_exists heap next focus replacement focused elsewhere edges children descendants
          exact ⟨.branch pfx terminal updated,
            .branch root pfx terminal edges updated (elsewhere root (.branch pfx terminal edges) same read) updatedRep⟩
  theorem edges_rep_update_exists (heap next : NodeHeap) (focus : NodeId) (replacement : Tree)
      (focused : NodeRep next focus replacement)
      (elsewhere : ∀ address node, address ≠ focus → heapRead heap address = some node → heapRead next address = some node)
      (edges : List StoredEdge) (children : Forest) (rep : EdgesRep heap edges children) :
      ∃ result, EdgesRep next edges result := by
    cases rep with
    | nil => exact ⟨.nil, .nil⟩
    | cons byte address tail child rest node remaining =>
        obtain ⟨newChild, childRep⟩ := node_rep_update_exists heap next focus replacement focused elsewhere address child node
        obtain ⟨newRest, restRep⟩ := edges_rep_update_exists heap next focus replacement focused elsewhere tail rest remaining
        exact ⟨.cons byte newChild newRest, .cons byte address tail newChild newRest childRep restRep⟩
end

theorem heap_write_modelled (heap : NodeHeap) (focus : NodeId) (node : StoredNode) (tree : Tree)
    (modelled : HeapModelled heap) (rep : NodeRep (heapWrite heap focus (some node)) focus tree) :
    HeapModelled (heapWrite heap focus (some node)) := by
  intro address stored read
  by_cases same : address = focus
  · subst address
    exact ⟨tree, rep⟩
  · rw [heap_write_elsewhere heap focus address (some node) same] at read
    obtain ⟨oldTree, oldRep⟩ := modelled address stored read
    exact node_rep_update_exists heap (heapWrite heap focus (some node)) focus tree rep
      (fun other value different original => by
        rw [heap_write_elsewhere heap focus other (some node) different]
        exact original) address oldTree oldRep

theorem heap_write_same_targets_count (heap : NodeHeap) (roots : List NodeId) (focus : NodeId)
    (old node : StoredNode) (address : NodeId) (read : heapRead heap focus = some old)
    (targets : storedChildren node = storedChildren old) :
    strongCount (heapWrite heap focus (some node)) roots address = strongCount heap roots address := by
  have balance := strong_count_write_delta heap roots focus (some old) (some node) address (heap_read_cell heap focus old read)
  simp only [cellTargets, targets] at balance
  omega

theorem heap_write_same_targets_live (heap : NodeHeap) (roots : List NodeId) (focus : NodeId)
    (old node : StoredNode) (live : HeapLive heap roots) (read : heapRead heap focus = some old)
    (targets : storedChildren node = storedChildren old) : HeapLive (heapWrite heap focus (some node)) roots := by
  intro address stored access
  rw [heap_write_same_targets_count heap roots focus old node address read targets]
  by_cases same : address = focus
  · subst address
    exact live focus old read
  · rw [heap_write_elsewhere heap focus address (some node) same] at access
    exact live address stored access

theorem heap_write_same_targets_owned (heap : NodeHeap) (roots : List NodeId) (focus : NodeId)
    (old node : StoredNode) (tree : Tree) (owned : HeapOwned heap roots) (read : heapRead heap focus = some old)
    (targets : storedChildren node = storedChildren old) (rep : NodeRep (heapWrite heap focus (some node)) focus tree) :
    HeapOwned (heapWrite heap focus (some node)) roots := by
  refine ⟨heap_write_modelled heap focus node tree owned.modelled rep, ?_,
    heap_write_same_targets_live heap roots focus old node owned.live read targets⟩
  intro root member
  obtain ⟨oldTree, oldRep⟩ := owned.rootsRepresented root member
  exact node_rep_update_exists heap (heapWrite heap focus (some node)) focus tree rep
    (fun other value different original => by
      rw [heap_write_elsewhere heap focus other (some node) different]
      exact original) root oldTree oldRep

theorem write_leaf_payload_refines (heap : NodeHeap) (focus : NodeId) (old fresh : Entry)
    (read : heapRead heap focus = some (.leaf old)) :
    NodeRep (heapWrite heap focus (some (.leaf fresh))) focus (.leaf fresh) :=
  .leaf focus fresh (heap_write_here heap focus (some (.leaf fresh)) (heap_read_bound heap focus (.leaf old) read))

theorem write_branch_payload_refines (heap : NodeHeap) (focus : NodeId) (oldPfx newPfx : Key)
    (oldTerminal newTerminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (rep : NodeRep heap focus (.branch oldPfx oldTerminal children))
    (read : heapRead heap focus = some (.branch oldPfx oldTerminal edges)) :
    NodeRep (heapWrite heap focus (some (.branch newPfx newTerminal edges))) focus (.branch newPfx newTerminal children) := by
  have descendants := node_rep_branch_edges heap focus oldPfx oldTerminal edges children rep read
  apply NodeRep.branch focus newPfx newTerminal edges children
  · exact heap_write_here heap focus _ (heap_read_bound heap focus _ read)
  · apply edges_rep_write_frame heap edges children focus _ descendants
    intro edge member
    exact represented_edge_no_return heap focus (.branch oldPfx oldTerminal children)
      (.branch oldPfx oldTerminal edges) edge.1 edge.2 rep read member

end Kv9.Radix
