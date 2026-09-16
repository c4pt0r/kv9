import HeapOwnership

set_option autoImplicit false

namespace Kv9.Radix

theorem node_rep_same_payload (heap : NodeHeap) (old fresh : NodeId) (tree : Tree)
    (rep : NodeRep heap old tree) (same : heapRead heap fresh = heapRead heap old) : NodeRep heap fresh tree := by
  cases rep with
  | leaf _ entry read => exact .leaf fresh entry (same.trans read)
  | branch _ pfx terminal edges children read descendants =>
      exact .branch fresh pfx terminal edges children (same.trans read) descendants

theorem clone_node_rep (heap : NodeHeap) (old : NodeId) (tree : Tree) (node : StoredNode)
    (rep : NodeRep heap old tree) (read : heapRead heap old = some node) :
    NodeRep (heapAlloc heap node) heap.length tree := by
  apply node_rep_same_payload (heapAlloc heap node) old heap.length tree (node_rep_alloc heap old tree node rep)
  rw [heap_alloc_fresh, heap_alloc_preserves_read heap old node node read]

theorem heap_reach_alloc_reflects (heap : NodeHeap) (root : NodeId) (tree : Tree) (fresh : StoredNode)
    (rep : NodeRep heap root tree) (address : NodeId) (reachable : HeapReach (heapAlloc heap fresh) root address) :
    HeapReach heap root address := by
  induction reachable with
  | root => exact .root
  | child parent address node byte _ read edge ih =>
      obtain ⟨parentTree, parentRep⟩ := node_rep_reachable heap root tree rep parent ih
      obtain ⟨stored, oldRead⟩ := node_rep_has_cell heap parent parentTree parentRep
      have preserved := heap_alloc_preserves_read heap parent stored fresh oldRead
      have same : stored = node := Option.some.inj (preserved.symm.trans read)
      exact .child parent address stored byte ih oldRead (by simpa only [same] using edge)

theorem allocation_separates_saved (heap : NodeHeap) (saved : List NodeId) (fresh : StoredNode)
    (represented : ∀ root ∈ saved, ∃ tree, NodeRep heap root tree) :
    HeapSeparated (heapAlloc heap fresh) saved heap.length := by
  intro root member reachable
  obtain ⟨tree, rep⟩ := represented root member
  exact node_rep_fresh_separate heap root tree rep
    (heap_reach_alloc_reflects heap root tree fresh rep heap.length reachable)

-- This is the root-slot specialization of the no-Weak make_mut contract.
-- Cloning copies owned payload values and retains the same child identities;
-- arcHandles consequently counts the extra outgoing child references.
def makeRootUnique (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId) :=
  (heapRead heap focus).map (fun node =>
    if strongCount heap (focus :: others) focus = 1 then (heap, focus)
    else (heapAlloc heap node, heap.length))

theorem make_root_unique_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (rep : NodeRep heap focus tree) (represented : ∀ root ∈ others, ∃ old, NodeRep heap root old) :
    ∃ next address, makeRootUnique heap focus others = some (next, address) ∧
      NodeRep next address tree ∧ HeapSeparated next others address ∧
      (∀ root old, NodeRep heap root old → NodeRep next root old) := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
  by_cases unique : strongCount heap (focus :: others) focus = 1
  · exact ⟨heap, focus, by simp [makeRootUnique, read, unique], rep,
      unique_root_separates heap focus others unique, fun _ _ original => original⟩
  · exact ⟨heapAlloc heap node, heap.length, by simp [makeRootUnique, read, unique],
      clone_node_rep heap focus tree node rep read, allocation_separates_saved heap others node represented,
      fun root old original => node_rep_alloc heap root old node original⟩

theorem make_root_unique_no_failure (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (rep : NodeRep heap focus tree) (represented : ∀ root ∈ others, ∃ old, NodeRep heap root old) :
    makeRootUnique heap focus others ≠ none := by
  obtain ⟨next, address, completed, _⟩ := make_root_unique_refines heap focus others tree rep represented
  rw [completed]
  simp

theorem root_cow_write_preserves_saved (heap next : NodeHeap) (focus address : NodeId) (others : List NodeId)
    (tree : Tree) (replacement : Option StoredNode) (rep : NodeRep heap focus tree)
    (represented : ∀ root ∈ others, ∃ old, NodeRep heap root old)
    (completed : makeRootUnique heap focus others = some (next, address))
    (root : NodeId) (old : Tree) (member : root ∈ others) (original : NodeRep heap root old) :
    NodeRep (heapWrite next address replacement) root old := by
  obtain ⟨actualHeap, actualAddress, result, _, separate, preserve⟩ :=
    make_root_unique_refines heap focus others tree rep represented
  have same := Option.some.inj (result.symm.trans completed)
  cases same
  exact separated_write_preserves_saved next others address replacement separate root old member (preserve root old original)

end Kv9.Radix
