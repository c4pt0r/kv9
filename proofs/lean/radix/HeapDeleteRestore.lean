import HeapNormalize

set_option autoImplicit false

namespace Kv9.Radix

def attachOwnedChild (heap : NodeHeap) (parent child : NodeId) (others : List NodeId) (index : Nat) (byte : UInt8) :
    Option (NodeHeap × NodeId) := do
  let (copied, address) ← makeRootUnique heap parent (child :: others)
  match heapRead copied address with
  | some (.branch pfx terminal edges) =>
      if index ≤ edges.length then
        some (heapWrite copied address (some (.branch pfx terminal (storedInsert edges index byte child))), address)
      else none
  | _ => none

theorem attach_owned_child_refines (heap : NodeHeap) (parent child : NodeId) (others : List NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (index : Nat) (byte : UInt8) (tree : Tree)
    (owned : HeapOwned heap (child :: parent :: others)) (parentRep : NodeRep heap parent (.branch pfx terminal children))
    (childRep : NodeRep heap child tree) (bound : index ≤ edgeCount children) :
    ∃ next address, attachOwnedChild heap parent child others index byte = some (next, address) ∧
      HeapOwned next (address :: others) ∧ NodeRep next address (.branch pfx terminal (edgeInsert index byte tree children)) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  have reordered := heap_owned_permute heap _ _ (List.Perm.swap parent child others) owned
  obtain ⟨copied, address, completed, copiedOwned, copiedParent, _, separate, preserve⟩ :=
    make_owned_unique_refines heap parent (child :: others) (.branch pfx terminal children) reordered parentRep
  have copiedChild := preserve child tree (by simp) childRep
  have noReturn := separate child (by simp)
  obtain ⟨edges, read, descendants⟩ := represented_branch_read copied address pfx terminal children copiedParent
  have actualBound : index ≤ edges.length := by rw [edges_rep_length copied edges children descendants]; exact bound
  have tokens := heap_owned_permute copied _ _ (List.Perm.swap child address others) copiedOwned
  have nextOwned := insert_edge_token_owned copied (address :: others) address child pfx terminal edges children index byte tree
    tokens copiedParent read copiedChild noReturn
  have nextRep := insert_edge_refines copied address child pfx terminal edges children index byte tree copiedParent read copiedChild noReturn
  refine ⟨heapWrite copied address (some (.branch pfx terminal (storedInsert edges index byte child))), address,
    by simp [attachOwnedChild, completed, read, actualBound], nextOwned, nextRep, ?_⟩
  intro saved value member original
  exact node_rep_write_frame copied saved address value _ (preserve saved value (by simp [member]) original)
    (separate saved (by simp [member]))

-- This contains exactly the fields saved in the Rust frame Vec. Abstract
-- prefixes, entries, and forests occur only in the representation predicate.
structure OwnedDeleteFrame where
  parent : NodeId
  index : Nat
  byte : UInt8

structure OwnedDeleteFrameRep (heap : NodeHeap) (frame : OwnedDeleteFrame) (value : DeleteFrame) : Prop where
  index : frame.index = value.index
  byte : frame.byte = value.byte
  parent : NodeRep heap frame.parent (.branch value.pfx value.terminal value.detached)

def restoreOwnedDelete (heap : NodeHeap) (frame : OwnedDeleteFrame) (others : List NodeId) (root : Option NodeId) :
    Option (NodeHeap × Option NodeId) := do
  let (attached, parent) ← match root with
    | none => some (heap, frame.parent)
    | some child => attachOwnedChild heap frame.parent child others frame.index frame.byte
  normalizeOwned attached parent others

theorem restore_owned_delete_refines (heap : NodeHeap) (frame : OwnedDeleteFrame) (value : DeleteFrame)
    (others : List NodeId) (root : Option NodeId) (replacement : Option Tree)
    (owned : HeapOwned heap (root.toList ++ frame.parent :: others))
    (frameRep : OwnedDeleteFrameRep heap frame value) (rep : RootRep heap root replacement)
    (bound : value.index ≤ edgeCount value.detached) :
    ∃ next address, restoreOwnedDelete heap frame others root = some (next, address) ∧
      OwnedResult heap others (restoreDelete value replacement) next address := by
  cases root with
  | none =>
      cases replacement with
      | some tree => cases rep
      | none =>
          obtain ⟨next, address, completed, result⟩ := normalize_owned_refines heap frame.parent others
            (.branch value.pfx value.terminal value.detached) owned frameRep.parent
          exact ⟨next, address, by simp [restoreOwnedDelete, completed], result⟩
  | some child =>
      cases replacement with
      | none => cases rep
      | some tree =>
          obtain ⟨attached, parent, attachedResult, attachedOwned, attachedRep, keep⟩ := attach_owned_child_refines heap frame.parent child others
            value.pfx value.terminal value.detached frame.index frame.byte tree owned frameRep.parent rep (by simpa [frameRep.index] using bound)
          obtain ⟨next, address, completed, result⟩ := normalize_owned_refines attached parent others
            (.branch value.pfx value.terminal (edgeInsert frame.index frame.byte tree value.detached)) attachedOwned attachedRep
          refine ⟨next, address, by simp [restoreOwnedDelete, attachedResult, completed], ⟨result.owned, ?_, ?_⟩⟩
          · simpa only [restoreDelete, frameRep.index, frameRep.byte] using result.represented
          · intro saved old member original
            exact result.saved saved old member (keep saved old member original)

end Kv9.Radix
