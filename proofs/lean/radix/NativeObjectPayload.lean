import NativeObjectUpdate

set_option autoImplicit false

namespace Kv9.Radix

def leafObjectBlocks (arc : ObjectBlock) : ResidentBlockKind → Option ObjectBlock
  | .arcNode => some arc
  | .branchBox => none

def branchObjectBlocks (arc box : ObjectBlock) : ResidentBlockKind → Option ObjectBlock
  | .arcNode => some arc
  | .branchBox => some box

theorem leaf_object_node_ready (layout : NativeObjectLayout) (limit : Nat) (entry : Entry) (arc : ObjectBlock)
    (size : arc.bytes = layout.arcBytes) (within : arc.Within limit) :
    ObjectNodeReady layout (some (.leaf entry)) (leafObjectBlocks arc) limit := by
  refine ⟨?_, ?_, ?_⟩
  · intro kind
    cases kind <;> simp [leafObjectBlocks, nodeObjectBytes, size]
  · intro kind block read
    cases kind with
    | arcNode => have same : arc = block := Option.some.inj read; simpa only [← same] using within
    | branchBox => contradiction
  · intro first second a b different left right
    cases first <;> cases second <;> simp_all [leafObjectBlocks]

theorem branch_object_node_ready (layout : NativeObjectLayout) (limit : Nat) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (arc box : ObjectBlock) (arcSize : arc.bytes = layout.arcBytes)
    (boxSize : box.bytes = layout.branchBytes) (arcWithin : arc.Within limit) (boxWithin : box.Within limit)
    (separate : arc.Disjoint box) :
    ObjectNodeReady layout (some (.branch pfx terminal edges)) (branchObjectBlocks arc box) limit := by
  refine ⟨?_, ?_, ?_⟩
  · intro kind
    cases kind <;> simp [branchObjectBlocks, nodeObjectBytes, arcSize, boxSize]
  · intro kind block read
    cases kind with
    | arcNode => have same : arc = block := Option.some.inj read; simpa only [← same] using arcWithin
    | branchBox => have same : box = block := Option.some.inj read; simpa only [← same] using boxWithin
  · intro first second a b different left right
    cases first <;> cases second
    · exact False.elim (different rfl)
    · have aEq : arc = a := Option.some.inj left
      have bEq : box = b := Option.some.inj right
      simpa only [← aEq, ← bEq] using separate
    · have aEq : box = a := Option.some.inj left
      have bEq : arc = b := Option.some.inj right
      simpa only [← aEq, ← bEq] using object_block_disjoint_symm arc box separate
    · exact False.elim (different rfl)

theorem resident_object_node_ready (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (old node : StoredNode) (objects : NativeObjects heap layout blocks limit)
    (read : heapRead heap focus = some old)
    (sameShape : ∀ kind, nodeObjectBytes layout (some old) kind = nodeObjectBytes layout (some node) kind) :
    ObjectNodeReady layout (some node) (fun kind => blocks ⟨focus, kind⟩) limit := by
  refine ⟨?_, ?_, ?_⟩
  · intro kind
    rw [objects.shape, expected_object_bytes_eq, read, sameShape kind]
  · intro kind block present
    exact objects.within ⟨focus, kind⟩ block present
  · intro first second a b different left right
    exact objects.separate ⟨focus, first⟩ ⟨focus, second⟩ a b
      (fun equal => different (congrArg ResidentBlockId.kind equal)) left right

theorem resident_object_node_fresh (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (objects : NativeObjects heap layout blocks limit) :
    ObjectsFreshAt blocks focus (fun kind => blocks ⟨focus, kind⟩) := by
  intro kind owner a b different left right
  exact objects.separate ⟨focus, kind⟩ owner a b
    (fun equal => different (congrArg ResidentBlockId.node equal).symm) left right

theorem overwrite_resident_object_node (blocks : ObjectBlocks) (focus : NodeId) :
    overwriteObjectNode blocks focus (fun kind => blocks ⟨focus, kind⟩) = blocks := by
  funext owner
  rcases owner with ⟨address, kind⟩
  by_cases same : address = focus <;> simp [overwriteObjectNode, same]

theorem native_objects_same_shape_write (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (old node : StoredNode) (objects : NativeObjects heap layout blocks limit)
    (read : heapRead heap focus = some old)
    (sameShape : ∀ kind, nodeObjectBytes layout (some old) kind = nodeObjectBytes layout (some node) kind) :
    NativeObjects (heapWrite heap focus (some node)) layout blocks limit := by
  have result := native_objects_write heap layout blocks limit focus (some node) (fun kind => blocks ⟨focus, kind⟩)
    objects (heap_read_bound heap focus old read) (resident_object_node_ready heap layout blocks limit focus old node objects read sameShape)
    (resident_object_node_fresh heap layout blocks limit focus objects)
  simpa only [overwrite_resident_object_node] using result

theorem native_objects_leaf_write (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (old fresh : Entry) (objects : NativeObjects heap layout blocks limit)
    (read : heapRead heap focus = some (.leaf old)) : NativeObjects (heapWrite heap focus (some (.leaf fresh))) layout blocks limit :=
  native_objects_same_shape_write heap layout blocks limit focus (.leaf old) (.leaf fresh) objects read
    (by intro kind; cases kind <;> rfl)

theorem native_objects_branch_write (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (oldPfx pfx : Key) (oldTerminal terminal : Option Entry) (oldEdges edges : List StoredEdge)
    (objects : NativeObjects heap layout blocks limit) (read : heapRead heap focus = some (.branch oldPfx oldTerminal oldEdges)) :
    NativeObjects (heapWrite heap focus (some (.branch pfx terminal edges))) layout blocks limit :=
  native_objects_same_shape_write heap layout blocks limit focus (.branch oldPfx oldTerminal oldEdges) (.branch pfx terminal edges)
    objects read (by intro kind; cases kind <;> rfl)

-- Normalization moves the terminal into the Node payload and releases the
-- old Box<Branch>. Its Arc allocation and all other object regions are kept.
theorem native_objects_branch_to_leaf (heap : NodeHeap) (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (focus : NodeId) (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (entry : Entry)
    (objects : NativeObjects heap layout blocks limit) (read : heapRead heap focus = some (.branch pfx terminal edges)) :
    ∃ arc, blocks ⟨focus, .arcNode⟩ = some arc ∧
      NativeObjects (heapWrite heap focus (some (.leaf entry))) layout
        (overwriteObjectNode blocks focus (leafObjectBlocks arc)) limit := by
  have shape := objects.shape ⟨focus, .arcNode⟩
  have size : (blocks ⟨focus, .arcNode⟩).map ObjectBlock.bytes = some layout.arcBytes := by
    simpa only [expectedObjectBytes, read] using shape
  obtain ⟨arc, found, bytes⟩ := Option.map_eq_some_iff.mp size
  refine ⟨arc, found, native_objects_write heap layout blocks limit focus (some (.leaf entry)) (leafObjectBlocks arc)
    objects (heap_read_bound heap focus _ read) (leaf_object_node_ready layout limit entry arc bytes (objects.within _ arc found)) ?_⟩
  intro kind owner a b different left right
  cases kind with
  | branchBox => contradiction
  | arcNode =>
      have same : arc = a := Option.some.inj left
      exact (same ▸ resident_object_node_fresh heap layout blocks limit focus objects .arcNode owner arc b different found right)

theorem native_payload_allocate (heap : NodeHeap) (roots : List NodeId) (layout : NativeObjectLayout) (blocks : ObjectBlocks)
    (node : StoredNode) (tree : Tree) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (objects : NativeObjects heap layout blocks USize.size) (owned : HeapOwned heap ((storedChildren node).map Prod.snd ++ roots))
    (payload : PayloadRep heap node tree) (localReady : ObjectNodeReady layout (some node) localBlocks USize.size)
    (fresh : ObjectsFreshAt blocks heap.length localBlocks) (valid : Valid [] tree) :
    NativeObjects (heapAlloc heap node) layout (overwriteObjectNode blocks heap.length localBlocks) USize.size ∧
      HeapOwned (heapAlloc heap node) (heap.length :: roots) ∧ NodeRep (heapAlloc heap node) heap.length tree ∧
      (entries tree).length < USize.size := by
  have allocated := native_objects_allocate heap layout blocks USize.size node localBlocks objects localReady fresh
  have rep := payload_alloc_rep heap node tree payload
  exact ⟨allocated, payload_allocation_owned heap roots node tree owned payload, rep,
    native_objects_cardinality _ layout _ allocated (some heap.length) (some tree) rep valid⟩

end Kv9.Radix
