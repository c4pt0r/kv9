import NativeCowInterleave

set_option autoImplicit false

namespace Kv9.Radix

-- Assignment moves the old slot token into this call's destructor worklist.
-- The other worklists remain in place, so their releases may interleave with
-- the old payload's teardown. Moving this token is not another Arc clone.
def assignedCopyPool (heap : NodeHeap) (focus : NodeId) (workers : List (List NodeId)) : DropPool :=
  ⟨heap, [focus] :: workers⟩

theorem copied_assignment_references (heap : NodeHeap) (fresh focus : NodeId) (saved : List NodeId) (workers : List (List NodeId)) :
    (fresh :: focus :: saved ++ workers.flatten).Perm (poolReferences (fresh :: saved) (assignedCopyPool heap focus workers)) := by
  simpa only [poolReferences, assignedCopyPool, List.flatten_cons, List.append_assoc, List.singleton_append,
    List.cons_append] using
    List.Perm.cons fresh ((List.perm_append_singleton focus saved).symm.append_right workers.flatten)

theorem native_cow_finish (heap : NodeHeap) (focus fresh : NodeId) (saved : List NodeId)
    (workers : List (List NodeId)) (tree : Tree) (layout : NativeObjectLayout) (blocks : ObjectBlocks)
    (limit count : Nat) (last : DropPool) (owned : HeapOwned heap (fresh :: focus :: saved ++ workers.flatten))
    (rep : NodeRep heap fresh tree) (one : strongCount heap (fresh :: focus :: saved ++ workers.flatten) fresh = 1)
    (objects : NativeObjects heap layout blocks limit)
    (run : PoolHistory (fresh :: saved) count (assignedCopyPool heap focus workers) last) :
    NativeObjects last.heap layout (survivingObjectBlocks last.heap blocks) limit ∧
      HeapOwned last.heap (poolReferences (fresh :: saved) last) ∧ NodeRep last.heap fresh tree ∧
      strongCount last.heap (poolReferences (fresh :: saved) last) fresh = 1 ∧
      HeapSeparated last.heap (saved ++ last.workers.flatten) fresh ∧
      (∀ root value, root ∈ saved → NodeRep heap root value → NodeRep last.heap root value) := by
  have reorder := copied_assignment_references heap fresh focus saved workers
  have input : HeapOwned heap (poolReferences (fresh :: saved) (assignedCopyPool heap focus workers)) :=
    heap_owned_permute heap _ _ reorder owned
  obtain ⟨lastObjects, lastOwned, preserve⟩ := native_objects_pool_history (fresh :: saved) count
    (assignedCopyPool heap focus workers) last layout blocks limit run input objects
  have smaller := pool_history_count_le (fresh :: saved) count (assignedCopyPool heap focus workers) last run fresh
  change strongCount last.heap (poolReferences (fresh :: saved) last) fresh ≤
    strongCount heap (poolReferences (fresh :: saved) (assignedCopyPool heap focus workers)) fresh at smaller
  rw [← strong_count_permute heap _ _ reorder, one] at smaller
  have positive := root_reference_positive last.heap fresh (saved ++ last.workers.flatten)
  have lastOne : strongCount last.heap (poolReferences (fresh :: saved) last) fresh = 1 := by
    change strongCount last.heap (fresh :: (saved ++ last.workers.flatten)) fresh ≤ 1 at smaller
    change strongCount last.heap (fresh :: (saved ++ last.workers.flatten)) fresh = 1
    omega
  exact ⟨lastObjects, lastOwned, preserve fresh tree (by simp) rep, lastOne,
    unique_root_separates last.heap fresh (saved ++ last.workers.flatten) lastOne,
    fun root value member original => preserve root value (by simp [member]) original⟩

-- Compose individually retained child tokens, allocation and arbitrarily
-- interleaved old-token destruction. The current owning token may have become
-- unique long before allocation; no final-time shared-count premise is used.
theorem native_payload_copy_release_prefix (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (state : PayloadCopyState) (valid : PayloadCopyValid focus saved node tree state) (done : state.remaining = [])
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects state.pool.heap layout blocks limit) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (ready : ObjectNodeReady layout (some node) localBlocks limit)
    (fresh : ObjectsFreshAt blocks state.pool.heap.length localBlocks) (count : Nat) (last : DropPool)
    (run : PoolHistory (state.pool.heap.length :: saved) count
      (assignedCopyPool (heapAlloc state.pool.heap node) focus state.pool.workers) last) :
    NativeObjects last.heap layout (survivingObjectBlocks last.heap (overwriteObjectNode blocks state.pool.heap.length localBlocks)) limit ∧
      HeapOwned last.heap (poolReferences (state.pool.heap.length :: saved) last) ∧
      NodeRep last.heap state.pool.heap.length tree ∧
      strongCount last.heap (poolReferences (state.pool.heap.length :: saved) last) state.pool.heap.length = 1 ∧
      HeapSeparated last.heap (saved ++ last.workers.flatten) state.pool.heap.length ∧
      (∀ root value, root ∈ saved → NodeRep state.pool.heap root value → NodeRep last.heap root value) := by
  obtain ⟨allocated, allocatedOwned, allocatedRep, one⟩ :=
    payload_copy_finish_allocate focus saved node tree state valid done layout blocks limit objects localBlocks ready fresh
  obtain ⟨lastObjects, lastOwned, lastRep, lastOne, separate, preserve⟩ :=
    native_cow_finish (heapAlloc state.pool.heap node) focus state.pool.heap.length saved state.pool.workers tree layout _ limit count last
      allocatedOwned allocatedRep one allocated run
  exact ⟨lastObjects, lastOwned, lastRep, lastOne, separate,
    fun root value member original => preserve root value member (node_rep_alloc state.pool.heap root value node original)⟩

-- A return additionally requires the current call's destructor worklist to
-- be empty. Other workers may still have outstanding releases.
theorem native_payload_copy_return (focus : NodeId) (saved : List NodeId) (node : StoredNode) (tree : Tree)
    (state : PayloadCopyState) (valid : PayloadCopyValid focus saved node tree state) (done : state.remaining = [])
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects state.pool.heap layout blocks limit) (localBlocks : ResidentBlockKind → Option ObjectBlock)
    (ready : ObjectNodeReady layout (some node) localBlocks limit)
    (fresh : ObjectsFreshAt blocks state.pool.heap.length localBlocks) (count : Nat)
    (next : NodeHeap) (workers : List (List NodeId))
    (run : PoolHistory (state.pool.heap.length :: saved) count
      (assignedCopyPool (heapAlloc state.pool.heap node) focus state.pool.workers) ⟨next, [] :: workers⟩) :
    NativeObjects next layout (survivingObjectBlocks next (overwriteObjectNode blocks state.pool.heap.length localBlocks)) limit ∧
      HeapOwned next (state.pool.heap.length :: saved ++ workers.flatten) ∧
      NodeRep next state.pool.heap.length tree ∧
      strongCount next (state.pool.heap.length :: saved ++ workers.flatten) state.pool.heap.length = 1 ∧
      HeapSeparated next (saved ++ workers.flatten) state.pool.heap.length ∧
      (∀ root value, root ∈ saved → NodeRep state.pool.heap root value → NodeRep next root value) := by
  simpa only [poolReferences, List.flatten_cons, List.nil_append, List.cons_append] using
    native_payload_copy_release_prefix focus saved node tree state valid done layout blocks limit objects localBlocks ready fresh count
      ⟨next, [] :: workers⟩ run

theorem native_cow_unique_release (focus : NodeId) (saved : List NodeId) (count : Nat) (first last : DropPool)
    (tree : Tree) (owned : HeapOwned first.heap (poolReferences (focus :: saved) first))
    (rep : NodeRep first.heap focus tree) (one : strongCount first.heap (poolReferences (focus :: saved) first) focus = 1)
    (run : PoolHistory (focus :: saved) count first last) :
    NodeRep last.heap focus tree ∧ strongCount last.heap (poolReferences (focus :: saved) last) focus = 1 ∧
      HeapSeparated last.heap (saved ++ last.workers.flatten) focus := by
  obtain ⟨_, preserve⟩ := pool_history_owned (focus :: saved) count first last run owned
  have smaller := pool_history_count_le (focus :: saved) count first last run focus
  rw [one] at smaller
  have positive := root_reference_positive last.heap focus (saved ++ last.workers.flatten)
  have lastOne : strongCount last.heap (poolReferences (focus :: saved) last) focus = 1 := by
    change strongCount last.heap (focus :: (saved ++ last.workers.flatten)) focus ≤ 1 at smaller
    change strongCount last.heap (focus :: (saved ++ last.workers.flatten)) focus = 1
    omega
  exact ⟨preserve focus tree (by simp) rep, lastOne, unique_root_separates last.heap focus (saved ++ last.workers.flatten) lastOne⟩

end Kv9.Radix
