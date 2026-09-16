import HeapEdgeDetach

set_option autoImplicit false

namespace Kv9.Radix

theorem make_owned_unique_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus tree) :
    ∃ next address, makeRootUnique heap focus others = some (next, address) ∧ HeapOwned next (address :: others) ∧
      NodeRep next address tree ∧ strongCount next (address :: others) address = 1 ∧ HeapSeparated next others address ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  have represented : RootsModelled heap others := fun saved member => owned.rootsRepresented saved (by simp [member])
  obtain ⟨next, address, completed, focused, separate, preserve⟩ := make_root_unique_refines heap focus others tree rep represented
  exact ⟨next, address, completed, make_root_unique_owned heap next focus address others owned completed, focused,
    make_root_unique_count heap next focus address others tree (heap_modelled_closed heap owned.modelled) rep represented completed,
    separate, fun saved value _ original => preserve saved value original⟩

theorem owned_write_same_children (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (old node : StoredNode) (replacement : Tree) (owned : HeapOwned heap (focus :: others))
    (one : strongCount heap (focus :: others) focus = 1) (read : heapRead heap focus = some old)
    (targets : storedChildren node = storedChildren old)
    (focused : NodeRep (heapWrite heap focus (some node)) focus replacement) :
    HeapOwned (heapWrite heap focus (some node)) (focus :: others) ∧
      strongCount (heapWrite heap focus (some node)) (focus :: others) focus = 1 ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep (heapWrite heap focus (some node)) saved value) := by
  have separate := unique_root_separates heap focus others one
  exact ⟨heap_write_same_targets_owned heap (focus :: others) focus old node replacement owned read targets focused,
    (heap_write_same_targets_count heap (focus :: others) focus old node focus read targets).trans one,
    fun saved value member original => node_rep_write_frame heap saved focus value (some node) original (separate saved member)⟩

def detachOwnedChild (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (index : Nat) :
    Option (NodeHeap × NodeId × UInt8 × NodeId) := do
  let (copied, parent) ← makeRootUnique heap focus others
  let (detached, byte, child) ← detachEdge copied parent index
  some (detached, parent, byte, child)

theorem detach_owned_child_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (index : Nat) (byte : UInt8) (childTree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus (.branch pfx terminal children))
    (access : edgeGet index children = some (byte, childTree)) :
    ∃ next parent child, detachOwnedChild heap focus others index = some (next, parent, byte, child) ∧
      HeapOwned next (child :: parent :: others) ∧ NodeRep next parent (.branch pfx terminal (edgeRemove index children)) ∧
      NodeRep next child childTree ∧ strongCount next (child :: parent :: others) parent = 1 ∧
      HeapSeparated next others parent ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨copied, parent, copiedResult, copiedOwned, copiedRep, parentOne, separate, preserve⟩ :=
    make_owned_unique_refines heap focus others (.branch pfx terminal children) owned rep
  have stored : ∃ edges, heapRead copied parent = some (.branch pfx terminal edges) ∧ EdgesRep copied edges children := by
    cases copiedRep with
    | branch _ _ _ edges _ read descendants => exact ⟨edges, read, descendants⟩
  obtain ⟨edges, read, descendants⟩ := stored
  obtain ⟨child, found, childRep⟩ := edges_rep_get_address copied edges children index byte childTree descendants access
  obtain ⟨detached, nextOwned, nextParent, nextChild, _, keep⟩ :=
    detach_edge_refines copied (parent :: others) parent child pfx terminal edges children index byte childTree copiedOwned copiedRep read found childRep
  let node := StoredNode.branch pfx terminal (storedRemove edges index)
  refine ⟨heapWrite copied parent (some node), parent, child, ?_, nextOwned, nextParent, nextChild, ?_, ?_, ?_⟩
  · simp [detachOwnedChild, copiedResult, detached, node]
  · rw [detach_edge_token_count copied (parent :: others) parent child parent pfx terminal edges index byte read found]
    exact parentOne
  · intro saved member reachable
    exact separate saved member (heap_reach_write_reflects copied saved parent (some node) (separate saved member) parent reachable)
  · intro saved value member original
    exact keep saved value (separate saved member) (preserve saved value member original)

-- After a payload write with unchanged child slots the unique owning token
-- remains unique, so a second make_mut observes the same allocation identity.
theorem make_owned_unique_identity (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (node : StoredNode)
    (read : heapRead heap focus = some node) (one : strongCount heap (focus :: others) focus = 1) :
    makeRootUnique heap focus others = some (heap, focus) := by
  simp [makeRootUnique, read, one]

end Kv9.Radix
