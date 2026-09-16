import HeapEdgeTransfer

set_option autoImplicit false

namespace Kv9.Radix

def assignRoot (heap : NodeHeap) (old fresh : NodeId) (others : List NodeId) : Option NodeHeap :=
  dropLoop heap (fresh :: others) [old]

theorem assign_root_refines (heap : NodeHeap) (old fresh : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (fresh :: old :: others)) (rep : NodeRep heap fresh tree) :
    ∃ next, assignRoot heap old fresh others = some next ∧ HeapOwned next (fresh :: others) ∧
      NodeRep next fresh tree ∧ (∀ root value, root ∈ others → NodeRep heap root value → NodeRep next root value) := by
  have reorder : (fresh :: old :: others).Perm ((fresh :: others) ++ [old]) :=
    List.Perm.cons fresh (List.perm_append_singleton old others).symm
  obtain ⟨next, result, nextOwned, preserve⟩ := drop_loop_owned heap (fresh :: others) [old]
    (heap_owned_permute heap _ _ reorder owned)
  exact ⟨next, result, nextOwned, preserve fresh tree (by simp) rep,
    fun root value member original => preserve root value (by simp [member]) original⟩

def swapEdgeToken (heap : NodeHeap) (parent : NodeId) (index : Nat) (fresh : NodeId) : Option (NodeHeap × NodeId) :=
  match heapRead heap parent with
  | some (.branch pfx terminal edges) => (edges[index]?).map (fun edge =>
      (heapWrite heap parent (some (.branch pfx terminal (edges.set index (edge.1, fresh)))), edge.2))
  | _ => none

def assignEdge (heap : NodeHeap) (roots : List NodeId) (parent : NodeId) (index : Nat) (fresh : NodeId) : Option NodeHeap :=
  (swapEdgeToken heap parent index fresh).bind (fun swapped => dropLoop swapped.1 roots [swapped.2])

theorem assign_edge_refines (heap : NodeHeap) (roots : List NodeId) (parent old fresh root : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap (fresh :: roots))
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, old))
    (replacement : NodeRep heap fresh tree) (noReturn : ¬ HeapReach heap fresh parent)
    (member : root ∈ roots) (reachable : HeapReach heap root parent) :
    ∃ next, assignEdge heap roots parent index fresh = some next ∧ HeapOwned next roots ∧
      NodeRep next parent (.branch pfx terminal (edgeSet index tree children)) ∧
      heapRead next parent = some (.branch pfx terminal (edges.set index (byte, fresh))) ∧
      NodeRep next fresh tree ∧
      (∀ saved value, saved ∈ roots → ¬ HeapReach heap saved parent → NodeRep heap saved value → NodeRep next saved value) := by
  let node := StoredNode.branch pfx terminal (edges.set index (byte, fresh))
  let transferred := heapWrite heap parent (some node)
  have transferredOwned := edge_token_transfer_owned heap roots parent old fresh pfx terminal edges children index byte tree
    owned rep read access replacement noReturn
  have workOwned : HeapOwned transferred (roots ++ [old]) :=
    heap_owned_permute transferred (old :: roots) (roots ++ [old]) (List.perm_append_singleton old roots).symm transferredOwned
  obtain ⟨next, result, nextOwned, preserve⟩ := drop_loop_owned transferred roots [old] workOwned
  have updated := write_edge_refines heap parent old fresh pfx terminal edges children index byte tree rep read access replacement noReturn
  have parentReach := heap_reach_to_rewritten heap root parent (some node) reachable
  have parentRead : heapRead transferred parent = some node := heap_write_here heap parent _ (heap_read_bound heap parent _ read)
  have keptParent := drop_loop_keeps_borrowed transferred next roots [old] workOwned result root parent member
    parentReach (.branch pfx terminal (edgeSet index tree children)) updated
  have keptRead := drop_loop_keeps_borrowed_read transferred next roots [old] workOwned result root parent member parentReach node parentRead
  obtain ⟨bound, _⟩ := List.getElem?_eq_some_iff.mp access
  have childEdge : (byte, fresh) ∈ storedChildren node :=
    List.mem_of_getElem? (List.getElem?_set_self bound)
  have childReach : HeapReach transferred root fresh := .child parent fresh node byte parentReach parentRead childEdge
  have childRep := node_rep_write_frame heap fresh parent tree (some node) replacement noReturn
  have keptChild := drop_loop_keeps_borrowed transferred next roots [old] workOwned result root fresh member childReach tree childRep
  refine ⟨next, ?_, nextOwned, keptParent, keptRead, keptChild, ?_⟩
  · simpa only [assignEdge, swapEdgeToken, read, access, Option.map_some, Option.bind_some] using result
  · intro saved value kept separate original
    exact preserve saved value kept (node_rep_write_frame heap saved parent value (some node) original separate)

end Kv9.Radix
