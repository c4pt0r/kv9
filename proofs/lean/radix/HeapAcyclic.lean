import HeapClone

set_option autoImplicit false

namespace Kv9.Radix

theorem edges_rep_member_work (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (rep : EdgesRep heap edges children) (edge : StoredEdge) (member : edge ∈ edges) :
    ∃ tree, NodeRep heap edge.2 tree ∧ treeWork tree ≤ forestWork children := by
  cases rep with
  | nil => simp at member
  | cons byte address tail child rest node remaining =>
      rcases List.mem_cons.mp member with same | later
      · subst edge
        exact ⟨child, node, Nat.le_add_right _ _⟩
      · obtain ⟨tree, childRep, bound⟩ := edges_rep_member_work heap tail rest remaining edge later
        exact ⟨tree, childRep, Nat.le_trans bound (Nat.le_add_left _ _)⟩

theorem node_rep_child_work (heap : NodeHeap) (parent : NodeId) (tree : Tree) (node : StoredNode)
    (byte : UInt8) (child : NodeId) (rep : NodeRep heap parent tree)
    (read : heapRead heap parent = some node) (edge : (byte, child) ∈ storedChildren node) :
    ∃ childTree, NodeRep heap child childTree ∧ treeWork childTree < treeWork tree := by
  cases rep with
  | leaf _ entry readLeaf =>
      have same : node = .leaf entry := Option.some.inj (read.symm.trans readLeaf)
      simp [same, storedChildren] at edge
  | branch _ pfx terminal edges children readBranch descendants =>
      have same : node = .branch pfx terminal edges := Option.some.inj (read.symm.trans readBranch)
      obtain ⟨childTree, childRep, bound⟩ := edges_rep_member_work heap edges children descendants (byte, child)
        (by simpa only [same, storedChildren] using edge)
      refine ⟨childTree, childRep, ?_⟩
      simp only [treeWork]
      omega

theorem node_rep_reach_work (heap : NodeHeap) (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree)
    (address : NodeId) (reachable : HeapReach heap root address) :
    ∃ child, NodeRep heap address child ∧ treeWork child ≤ treeWork tree := by
  induction reachable with
  | root => exact ⟨tree, rep, Nat.le_refl _⟩
  | child parent address node byte _ read edge ih =>
      obtain ⟨parentTree, parentRep, bound⟩ := ih
      obtain ⟨childTree, childRep, smaller⟩ := node_rep_child_work heap parent parentTree node byte address parentRep read edge
      exact ⟨childTree, childRep, Nat.le_trans (Nat.le_of_lt smaller) bound⟩

theorem represented_edge_no_return (heap : NodeHeap) (parent : NodeId) (tree : Tree) (node : StoredNode)
    (byte : UInt8) (child : NodeId) (rep : NodeRep heap parent tree)
    (read : heapRead heap parent = some node) (edge : (byte, child) ∈ storedChildren node) :
    ¬ HeapReach heap child parent := by
  obtain ⟨childTree, childRep, smaller⟩ := node_rep_child_work heap parent tree node byte child rep read edge
  intro reachable
  obtain ⟨returned, returnedRep, bound⟩ := node_rep_reach_work heap child childTree childRep parent reachable
  have same := node_rep_unique heap parent returned tree returnedRep rep
  rw [same] at bound
  exact Nat.not_le_of_gt smaller bound

theorem edges_rep_alloc (heap : NodeHeap) (edges : List StoredEdge) (children : Forest) (fresh : StoredNode)
    (rep : EdgesRep heap edges children) : EdgesRep (heapAlloc heap fresh) edges children := by
  cases rep with
  | nil => exact .nil
  | cons byte address tail child rest node remaining =>
      exact .cons byte address tail child rest (node_rep_alloc heap address child fresh node)
        (edges_rep_alloc heap tail rest fresh remaining)

theorem edges_rep_write_frame (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (changed : NodeId) (replacement : Option StoredNode) (rep : EdgesRep heap edges children)
    (separate : ∀ edge ∈ edges, ¬ HeapReach heap edge.2 changed) :
    EdgesRep (heapWrite heap changed replacement) edges children := by
  cases rep with
  | nil => exact .nil
  | cons byte address tail child rest node remaining =>
      exact .cons byte address tail child rest
        (node_rep_write_frame heap address changed child replacement node (separate (byte, address) (by simp)))
        (edges_rep_write_frame heap tail rest changed replacement remaining
          (fun edge member => separate edge (by simp [member])))

theorem node_rep_alloc_write_frame (heap : NodeHeap) (root changed : NodeId) (tree : Tree)
    (fresh : StoredNode) (replacement : Option StoredNode) (rep : NodeRep heap root tree)
    (separate : ¬ HeapReach heap root changed) :
    NodeRep (heapWrite (heapAlloc heap fresh) changed replacement) root tree := by
  exact node_rep_write_frame (heapAlloc heap fresh) root changed tree replacement
    (node_rep_alloc heap root tree fresh rep)
    (fun reachable => separate (heap_reach_alloc_reflects heap root tree fresh rep changed reachable))

end Kv9.Radix
