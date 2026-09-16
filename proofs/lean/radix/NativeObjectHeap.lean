import NativeEntryStorage

set_option autoImplicit false

namespace Kv9.Radix

abbrev ObjectBlocks := ResidentBlockId → Option ObjectBlock

-- Field offsets and object extents belong to the pinned compiler/layout
-- contract. In particular, Entry is nonzero-sized even when both Vecs are empty.
structure NativeObjectLayout where
  arcBytes : Nat
  branchBytes : Nat
  entryBytes : Nat
  leafEntryOffset : Nat
  branchEntryOffset : Nat

def NativeObjectLayout.Valid (layout : NativeObjectLayout) : Prop :=
  0 < layout.entryBytes ∧ layout.leafEntryOffset + layout.entryBytes ≤ layout.arcBytes ∧
    layout.branchEntryOffset + layout.entryBytes ≤ layout.branchBytes

def NativeObjectLayout.entryOffset (layout : NativeObjectLayout) : StoredNode → Nat
  | .leaf _ => layout.leafEntryOffset
  | .branch _ _ _ => layout.branchEntryOffset

def expectedObjectBytes (heap : NodeHeap) (layout : NativeObjectLayout) (owner : ResidentBlockId) : Option Nat :=
  match heapRead heap owner.node, owner.kind with
  | none, _ => none
  | some _, .arcNode => some layout.arcBytes
  | some (.leaf _), .branchBox => none
  | some (.branch _ _ _), .branchBox => some layout.branchBytes

-- Exact occupancy distinguishes the Arc<Node> payload from a separately owned
-- Box<Branch>. Shared Arc edges refer to the same owner identity; they do not
-- create duplicate Box ownership. Byte-buffer storage is a separate relation.
structure NativeObjects (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat) : Prop where
  layoutValid : layout.Valid
  shape : ∀ owner, (blocks owner).map ObjectBlock.bytes = expectedObjectBytes heap layout owner
  within : ∀ owner block, blocks owner = some block → block.Within limit
  separate : ∀ first second a b, first ≠ second → blocks first = some a → blocks second = some b → a.Disjoint b

def objectEntryImage (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) : EntryStorageImage where
  blocks := blocks
  entryBytes := layout.entryBytes
  entryOffset := fun address => match heapRead heap address with
    | none => 0
    | some node => layout.entryOffset node

theorem native_objects_empty (layout : NativeObjectLayout) (limit : Nat) (valid : layout.Valid) :
    NativeObjects [] layout (fun _ => none) limit := by
  refine ⟨valid, ?_, ?_, ?_⟩
  · intro owner
    cases owner.kind <;> rfl
  · intro owner block read
    contradiction
  · intro first second a b different left right
    contradiction

theorem native_objects_entry_storage (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects heap layout blocks limit) : EntryStorageRep heap (objectEntryImage heap layout blocks) limit := by
  refine ⟨objects.layoutValid.1, objects.within, objects.separate, ?_⟩
  intro address node entry read payload
  have shape := objects.shape ⟨address, residentEntryOwner node⟩
  cases node with
  | leaf value =>
      have size : (blocks ⟨address, .arcNode⟩).map ObjectBlock.bytes = some layout.arcBytes := by
        simpa only [expectedObjectBytes, residentEntryOwner, read] using shape
      obtain ⟨block, existsBlock, bytes⟩ := Option.map_eq_some_iff.mp size
      exact ⟨block, existsBlock, by simpa only [objectEntryImage, read, NativeObjectLayout.entryOffset, bytes] using objects.layoutValid.2.1⟩
  | branch pfx terminal edges =>
      have size : (blocks ⟨address, .branchBox⟩).map ObjectBlock.bytes = some layout.branchBytes := by
        simpa only [expectedObjectBytes, residentEntryOwner, read] using shape
      obtain ⟨block, existsBlock, bytes⟩ := Option.map_eq_some_iff.mp size
      exact ⟨block, existsBlock, by simpa only [objectEntryImage, read, NativeObjectLayout.entryOffset, bytes] using objects.layoutValid.2.2⟩

theorem native_objects_cardinality (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks)
    (objects : NativeObjects heap layout blocks USize.size) (root : Option NodeId) (tree : Option Tree)
    (rep : RootRep heap root tree) (valid : ValidOptional [] tree) : (optionalEntries tree).length < USize.size :=
  native_resident_cardinality heap (objectEntryImage heap layout blocks) USize.size
    (native_objects_entry_storage heap layout blocks USize.size objects) USize.size_pos root tree rep valid

theorem native_object_owner_live (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects heap layout blocks limit) (owner : ResidentBlockId) (block : ObjectBlock)
    (present : blocks owner = some block) : ∃ node, heapRead heap owner.node = some node := by
  have shape := objects.shape owner
  cases read : heapRead heap owner.node with
  | none => cases owner.kind <;> simp [present, expectedObjectBytes, read] at shape
  | some node => exact ⟨node, rfl⟩

end Kv9.Radix
