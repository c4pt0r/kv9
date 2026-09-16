import HeapInsertResult

set_option autoImplicit false

namespace Kv9.Radix

-- Vec::remove preserves the order of every remaining edge. Its removed Arc
-- moves to an external temporary; removing the slot is not an Arc release.
def storedRemove (edges : List StoredEdge) (index : Nat) : List StoredEdge :=
  edges.take index ++ edges.drop (index + 1)

theorem edges_rep_remove (heap : NodeHeap) (edges : List StoredEdge) (children : Forest) (index : Nat)
    (rep : EdgesRep heap edges children) : EdgesRep heap (storedRemove edges index) (edgeRemove index children) := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => exact .nil
      | cons _ _ _ _ _ _ remaining => exact remaining
  | succ index ih =>
      cases rep with
      | nil => exact .nil
      | cons byte address tail child rest node remaining =>
          exact .cons byte address (storedRemove tail index) child (edgeRemove index rest) node (ih tail rest remaining)

theorem stored_remove_count (edges : List StoredEdge) (index : Nat) (byte : UInt8) (child address : NodeId)
    (access : edges[index]? = some (byte, child)) :
    (edges.map Prod.snd).count address = ((storedRemove edges index).map Prod.snd).count address + referenceHit child address := by
  induction index generalizing edges with
  | zero =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have same : head = (byte, child) := Option.some.inj access
          subst head
          simp [storedRemove, referenceHit, List.count_cons, Nat.add_comm]
  | succ index ih =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          have previous := ih tail access
          simp only [storedRemove, List.take_succ_cons, List.drop_succ_cons, List.cons_append,
            List.map_cons, List.count_cons] at *
          split <;> omega

theorem detach_edge_token_count (heap : NodeHeap) (roots : List NodeId) (parent child address : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, child)) :
    strongCount (heapWrite heap parent (some (.branch pfx terminal (storedRemove edges index)))) (child :: roots) address =
      strongCount heap roots address := by
  have balance := strong_count_write_delta heap roots parent (some (.branch pfx terminal edges))
    (some (.branch pfx terminal (storedRemove edges index))) address (heap_read_cell heap parent _ read)
  have removed := stored_remove_count edges index byte child address access
  simp only [cellTargets, storedChildren] at balance
  rw [strong_count_root_add]
  omega

theorem detach_edge_parent_rep (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (children : Forest) (index : Nat)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) :
    NodeRep (heapWrite heap parent (some (.branch pfx terminal (storedRemove edges index))))
      parent (.branch pfx terminal (edgeRemove index children)) := by
  let replacement := some (StoredNode.branch pfx terminal (storedRemove edges index))
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children rep read
  have kept := edges_rep_write_frame heap edges children parent replacement descendants
    (fun edge member => represented_edge_no_return heap parent (.branch pfx terminal children)
      (.branch pfx terminal edges) edge.1 edge.2 rep read member)
  exact .branch parent pfx terminal (storedRemove edges index) (edgeRemove index children)
    (heap_write_here heap parent replacement (heap_read_bound heap parent _ read))
    (edges_rep_remove _ edges children index kept)

theorem detach_edge_token_owned (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (owned : HeapOwned heap roots)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, child)) :
    HeapOwned (heapWrite heap parent (some (.branch pfx terminal (storedRemove edges index)))) (child :: roots) := by
  let node := StoredNode.branch pfx terminal (storedRemove edges index)
  have updated := detach_edge_parent_rep heap parent pfx terminal edges children index rep read
  refine ⟨heap_write_modelled heap parent node (.branch pfx terminal (edgeRemove index children)) owned.modelled updated, ?_, ?_⟩
  · intro root member
    rcases List.mem_cons.mp member with selected | previous
    · subst root
      obtain ⟨tree, _, focused⟩ := edges_rep_get heap edges children index byte child
        (node_rep_branch_edges heap parent pfx terminal edges children rep read) access
      exact ⟨tree, node_rep_write_frame heap child parent tree (some node) focused
        (represented_edge_no_return heap parent (.branch pfx terminal children) (.branch pfx terminal edges)
          byte child rep read (List.mem_of_getElem? access))⟩
    · obtain ⟨tree, original⟩ := owned.rootsRepresented root previous
      exact node_rep_update_exists heap (heapWrite heap parent (some node)) parent _ updated
        (fun address stored different before => by rw [heap_write_elsewhere heap parent address (some node) different]; exact before)
        root tree original
  · intro address stored after
    rw [detach_edge_token_count heap roots parent child address pfx terminal edges index byte read access]
    by_cases same : address = parent
    · subst address
      exact owned.live parent _ read
    · rw [heap_write_elsewhere heap parent address (some node) same] at after
      exact owned.live address stored after

def detachEdge (heap : NodeHeap) (parent : NodeId) (index : Nat) : Option (NodeHeap × StoredEdge) :=
  match heapRead heap parent with
  | some (.branch pfx terminal edges) => (edges[index]?).map (fun edge =>
      (heapWrite heap parent (some (.branch pfx terminal (storedRemove edges index))), edge))
  | _ => none

theorem detach_edge_refines (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap roots)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, child))
    (focused : NodeRep heap child tree) :
    let next := heapWrite heap parent (some (.branch pfx terminal (storedRemove edges index)))
    detachEdge heap parent index = some (next, byte, child) ∧ HeapOwned next (child :: roots) ∧
      NodeRep next parent (.branch pfx terminal (edgeRemove index children)) ∧ NodeRep next child tree ∧
      heapRead next parent = some (.branch pfx terminal (storedRemove edges index)) ∧
      (∀ saved value, ¬ HeapReach heap saved parent → NodeRep heap saved value → NodeRep next saved value) := by
  exact ⟨by simp [detachEdge, read, access], detach_edge_token_owned heap roots parent child pfx terminal edges children index byte owned rep read access,
    detach_edge_parent_rep heap parent pfx terminal edges children index rep read,
    node_rep_write_frame heap child parent tree _ focused
      (represented_edge_no_return heap parent (.branch pfx terminal children) (.branch pfx terminal edges)
        byte child rep read (List.mem_of_getElem? access)),
    heap_write_here heap parent _ (heap_read_bound heap parent _ read),
    fun saved value separate original => node_rep_write_frame heap saved parent value _ original separate⟩

end Kv9.Radix
