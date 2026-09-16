import HeapNormalizePrefix

set_option autoImplicit false

namespace Kv9.Radix

-- Normalization uses Vec::pop, unlike the indexed removal in deletion descent.
def popOwnedChild (heap : NodeHeap) (focus : NodeId) (others : List NodeId) :
    Option (NodeHeap × NodeId × UInt8 × NodeId) := do
  let (copied, parent) ← makeRootUnique heap focus others
  match heapRead copied parent with
  | some (.branch pfx terminal edges) =>
      let (byte, child) ← edges.getLast?
      some (heapWrite copied parent (some (.branch pfx terminal edges.dropLast)), parent, byte, child)
  | _ => none

theorem pop_owned_singleton_eq (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (terminal : Option Entry) (byte : UInt8) (tree : Tree)
    (owned : HeapOwned heap (focus :: others))
    (rep : NodeRep heap focus (.branch pfx terminal (.cons byte tree .nil))) :
    popOwnedChild heap focus others = detachOwnedChild heap focus others 0 := by
  obtain ⟨copied, parent, completed, _, focused, _⟩ :=
    make_owned_unique_refines heap focus others (.branch pfx terminal (.cons byte tree .nil)) owned rep
  obtain ⟨edges, read, descendants⟩ := represented_branch_read copied parent pfx terminal (.cons byte tree .nil) focused
  cases descendants with
  | cons _ child _ _ _ _ tail =>
      cases tail
      simp [popOwnedChild, detachOwnedChild, detachEdge, completed, read, storedRemove]

def promoteDetachedChild (heap : NodeHeap) (parent child : NodeId) (others : List NodeId) (byte : UInt8) :
    Option (NodeHeap × NodeId) := do
  let (merged, address) ← match heapRead heap child with
    | some (.leaf _) => some (heap, child)
    | some (.branch _ _ _) => mergeDetachedPrefix heap parent child others byte
    | none => none
  let released ← assignRoot merged parent address others
  some (released, address)

theorem promote_detached_child_refines (heap : NodeHeap) (parent child : NodeId) (others : List NodeId)
    (pfx : Key) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap (child :: parent :: others))
    (parentRep : NodeRep heap parent (.branch pfx none .nil)) (childRep : NodeRep heap child tree)
    (parentOne : strongCount heap (child :: parent :: others) parent = 1) :
    ∃ next address, promoteDetachedChild heap parent child others byte = some (next, address) ∧
      OwnedResult heap others (normalize (.branch pfx none (.cons byte tree .nil))) next (some address) := by
  cases tree with
  | leaf entry =>
      have read : heapRead heap child = some (.leaf entry) := by cases childRep; assumption
      obtain ⟨next, completed, nextOwned, nextRep, preserve⟩ := assign_root_refines heap parent child others (.leaf entry) owned childRep
      exact ⟨next, child, by simp [promoteDetachedChild, read, completed], ⟨nextOwned, nextRep, preserve⟩⟩
  | branch childPfx terminal children =>
      obtain ⟨edges, read, _⟩ := represented_branch_read heap child childPfx terminal children childRep
      obtain ⟨merged, address, mergeResult, mergedOwned, mergedRep, _, keep⟩ :=
        merge_detached_prefix_refines heap parent child others pfx childPfx terminal children byte owned parentRep childRep parentOne
      obtain ⟨next, completed, nextOwned, nextRep, preserve⟩ := assign_root_refines merged parent address others
        (.branch (pfx ++ byte :: childPfx) terminal children) mergedOwned mergedRep
      exact ⟨next, address, by simp [promoteDetachedChild, read, mergeResult, completed],
        ⟨nextOwned, nextRep, fun saved value member original => preserve saved value member (keep saved value member original)⟩⟩

def singletonOwnedChild (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId) := do
  let (detached, parent, byte, child) ← popOwnedChild heap focus others
  promoteDetachedChild detached parent child others byte

theorem singleton_owned_child_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (byte : UInt8) (tree : Tree) (owned : HeapOwned heap (focus :: others))
    (rep : NodeRep heap focus (.branch pfx none (.cons byte tree .nil))) :
    ∃ next address, singletonOwnedChild heap focus others = some (next, address) ∧
      OwnedResult heap others (normalize (.branch pfx none (.cons byte tree .nil))) next (some address) := by
  have pop := pop_owned_singleton_eq heap focus others pfx none byte tree owned rep
  obtain ⟨detached, parent, child, completed, nextOwned, parentRep, childRep, parentOne, _, keep⟩ :=
    detach_owned_child_refines heap focus others pfx none (.cons byte tree .nil) 0 byte tree owned rep rfl
  obtain ⟨next, address, promoted, result⟩ := promote_detached_child_refines detached parent child others pfx byte tree
    nextOwned parentRep childRep parentOne
  exact ⟨next, address, by simp [singletonOwnedChild, pop, completed, promoted],
    ⟨result.owned, result.represented,
      fun saved value member original => result.saved saved value member (keep saved value member original)⟩⟩

end Kv9.Radix
