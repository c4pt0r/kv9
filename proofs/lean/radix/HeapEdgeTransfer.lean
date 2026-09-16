import HeapDropFrame

set_option autoImplicit false

namespace Kv9.Radix

theorem edges_rep_replace_tree (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (old fresh : NodeId) (tree : Tree)
    (rep : EdgesRep heap edges children) (access : edges[index]? = some (byte, old))
    (replacement : NodeRep heap fresh tree) :
    EdgesRep heap (edges.set index (byte, fresh)) (edgeSet index tree children) := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => simp at access
      | cons storedByte storedAddress tail child rest _ remaining =>
          have same : storedByte = byte ∧ storedAddress = old := by simpa using access
          rcases same with ⟨rfl, rfl⟩
          exact .cons _ fresh tail tree rest replacement remaining
  | succ index ih =>
      cases rep with
      | nil => simp at access
      | cons storedByte storedAddress tail child rest node remaining =>
          exact .cons storedByte storedAddress (tail.set index (byte, fresh)) child (edgeSet index tree rest) node
            (ih tail rest remaining access)

theorem edge_token_transfer_count (heap : NodeHeap) (roots : List NodeId) (parent old fresh : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (address : NodeId) (read : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, old)) :
    strongCount (heapWrite heap parent (some (.branch pfx terminal (edges.set index (byte, fresh))))) (old :: roots) address =
      strongCount heap (fresh :: roots) address := by
  have balance := strong_count_write_delta heap roots parent (some (.branch pfx terminal edges))
    (some (.branch pfx terminal (edges.set index (byte, fresh)))) address (heap_read_cell heap parent _ read)
  have edge := edge_repoint_count_delta edges index byte old fresh address access
  simp only [cellTargets, storedChildren] at balance
  rw [strong_count_root_add, strong_count_root_add]
  omega

theorem write_edge_refines (heap : NodeHeap) (parent old fresh : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (children : Forest) (index : Nat) (byte : UInt8) (tree : Tree)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, old))
    (replacement : NodeRep heap fresh tree) (noReturn : ¬ HeapReach heap fresh parent) :
    NodeRep (heapWrite heap parent (some (.branch pfx terminal (edges.set index (byte, fresh)))))
      parent (.branch pfx terminal (edgeSet index tree children)) := by
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children rep read
  apply NodeRep.branch parent pfx terminal (edges.set index (byte, fresh)) (edgeSet index tree children)
  · exact heap_write_here heap parent _ (heap_read_bound heap parent _ read)
  · apply edges_rep_replace_tree _ edges children index byte old fresh tree
    · apply edges_rep_write_frame heap edges children parent _ descendants
      intro edge member
      exact represented_edge_no_return heap parent (.branch pfx terminal children)
        (.branch pfx terminal edges) edge.1 edge.2 rep read member
    · exact access
    · exact node_rep_write_frame heap fresh parent tree _ replacement noReturn

-- The new temporary token moves into the parent edge while the old edge token
-- moves out for release. No Arc is cloned by this ownership transfer.
theorem edge_token_transfer_owned (heap : NodeHeap) (roots : List NodeId) (parent old fresh : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap (fresh :: roots))
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, old))
    (replacement : NodeRep heap fresh tree) (noReturn : ¬ HeapReach heap fresh parent) :
    HeapOwned (heapWrite heap parent (some (.branch pfx terminal (edges.set index (byte, fresh))))) (old :: roots) := by
  let node := StoredNode.branch pfx terminal (edges.set index (byte, fresh))
  have updated := write_edge_refines heap parent old fresh pfx terminal edges children index byte tree rep read access replacement noReturn
  have elsewhere : ∀ address value, address ≠ parent → heapRead heap address = some value →
      heapRead (heapWrite heap parent (some node)) address = some value := by
    intro address value different original
    rw [heap_write_elsewhere heap parent address (some node) different]
    exact original
  refine ⟨heap_write_modelled heap parent node (.branch pfx terminal (edgeSet index tree children)) owned.modelled updated, ?_, ?_⟩
  · intro root member
    rcases List.mem_cons.mp member with atOld | retained
    · subst root
      have descendants := node_rep_branch_edges heap parent pfx terminal edges children rep read
      obtain ⟨oldTree, _, oldRep⟩ := edges_rep_get heap edges children index byte old descendants access
      exact ⟨oldTree, node_rep_write_frame heap old parent oldTree (some node) oldRep
        (represented_edge_no_return heap parent (.branch pfx terminal children) (.branch pfx terminal edges)
          byte old rep read (List.mem_of_getElem? access))⟩
    · obtain ⟨oldTree, oldRep⟩ := owned.rootsRepresented root (by simp [retained])
      exact node_rep_update_exists heap (heapWrite heap parent (some node)) parent
        (.branch pfx terminal (edgeSet index tree children)) updated elsewhere root oldTree oldRep
  · intro address stored after
    rw [edge_token_transfer_count heap roots parent old fresh pfx terminal edges index byte address read access]
    by_cases same : address = parent
    · subst address
      exact owned.live parent (.branch pfx terminal edges) read
    · rw [heap_write_elsewhere heap parent address (some node) same] at after
      exact owned.live address stored after

theorem heap_reach_write_until (heap : NodeHeap) (root changed address : NodeId) (node : Option StoredNode)
    (reachable : HeapReach heap root address) :
    HeapReach (heapWrite heap changed node) root changed ∨ HeapReach (heapWrite heap changed node) root address := by
  induction reachable with
  | root => exact Or.inr .root
  | child parent address stored byte _ read edge ih =>
      rcases ih with arrived | previous
      · exact Or.inl arrived
      · by_cases same : parent = changed
        · exact Or.inl (same ▸ previous)
        · apply Or.inr
          exact .child parent address stored byte previous
            (by rw [heap_write_elsewhere heap changed parent node same]; exact read) edge

theorem heap_reach_to_rewritten (heap : NodeHeap) (root changed : NodeId) (node : Option StoredNode)
    (reachable : HeapReach heap root changed) : HeapReach (heapWrite heap changed node) root changed :=
  (heap_reach_write_until heap root changed changed node reachable).elim id id

end Kv9.Radix
