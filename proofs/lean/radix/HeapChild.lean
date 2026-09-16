import HeapFrameReach

set_option autoImplicit false

namespace Kv9.Radix

theorem node_rep_branch_edges (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (children : Forest) (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) : EdgesRep heap edges children := by
  cases rep with
  | branch _ _ _ stored _ original descendants =>
      have same : stored = edges := (StoredNode.branch.inj (Option.some.inj (original.symm.trans read))).2.2
      simpa only [same] using descendants

def cloneChildAt (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (index : Nat) (byte : UInt8) (node : StoredNode) : NodeHeap :=
  heapWrite (heapAlloc heap node) parent (some (.branch pfx terminal (edges.set index (byte, heap.length))))

theorem clone_child_at_refines (heap : NodeHeap) (saved : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (node : StoredNode) (childTree : Tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (pureAccess : edgeGet index children = some (byte, childTree))
    (childRep : NodeRep heap child childTree) (childRead : heapRead heap child = some node)
    (represented : ∀ root ∈ saved, ∃ tree, NodeRep heap root tree) (parentSeparate : HeapSeparated heap saved parent) :
    let next := cloneChildAt heap parent pfx terminal edges index byte node
    NodeRep next parent (.branch pfx terminal children) ∧ NodeRep next heap.length childTree ∧
      HeapSeparated next saved heap.length ∧
      (∀ root tree, root ∈ saved → NodeRep heap root tree → NodeRep next root tree) := by
  let replacement := some (StoredNode.branch pfx terminal (edges.set index (byte, heap.length)))
  have parentBound := heap_read_bound heap parent (.branch pfx terminal edges) parentRead
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep parentRead
  have edgeMember : (byte, child) ∈ edges := List.mem_of_getElem? access
  have childSeparate := represented_edge_no_return heap parent (.branch pfx terminal children)
    (.branch pfx terminal edges) byte child parentRep parentRead edgeMember
  have cloned : NodeRep (heapWrite (heapAlloc heap node) parent replacement) heap.length childTree :=
    node_rep_write_frame (heapAlloc heap node) heap.length parent childTree replacement
      (clone_node_rep heap child childTree node childRep childRead)
      (clone_separates_ancestor heap child parent childTree node childRep childRead parentBound childSeparate)
  have kept : EdgesRep (heapWrite (heapAlloc heap node) parent replacement) edges children := by
    apply edges_rep_write_frame (heapAlloc heap node) edges children parent replacement
      (edges_rep_alloc heap edges children node descendants)
    intro edge member reachable
    obtain ⟨subtree, subRep⟩ := edges_rep_member heap edges children descendants edge member
    exact represented_edge_no_return heap parent (.branch pfx terminal children)
      (.branch pfx terminal edges) edge.1 edge.2 parentRep parentRead member
      (heap_reach_alloc_reflects heap edge.2 subtree node subRep parent reachable)
  have updated : NodeRep (heapWrite (heapAlloc heap node) parent replacement) parent (.branch pfx terminal children) := by
    apply NodeRep.branch parent pfx terminal (edges.set index (byte, heap.length)) children
    · apply heap_write_here
      exact Nat.lt_of_lt_of_le parentBound (by simp [heapAlloc])
    · exact edges_rep_repoint (heapWrite (heapAlloc heap node) parent replacement) edges children index byte child
        heap.length childTree kept access pureAccess cloned
  exact ⟨updated, cloned, allocated_child_write_preserves_saved heap saved parent node replacement represented parentSeparate,
    fun root tree member original => node_rep_alloc_write_frame heap root parent tree node replacement original
      (parentSeparate root member)⟩

-- The parent-unique guard checks the ghost precondition for obtaining the
-- nested mutable Arc slot. Rust's prior make_mut and borrow enforce it; this
-- is not an added runtime check or a claim of compiler-verified borrowing.
def makeChildUnique (heap : NodeHeap) (roots : List NodeId) (parent : NodeId) (index : Nat) : Option (NodeHeap × NodeId) :=
  if strongCount heap roots parent = 1 then
    match heapRead heap parent with
    | some (.branch pfx terminal edges) =>
        match edges[index]? with
        | none => none
        | some (byte, child) =>
            (heapRead heap child).map (fun node =>
              if strongCount heap roots child = 1 then (heap, child)
              else (cloneChildAt heap parent pfx terminal edges index byte node, heap.length))
    | _ => none
  else none

theorem make_child_unique_refines (heap : NodeHeap) (roots saved : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (parentOne : strongCount heap roots parent = 1)
    (included : ∀ root ∈ saved, root ∈ roots) (represented : ∀ root ∈ saved, ∃ tree, NodeRep heap root tree)
    (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, child)) (parentSeparate : HeapSeparated heap saved parent) :
    ∃ next address childTree, makeChildUnique heap roots parent index = some (next, address) ∧
      edgeGet index children = some (byte, childTree) ∧ NodeRep next parent (.branch pfx terminal children) ∧
      NodeRep next address childTree ∧ HeapSeparated next saved address ∧
      (∀ root tree, root ∈ saved → NodeRep heap root tree → NodeRep next root tree) := by
  have descendants := node_rep_branch_edges heap parent pfx terminal edges children parentRep parentRead
  obtain ⟨childTree, pureAccess, childRep⟩ := edges_rep_get heap edges children index byte child descendants access
  obtain ⟨node, childRead⟩ := node_rep_has_cell heap child childTree childRep
  by_cases unique : strongCount heap roots child = 1
  · refine ⟨heap, child, childTree, ?_, pureAccess, parentRep, childRep, ?_, fun _ _ _ original => original⟩
    · simp [makeChildUnique, parentOne, parentRead, access, childRead, unique]
    · apply unique_child_separates heap roots saved parent child index included parentSeparate
      · simp only [arcSlotRead, parentRead, Option.bind_some, storedChildren, access, Option.map_some]
      · exact unique
  · obtain ⟨newParent, newChild, separate, preserve⟩ := clone_child_at_refines heap saved parent child pfx terminal
      edges children index byte node childTree parentRep parentRead access pureAccess childRep childRead represented parentSeparate
    refine ⟨cloneChildAt heap parent pfx terminal edges index byte node, heap.length, childTree, ?_,
      pureAccess, newParent, newChild, separate, preserve⟩
    simp [makeChildUnique, parentOne, parentRead, access, childRead, unique]

end Kv9.Radix
