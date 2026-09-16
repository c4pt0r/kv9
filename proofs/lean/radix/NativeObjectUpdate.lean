import NativeObjectHeap

set_option autoImplicit false

namespace Kv9.Radix

def nodeObjectBytes (layout : NativeObjectLayout) : Option StoredNode → ResidentBlockKind → Option Nat
  | none, _ => none
  | some _, .arcNode => some layout.arcBytes
  | some (.leaf _), .branchBox => none
  | some (.branch _ _ _), .branchBox => some layout.branchBytes

theorem expected_object_bytes_eq (heap : NodeHeap) (layout : NativeObjectLayout) (owner : ResidentBlockId) :
    expectedObjectBytes heap layout owner = nodeObjectBytes layout (heapRead heap owner.node) owner.kind := by
  rcases owner with ⟨address, kind⟩
  cases read : heapRead heap address with
  | none => cases kind <;> simp only [expectedObjectBytes, read, nodeObjectBytes]
  | some node => cases node <;> cases kind <;> simp only [expectedObjectBytes, read, nodeObjectBytes]

-- A successful allocator/layout or in-place payload operation supplies these
-- local object blocks. Freshness below is relative to all other live owners;
-- an in-place write may retain this node's existing Arc allocation.
structure ObjectNodeReady (layout : NativeObjectLayout) (node : Option StoredNode)
    (localBlocks : ResidentBlockKind → Option ObjectBlock) (limit : Nat) : Prop where
  shape : ∀ kind, (localBlocks kind).map ObjectBlock.bytes = nodeObjectBytes layout node kind
  within : ∀ kind block, localBlocks kind = some block → block.Within limit
  separate : ∀ first second a b, first ≠ second → localBlocks first = some a → localBlocks second = some b → a.Disjoint b

def overwriteObjectNode (blocks : ObjectBlocks) (focus : NodeId) (localBlocks : ResidentBlockKind → Option ObjectBlock) : ObjectBlocks :=
  fun owner => if owner.node = focus then localBlocks owner.kind else blocks owner

def ObjectsFreshAt (blocks : ObjectBlocks) (focus : NodeId) (localBlocks : ResidentBlockKind → Option ObjectBlock) : Prop :=
  ∀ kind owner a b, owner.node ≠ focus → localBlocks kind = some a → blocks owner = some b → a.Disjoint b

theorem object_block_disjoint_symm (first second : ObjectBlock) (separate : first.Disjoint second) : second.Disjoint first :=
  separate.symm

theorem native_objects_update (heap next : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (node : Option StoredNode) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (objects : NativeObjects heap layout blocks limit) (read : heapRead next focus = node)
    (kept : ∀ address, address ≠ focus → heapRead next address = heapRead heap address)
    (localReady : ObjectNodeReady layout node localBlocks limit) (fresh : ObjectsFreshAt blocks focus localBlocks) :
    NativeObjects next layout (overwriteObjectNode blocks focus localBlocks) limit := by
  refine ⟨objects.layoutValid, ?_, ?_, ?_⟩
  · intro owner
    by_cases same : owner.node = focus
    · rw [overwriteObjectNode, if_pos same, expected_object_bytes_eq, same, read]
      exact localReady.shape owner.kind
    · rw [overwriteObjectNode, if_neg same, objects.shape owner]
      unfold expectedObjectBytes
      rw [kept owner.node same]
  · intro owner block present
    by_cases same : owner.node = focus
    · rw [overwriteObjectNode, if_pos same] at present
      exact localReady.within owner.kind block present
    · rw [overwriteObjectNode, if_neg same] at present
      exact objects.within owner block present
  · intro first second a b different left right
    by_cases firstHere : first.node = focus
    · rw [overwriteObjectNode, if_pos firstHere] at left
      by_cases secondHere : second.node = focus
      · rw [overwriteObjectNode, if_pos secondHere] at right
        have kindsDifferent : first.kind ≠ second.kind := by
          intro equal
          apply different
          cases first
          cases second
          simp_all
        exact localReady.separate first.kind second.kind a b kindsDifferent left right
      · rw [overwriteObjectNode, if_neg secondHere] at right
        exact fresh first.kind second a b secondHere left right
    · rw [overwriteObjectNode, if_neg firstHere] at left
      by_cases secondHere : second.node = focus
      · rw [overwriteObjectNode, if_pos secondHere] at right
        exact object_block_disjoint_symm b a (fresh second.kind first b a firstHere right left)
      · rw [overwriteObjectNode, if_neg secondHere] at right
        exact objects.separate first second a b different left right

theorem heap_alloc_read_other (heap : NodeHeap) (node : StoredNode) (address : NodeId)
    (different : address ≠ heap.length) : heapRead (heapAlloc heap node) address = heapRead heap address := by
  cases before : heapRead heap address with
  | some stored => exact heap_alloc_preserves_read heap address stored node before
  | none =>
      cases after : heapRead (heapAlloc heap node) address with
      | none => rfl
      | some stored =>
          rcases heap_alloc_read_cases heap node address stored after with new | old
          · exact False.elim (different new.1)
          · rw [before] at old
            contradiction

theorem native_objects_allocate (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (node : StoredNode) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (objects : NativeObjects heap layout blocks limit) (localReady : ObjectNodeReady layout (some node) localBlocks limit)
    (fresh : ObjectsFreshAt blocks heap.length localBlocks) :
    NativeObjects (heapAlloc heap node) layout (overwriteObjectNode blocks heap.length localBlocks) limit :=
  native_objects_update heap (heapAlloc heap node) layout blocks limit heap.length (some node) localBlocks objects
    (heap_alloc_fresh heap node) (fun address different => heap_alloc_read_other heap node address different) localReady fresh

theorem native_objects_write (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (node : Option StoredNode) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (objects : NativeObjects heap layout blocks limit) (bound : focus < heap.length)
    (localReady : ObjectNodeReady layout node localBlocks limit) (fresh : ObjectsFreshAt blocks focus localBlocks) :
    NativeObjects (heapWrite heap focus node) layout (overwriteObjectNode blocks focus localBlocks) limit :=
  native_objects_update heap (heapWrite heap focus node) layout blocks limit focus node localBlocks objects
    (heap_write_here heap focus node bound) (fun address different => heap_write_elsewhere heap focus address node different) localReady fresh

theorem empty_object_node_ready (layout : NativeObjectLayout) (limit : Nat) :
    ObjectNodeReady layout none (fun _ => none) limit := by
  refine ⟨?_, ?_, ?_⟩
  · intro kind
    cases kind <;> rfl
  · intro kind block read
    contradiction
  · intro first second a b different left right
    contradiction

theorem native_objects_reclaim (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (objects : NativeObjects heap layout blocks limit) (bound : focus < heap.length) :
    NativeObjects (heapWrite heap focus none) layout (overwriteObjectNode blocks focus (fun _ => none)) limit :=
  native_objects_write heap layout blocks limit focus none (fun _ => none) objects bound (empty_object_node_ready layout limit)
    (by intro kind owner a b different left right; contradiction)

end Kv9.Radix
