import NativeCowCopy

set_option autoImplicit false

namespace Kv9.Radix

-- Outgoing Arc clones are retained individually. Their reverse accumulation
-- is an ownership inventory, not a change to the source Vec's edge order.
-- Other workers may complete any number of releases between child clones.
structure PayloadCopyState where
  pool : DropPool
  retained : List NodeId
  remaining : List NodeId

def copyHeld (focus : NodeId) (saved : List NodeId) (state : PayloadCopyState) : List NodeId :=
  state.retained ++ focus :: saved

structure PayloadCopyValid (focus : NodeId) (saved : List NodeId) (node : StoredNode)
    (tree : Tree) (state : PayloadCopyState) : Prop where
  owned : HeapOwned state.pool.heap (poolReferences (copyHeld focus saved state) state.pool)
  represented : NodeRep state.pool.heap focus tree
  read : heapRead state.pool.heap focus = some node
  progress : state.retained.reverse ++ state.remaining = (storedChildren node).map Prod.snd

inductive PayloadCopyStep (focus : NodeId) (saved : List NodeId) : PayloadCopyState → PayloadCopyState → Prop where
  | retain (pool : DropPool) (retained : List NodeId) (child : NodeId) (remaining : List NodeId) :
      PayloadCopyStep focus saved ⟨pool, retained, child :: remaining⟩ ⟨pool, child :: retained, remaining⟩
  | release (first next : DropPool) (retained remaining : List NodeId)
      (step : PoolRelease (retained ++ focus :: saved) first next) :
      PayloadCopyStep focus saved ⟨first, retained, remaining⟩ ⟨next, retained, remaining⟩

theorem payload_copy_begin (focus : NodeId) (saved : List NodeId) (pool : DropPool) (node : StoredNode)
    (tree : Tree) (owned : HeapOwned pool.heap (poolReferences (focus :: saved) pool))
    (rep : NodeRep pool.heap focus tree) (read : heapRead pool.heap focus = some node) :
    PayloadCopyValid focus saved node tree ⟨pool, [], (storedChildren node).map Prod.snd⟩ :=
  ⟨owned, rep, read, rfl⟩

theorem payload_copy_step_valid (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (first next : PayloadCopyState) (step : PayloadCopyStep focus saved first next)
    (valid : PayloadCopyValid focus saved node tree first) :
    PayloadCopyValid focus saved node tree next ∧
      (∀ root value, root ∈ saved → NodeRep first.pool.heap root value → NodeRep next.pool.heap root value) := by
  cases step with
  | retain pool retained child remaining =>
      have member : child ∈ (storedChildren node).map Prod.snd := by
        rw [← valid.progress]
        exact List.mem_append_right _ (by simp)
      obtain ⟨edge, edgeMember, target⟩ := List.mem_map.mp member
      obtain ⟨childTree, childRep, _⟩ := node_rep_child_work pool.heap focus tree node edge.1 edge.2
        valid.represented valid.read edgeMember
      have retainedOwned := heap_owned_add_reference pool.heap _ child childTree valid.owned (target ▸ childRep)
      refine ⟨⟨retainedOwned, valid.represented, valid.read, ?_⟩, fun _ _ _ original => original⟩
      simpa only [List.reverse_cons, List.append_assoc, List.singleton_append] using valid.progress
  | release first next retained remaining transition =>
      obtain ⟨nextOwned, preserve⟩ := pool_release_owned (retained ++ focus :: saved) first next transition valid.owned
      have rep := preserve focus tree (by simp) valid.represented
      obtain ⟨stored, read⟩ := node_rep_has_cell next.heap focus tree rep
      have same := Option.some.inj ((pool_release_read_back _ first next transition focus stored read).symm.trans valid.read)
      refine ⟨⟨nextOwned, rep, ?_, valid.progress⟩, ?_⟩
      · simpa only [same] using read
      · intro root value member original
        exact preserve root value (by simp [member]) original

theorem payload_copy_step_objects (focus : NodeId) (saved : List NodeId) (first next : PayloadCopyState)
    (step : PayloadCopyStep focus saved first next) (layout : NativeObjectLayout) (blocks : ObjectBlocks)
    (limit : Nat) (objects : NativeObjects first.pool.heap layout blocks limit) :
    ∃ nextBlocks, NativeObjects next.pool.heap layout nextBlocks limit := by
  cases step with
  | retain => exact ⟨blocks, objects⟩
  | release first next retained remaining transition =>
      exact ⟨survivingObjectBlocks next.heap blocks,
        native_objects_read_back first.heap next.heap layout blocks limit objects
          (pool_release_read_back (retained ++ focus :: saved) first next transition)⟩

inductive PayloadCopyHistory (focus : NodeId) (saved : List NodeId) : Nat → PayloadCopyState → PayloadCopyState → Prop where
  | nil (state : PayloadCopyState) : PayloadCopyHistory focus saved 0 state state
  | step {count : Nat} {first middle last : PayloadCopyState}
      (advance : PayloadCopyStep focus saved first middle) (rest : PayloadCopyHistory focus saved count middle last) :
      PayloadCopyHistory focus saved (count + 1) first last

theorem payload_copy_history_valid (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (count : Nat) (first last : PayloadCopyState) (run : PayloadCopyHistory focus saved count first last)
    (valid : PayloadCopyValid focus saved node tree first) : PayloadCopyValid focus saved node tree last ∧
      (∀ root value, root ∈ saved → NodeRep first.pool.heap root value → NodeRep last.pool.heap root value) := by
  induction run with
  | nil => exact ⟨valid, fun _ _ _ original => original⟩
  | step advance rest ih =>
      obtain ⟨middleValid, preserve⟩ := payload_copy_step_valid focus saved node tree _ _ advance valid
      obtain ⟨lastValid, finalPreserve⟩ := ih middleValid
      exact ⟨lastValid, fun root value member original => finalPreserve root value member (preserve root value member original)⟩

theorem payload_copy_history_objects (focus : NodeId) (saved : List NodeId)
    (count : Nat) (first last : PayloadCopyState) (run : PayloadCopyHistory focus saved count first last)
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects first.pool.heap layout blocks limit) : ∃ lastBlocks, NativeObjects last.pool.heap layout lastBlocks limit := by
  induction run generalizing blocks with
  | nil => exact ⟨blocks, objects⟩
  | step advance rest ih =>
      obtain ⟨middleBlocks, middleObjects⟩ := payload_copy_step_objects focus saved _ _ advance layout blocks limit objects
      exact ih middleBlocks middleObjects

theorem node_rep_payload (heap : NodeHeap) (focus : NodeId) (node : StoredNode) (tree : Tree)
    (rep : NodeRep heap focus tree) (read : heapRead heap focus = some node) : PayloadRep heap node tree := by
  cases rep with
  | leaf _ entry access =>
      have same := Option.some.inj (access.symm.trans read)
      exact same ▸ PayloadRep.leaf entry
  | branch _ pfx terminal edges children access descendants =>
      have same := Option.some.inj (access.symm.trans read)
      exact same ▸ PayloadRep.branch pfx terminal edges children descendants

theorem payload_copy_finished_owned (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (state : PayloadCopyState) (valid : PayloadCopyValid focus saved node tree state) (done : state.remaining = []) :
    HeapOwned state.pool.heap ((storedChildren node).map Prod.snd ++ (focus :: saved ++ state.pool.workers.flatten)) := by
  have complete : state.retained.reverse = (storedChildren node).map Prod.snd := by
    simpa only [done, List.append_nil] using valid.progress
  have reorder := (List.reverse_perm state.retained).symm.append_right (focus :: saved ++ state.pool.workers.flatten)
  rw [complete] at reorder
  exact heap_owned_permute state.pool.heap _ _ reorder
    (by simpa only [poolReferences, copyHeld, List.append_assoc] using valid.owned)

-- Successful allocation moves the individually retained child tokens into
-- the new payload, without cloning them a second time. The old owning token
-- is still held. Native buffers and allocation success remain explicit inputs.
theorem payload_copy_finish_allocate (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (state : PayloadCopyState) (valid : PayloadCopyValid focus saved node tree state) (done : state.remaining = [])
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects state.pool.heap layout blocks limit) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (ready : ObjectNodeReady layout (some node) localBlocks limit)
    (fresh : ObjectsFreshAt blocks state.pool.heap.length localBlocks) :
    NativeObjects (heapAlloc state.pool.heap node) layout (overwriteObjectNode blocks state.pool.heap.length localBlocks) limit ∧
      HeapOwned (heapAlloc state.pool.heap node) (state.pool.heap.length :: focus :: saved ++ state.pool.workers.flatten) ∧
      NodeRep (heapAlloc state.pool.heap node) state.pool.heap.length tree ∧
      strongCount (heapAlloc state.pool.heap node) (state.pool.heap.length :: focus :: saved ++ state.pool.workers.flatten) state.pool.heap.length = 1 := by
  have payload := node_rep_payload state.pool.heap focus node tree valid.represented valid.read
  have readyOwned := payload_copy_finished_owned focus saved node tree state valid done
  exact ⟨native_objects_allocate state.pool.heap layout blocks limit node localBlocks objects ready fresh,
    payload_allocation_owned state.pool.heap (focus :: saved ++ state.pool.workers.flatten) node tree readyOwned payload,
    payload_alloc_rep state.pool.heap node tree payload,
    payload_allocation_unique state.pool.heap (focus :: saved ++ state.pool.workers.flatten) node readyOwned⟩

end Kv9.Radix
