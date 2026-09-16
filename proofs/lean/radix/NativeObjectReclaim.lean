import NativeObjectPayload

set_option autoImplicit false

namespace Kv9.Radix

-- Reclamation removes object regions only when the corresponding graph cell
-- is absent. Surviving payloads keep their original addresses and extents.
def survivingObjectBlocks (next : NodeHeap) (blocks : ObjectBlocks) : ObjectBlocks :=
  fun owner => match heapRead next owner.node with
    | none => none
    | some _ => blocks owner

theorem native_objects_read_back (heap next : NodeHeap) (layout : NativeObjectLayout)
    (blocks : ObjectBlocks) (limit : Nat) (objects : NativeObjects heap layout blocks limit)
    (back : ∀ address node, heapRead next address = some node → heapRead heap address = some node) :
    NativeObjects next layout (survivingObjectBlocks next blocks) limit := by
  refine ⟨objects.layoutValid, ?_, ?_, ?_⟩
  · intro owner
    cases read : heapRead next owner.node with
    | none => cases owner.kind <;> simp [survivingObjectBlocks, expectedObjectBytes, read]
    | some node =>
        have before := back owner.node node read
        simpa only [survivingObjectBlocks, read, expected_object_bytes_eq, before] using objects.shape owner
  · intro owner block present
    cases read : heapRead next owner.node with
    | none => simp [survivingObjectBlocks, read] at present
    | some node => exact objects.within owner block (by simpa only [survivingObjectBlocks, read] using present)
  · intro first second a b different left right
    cases leftRead : heapRead next first.node with
    | none => simp [survivingObjectBlocks, leftRead] at left
    | some firstNode =>
        cases rightRead : heapRead next second.node with
        | none => simp [survivingObjectBlocks, rightRead] at right
        | some secondNode =>
            exact objects.separate first second a b different
              (by simpa only [survivingObjectBlocks, leftRead] using left)
              (by simpa only [survivingObjectBlocks, rightRead] using right)

theorem native_objects_drop_loop (heap next : NodeHeap) (held pending : List NodeId)
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (objects : NativeObjects heap layout blocks limit) (owned : HeapOwned heap (held ++ pending))
    (completed : dropLoop heap held pending = some next) :
    NativeObjects next layout (survivingObjectBlocks next blocks) limit ∧ HeapOwned next held ∧
      (∀ root tree, root ∈ held → NodeRep heap root tree → NodeRep next root tree) := by
  obtain ⟨actual, result, nextOwned, preserve⟩ := drop_loop_owned heap held pending owned
  have same := Option.some.inj (result.symm.trans completed)
  subst actual
  exact ⟨native_objects_read_back heap next layout blocks limit objects
    (drop_loop_read_back heap next held pending completed), nextOwned, preserve⟩

theorem pool_release_read_back (held : List NodeId) (first next : DropPool)
    (step : PoolRelease held first next) (address : NodeId) (node : StoredNode)
    (read : heapRead next.heap address = some node) : heapRead first.heap address = some node := by
  cases step with
  | worker heap next before after pending work transition =>
      exact drop_step_read_back heap next ((held ++ before.flatten) ++ after.flatten) pending work transition address node read

theorem pool_history_read_back (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) (address : NodeId) (node : StoredNode)
    (read : heapRead last.heap address = some node) : heapRead first.heap address = some node := by
  induction run with
  | nil => exact read
  | step advance rest ih => exact pool_release_read_back held _ _ advance address node (ih read)

theorem native_objects_pool_history (held : List NodeId) (count : Nat) (first last : DropPool)
    (layout : NativeObjectLayout) (blocks : ObjectBlocks) (limit : Nat)
    (run : PoolHistory held count first last) (owned : HeapOwned first.heap (poolReferences held first))
    (objects : NativeObjects first.heap layout blocks limit) :
    NativeObjects last.heap layout (survivingObjectBlocks last.heap blocks) limit ∧
      HeapOwned last.heap (poolReferences held last) ∧
      (∀ root tree, root ∈ held → NodeRep first.heap root tree → NodeRep last.heap root tree) := by
  obtain ⟨lastOwned, preserve⟩ := pool_history_owned held count first last run owned
  exact ⟨native_objects_read_back first.heap last.heap layout blocks limit objects
    (pool_history_read_back held count first last run), lastOwned, preserve⟩

theorem drop_step_count_le (heap next : NodeHeap) (held pending work : List NodeId)
    (step : stepDrop heap held pending = .more next work) (address : NodeId) :
    strongCount next (held ++ work) address ≤ strongCount heap (held ++ pending) address := by
  obtain ⟨focus, others, remaining, before, _, release, after⟩ := drop_step_release heap next held pending work step
  have balance := release_external_count_delta heap next focus others remaining address release
  rw [strong_count_permute heap _ _ before, ← strong_count_permute next _ _ after]
  omega

theorem drop_loop_count_le (heap next : NodeHeap) (held pending : List NodeId)
    (completed : dropLoop heap held pending = some next) (address : NodeId) :
    strongCount next held address ≤ strongCount heap (held ++ pending) address := by
  rw [dropLoop] at completed
  split at completed
  · contradiction
  · rename_i transition
    have empty := drop_step_done heap held pending transition
    have same := Option.some.inj completed
    simp only [empty, List.append_nil, same, Nat.le_refl]
  · rename_i intermediate work transition
    have balance := drop_step_potential heap intermediate held pending work transition
    exact Nat.le_trans (drop_loop_count_le intermediate next held work completed address)
      (drop_step_count_le heap intermediate held pending work transition address)
termination_by (heapTargets heap).length + pending.length
decreasing_by omega

theorem pool_release_count_le (held : List NodeId) (first next : DropPool)
    (step : PoolRelease held first next) (address : NodeId) :
    strongCount next.heap (poolReferences held next) address ≤ strongCount first.heap (poolReferences held first) address := by
  cases step with
  | worker heap next before after pending work transition =>
      rw [poolReferences, poolReferences,
        strong_count_permute heap _ _ (worker_references_permute held before after pending),
        strong_count_permute next _ _ (worker_references_permute held before after work)]
      exact drop_step_count_le heap next ((held ++ before.flatten) ++ after.flatten) pending work transition address

theorem pool_history_count_le (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) (address : NodeId) :
    strongCount last.heap (poolReferences held last) address ≤ strongCount first.heap (poolReferences held first) address := by
  induction run with
  | nil => exact Nat.le_refl _
  | step advance rest ih => exact Nat.le_trans ih (pool_release_count_le held _ _ advance address)

end Kv9.Radix
