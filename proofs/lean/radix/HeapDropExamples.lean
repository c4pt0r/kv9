import HeapConcurrentDrop

set_option autoImplicit false

namespace Kv9.Radix

def singleLeafHeap : NodeHeap := heapAlloc [] (.leaf aliasEntryZero)

theorem single_leaf_heap_owned : HeapOwned singleLeafHeap [0] :=
  heap_alloc_leaf_owned [] [] aliasEntryZero heap_owned_empty

theorem shared_leaf_heap_owned : HeapOwned singleLeafHeap [0, 0] := by
  exact heap_owned_add_reference singleLeafHeap [0] 0 (.leaf aliasEntryZero) single_leaf_heap_owned (.leaf 0 _ rfl)

theorem drop_shared_leaf_keeps_snapshot : dropLoop singleLeafHeap [0] [0] = some singleLeafHeap := by
  rw [dropLoop]
  change dropLoop singleLeafHeap [0] [] = some singleLeafHeap
  rw [dropLoop]
  rfl

theorem drop_two_leaf_references_reclaims : dropLoop singleLeafHeap [] [0, 0] = some [none] := by
  rw [dropLoop]
  change dropLoop singleLeafHeap [] [0] = some [none]
  rw [dropLoop]
  change dropLoop [none] [] [] = some [none]
  rw [dropLoop]
  rfl

theorem branch_drop_appends_children :
    stepDrop aliasHeap [] [0] = .more [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne)] [1, 2] := by
  rfl

theorem branch_drop_appends_after_remaining_work :
    stepDrop aliasHeap [] [1, 0] = .more [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne)] [1, 1, 2] := by
  rfl

theorem branch_drop_reclaims_descendants : dropLoop aliasHeap [] [0] = some [none, none, none] := by
  rw [dropLoop]
  change dropLoop [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne)] [] [1, 2] = some [none, none, none]
  rw [dropLoop]
  change dropLoop [none, some (.leaf aliasEntryZero), none] [] [1] = some [none, none, none]
  rw [dropLoop]
  change dropLoop [none, none, none] [] [] = some [none, none, none]
  rw [dropLoop]
  rfl

theorem branch_drop_keeps_shared_snapshot : dropLoop aliasHeap [0] [0] = some aliasHeap := by
  rw [dropLoop]
  change dropLoop aliasHeap [0] [] = some aliasHeap
  rw [dropLoop]
  rfl

theorem two_workers_last_owner_handoff :
    PoolHistory [] 2 ⟨singleLeafHeap, [[0], [0]]⟩ ⟨[none], [[], []]⟩ := by
  apply PoolHistory.step (PoolRelease.worker singleLeafHeap singleLeafHeap [] [[0]] [0] [] (by rfl))
  apply PoolHistory.step (PoolRelease.worker singleLeafHeap [none] [[]] [] [0] [] (by rfl))
  exact .nil _

theorem two_workers_reverse_handoff :
    PoolHistory [] 2 ⟨singleLeafHeap, [[0], [0]]⟩ ⟨[none], [[], []]⟩ := by
  apply PoolHistory.step (PoolRelease.worker singleLeafHeap singleLeafHeap [[0]] [] [0] [] (by rfl))
  apply PoolHistory.step (PoolRelease.worker singleLeafHeap [none] [] [[]] [0] [] (by rfl))
  exact .nil _

end Kv9.Radix
