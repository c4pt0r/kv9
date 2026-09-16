import HeapInsertReplace

set_option autoImplicit false

namespace Kv9.Radix

-- The indexed Vec::insert contract preserves order and shifts the suffix.
-- The caller must check index <= length; it moves the new child Arc token.
def storedInsert (edges : List StoredEdge) (index : Nat) (byte : UInt8) (child : NodeId) : List StoredEdge :=
  edges.take index ++ (byte, child) :: edges.drop index

theorem edges_rep_length (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (rep : EdgesRep heap edges children) : edges.length = edgeCount children := by
  cases rep with
  | nil => rfl
  | cons _ _ tail _ rest _ remaining =>
      simpa only [List.length_cons, edgeCount] using congrArg (· + 1) (edges_rep_length heap tail rest remaining)

theorem edges_rep_insert (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (child : NodeId) (tree : Tree)
    (rep : EdgesRep heap edges children) (focused : NodeRep heap child tree) :
    EdgesRep heap (storedInsert edges index byte child) (edgeInsert index byte tree children) := by
  induction index generalizing edges children with
  | zero => exact .cons byte child edges tree children focused rep
  | succ index ih =>
      cases rep with
      | nil => exact .cons byte child [] tree .nil focused .nil
      | cons oldByte address tail old rest node remaining =>
          exact .cons oldByte address (storedInsert tail index byte child) old (edgeInsert index byte tree rest)
            node (ih tail rest remaining)

theorem stored_insert_count (edges : List StoredEdge) (index : Nat) (byte : UInt8) (child address : NodeId) :
    ((storedInsert edges index byte child).map Prod.snd).count address =
      (edges.map Prod.snd).count address + referenceHit child address := by
  have partition := congrArg (fun items : List StoredEdge => (items.map Prod.snd).count address) (List.take_append_drop index edges)
  simp only [List.map_append, List.count_append] at partition
  simp only [storedInsert, List.map_append, List.count_append, List.map_cons, List.count_cons]
  by_cases same : child = address
  · subst address
    simp only [beq_self_eq_true, if_true, referenceHit]
    omega
  · have different : (child == address) = false := beq_eq_false_iff_ne.mpr same
    simp only [different, Bool.false_eq_true, if_false, referenceHit, if_neg same, Nat.add_zero]
    exact partition

theorem insert_edge_token_count (heap : NodeHeap) (roots : List NodeId) (parent fresh address : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (index : Nat) (byte : UInt8)
    (read : heapRead heap parent = some (.branch pfx terminal edges)) :
    strongCount (heapWrite heap parent (some (.branch pfx terminal (storedInsert edges index byte fresh)))) roots address =
      strongCount heap (fresh :: roots) address := by
  have balance := strong_count_write_delta heap roots parent (some (.branch pfx terminal edges))
    (some (.branch pfx terminal (storedInsert edges index byte fresh))) address (heap_read_cell heap parent _ read)
  simp only [cellTargets, storedChildren, stored_insert_count] at balance
  rw [strong_count_root_add]
  omega

theorem insert_edge_refines (heap : NodeHeap) (parent fresh : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (children : Forest) (index : Nat) (byte : UInt8) (tree : Tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges))
    (focused : NodeRep heap fresh tree) (noReturn : ¬ HeapReach heap fresh parent) :
    NodeRep (heapWrite heap parent (some (.branch pfx terminal (storedInsert edges index byte fresh))))
      parent (.branch pfx terminal (edgeInsert index byte tree children)) := by
  let replacement := some (StoredNode.branch pfx terminal (storedInsert edges index byte fresh))
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep read
  have kept := edges_rep_write_frame heap edges children parent replacement descendants
    (fun edge member => represented_edge_no_return heap parent (.branch pfx terminal children)
      (.branch pfx terminal edges) edge.1 edge.2 parentRep read member)
  exact .branch parent pfx terminal (storedInsert edges index byte fresh) (edgeInsert index byte tree children)
    (heap_write_here heap parent replacement (heap_read_bound heap parent _ read))
    (edges_rep_insert _ edges children index byte fresh tree kept
      (node_rep_write_frame heap fresh parent tree replacement focused noReturn))

theorem insert_edge_token_owned (heap : NodeHeap) (roots : List NodeId) (parent fresh : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap (fresh :: roots))
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges))
    (focused : NodeRep heap fresh tree) (noReturn : ¬ HeapReach heap fresh parent) :
    HeapOwned (heapWrite heap parent (some (.branch pfx terminal (storedInsert edges index byte fresh)))) roots := by
  let node := StoredNode.branch pfx terminal (storedInsert edges index byte fresh)
  have updated := insert_edge_refines heap parent fresh pfx terminal edges children index byte tree parentRep read focused noReturn
  refine ⟨heap_write_modelled heap parent node (.branch pfx terminal (edgeInsert index byte tree children)) owned.modelled updated, ?_, ?_⟩
  · intro root member
    obtain ⟨value, original⟩ := owned.rootsRepresented root (by simp [member])
    exact node_rep_update_exists heap (heapWrite heap parent (some node)) parent _ updated
      (fun address stored different before => by rw [heap_write_elsewhere heap parent address (some node) different]; exact before)
      root value original
  · intro address stored after
    rw [insert_edge_token_count heap roots parent fresh address pfx terminal edges index byte read]
    by_cases same : address = parent
    · subst address
      exact owned.live parent _ read
    · rw [heap_write_elsewhere heap parent address (some node) same] at after
      exact owned.live address stored after

end Kv9.Radix
