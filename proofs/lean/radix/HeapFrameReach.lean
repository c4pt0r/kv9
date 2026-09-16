import HeapCounts

set_option autoImplicit false

namespace Kv9.Radix

theorem heap_reach_write_reflects (heap : NodeHeap) (root changed : NodeId) (replacement : Option StoredNode)
    (separate : ¬ HeapReach heap root changed) (address : NodeId)
    (reachable : HeapReach (heapWrite heap changed replacement) root address) : HeapReach heap root address := by
  induction reachable with
  | root => exact .root
  | child parent address node byte _ read edge ih =>
      have different : parent ≠ changed := fun same => separate (same ▸ ih)
      rw [heap_write_elsewhere heap changed parent replacement different] at read
      exact .child parent address node byte ih read edge

theorem same_payload_reach (heap : NodeHeap) (old fresh : NodeId)
    (same : heapRead heap fresh = heapRead heap old) (address : NodeId)
    (reachable : HeapReach heap fresh address) : address = fresh ∨ HeapReach heap old address := by
  induction reachable with
  | root => exact Or.inl rfl
  | child parent address node byte _ read edge ih =>
      apply Or.inr
      rcases ih with isFresh | previous
      · rw [isFresh, same] at read
        exact .child old address node byte .root read edge
      · exact .child parent address node byte previous read edge

theorem clone_separates_ancestor (heap : NodeHeap) (focus ancestor : NodeId) (tree : Tree) (node : StoredNode)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node)
    (ancestorBound : ancestor < heap.length) (separate : ¬ HeapReach heap focus ancestor) :
    ¬ HeapReach (heapAlloc heap node) heap.length ancestor := by
  have payload : heapRead (heapAlloc heap node) heap.length = heapRead (heapAlloc heap node) focus := by
    rw [heap_alloc_fresh, heap_alloc_preserves_read heap focus node node read]
  intro reachable
  rcases same_payload_reach (heapAlloc heap node) focus heap.length payload ancestor reachable with fresh | previous
  · rw [fresh] at ancestorBound
    exact Nat.lt_irrefl heap.length ancestorBound
  · exact separate (heap_reach_alloc_reflects heap focus tree node rep ancestor previous)

theorem allocated_child_write_preserves_saved (heap : NodeHeap) (saved : List NodeId)
    (parent : NodeId) (fresh : StoredNode) (replacement : Option StoredNode)
    (represented : ∀ root ∈ saved, ∃ tree, NodeRep heap root tree) (separate : HeapSeparated heap saved parent) :
    HeapSeparated (heapWrite (heapAlloc heap fresh) parent replacement) saved heap.length := by
  intro root member reachable
  obtain ⟨tree, rep⟩ := represented root member
  have parentSeparate : ¬ HeapReach (heapAlloc heap fresh) root parent := fun path =>
    separate root member (heap_reach_alloc_reflects heap root tree fresh rep parent path)
  have before := heap_reach_write_reflects (heapAlloc heap fresh) root parent replacement parentSeparate heap.length reachable
  exact allocation_separates_saved heap saved fresh represented root member before

theorem edges_rep_get (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (address : NodeId) (rep : EdgesRep heap edges children)
    (access : edges[index]? = some (byte, address)) :
    ∃ child, edgeGet index children = some (byte, child) ∧ NodeRep heap address child := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => simp at access
      | cons storedByte storedAddress tail child rest node _ =>
          have same : storedByte = byte ∧ storedAddress = address := by simpa using access
          rcases same with ⟨rfl, rfl⟩
          exact ⟨child, rfl, node⟩
  | succ n ih =>
      cases rep with
      | nil => simp at access
      | cons _ _ tail _ rest _ remaining => exact ih tail rest remaining access

theorem edges_rep_repoint (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (old fresh : NodeId) (child : Tree) (rep : EdgesRep heap edges children)
    (access : edges[index]? = some (byte, old)) (pureAccess : edgeGet index children = some (byte, child))
    (replacement : NodeRep heap fresh child) : EdgesRep heap (edges.set index (byte, fresh)) children := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => simp at access
      | cons storedByte storedAddress tail storedChild rest _ remaining =>
          have same : storedByte = byte ∧ storedAddress = old := by simpa using access
          rcases same with ⟨rfl, rfl⟩
          have childEq : storedChild = child := by simpa [edgeGet] using pureAccess
          subst storedChild
          exact .cons _ fresh tail child rest replacement remaining
  | succ n ih =>
      cases rep with
      | nil => simp at access
      | cons storedByte storedAddress tail storedChild rest node remaining =>
          exact .cons storedByte storedAddress (tail.set n (byte, fresh)) storedChild rest node
            (ih tail rest remaining access pureAccess)

end Kv9.Radix
