import NativeCowFinish

set_option autoImplicit false

namespace Kv9.Radix

theorem cow_snapshot_release_after_shared_decision :
    strongCount singleLeafHeap [0, 0] 0 = 2 ∧
      PoolHistory [0] 1 ⟨singleLeafHeap, [[0]]⟩ ⟨singleLeafHeap, [[]]⟩ ∧
      strongCount singleLeafHeap [0] 0 = 1 := by
  refine ⟨by decide, ?_, by decide⟩
  exact .step (.worker singleLeafHeap singleLeafHeap [] [] [0] [] (by rfl)) (.nil _)

theorem cow_copy_after_last_snapshot_release :
    copyOwnedRoot singleLeafHeap 0 [] = some ([none, some (.leaf aliasEntryZero)], 1) := by
  change (dropLoop [some (.leaf aliasEntryZero), some (.leaf aliasEntryZero)] [1] [0]).bind
    (fun next => some (next, 1)) = _
  rw [dropLoop]
  change (dropLoop [none, some (.leaf aliasEntryZero)] [1] []).bind (fun next => some (next, 1)) = _
  rw [dropLoop]
  rfl

theorem cow_copy_branch_retains_children_after_old_drop :
    copyOwnedRoot aliasHeap 0 [] =
      some ([none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne), some aliasParent], 3) := by
  change (dropLoop (heapAlloc aliasHeap aliasParent) [3] [0]).bind (fun next => some (next, 3)) = _
  rw [dropLoop]
  change (dropLoop [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne), some aliasParent] [3] [1, 2]).bind
    (fun next => some (next, 3)) = _
  rw [dropLoop]
  change (dropLoop [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne), some aliasParent] [3] [1]).bind
    (fun next => some (next, 3)) = _
  rw [dropLoop]
  change (dropLoop [none, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne), some aliasParent] [3] []).bind
    (fun next => some (next, 3)) = _
  rw [dropLoop]
  rfl

theorem cow_release_between_child_retains :
    PayloadCopyHistory 0 [] 3 ⟨⟨aliasHeap, [[0]]⟩, [], [1, 2]⟩ ⟨⟨aliasHeap, [[]]⟩, [2, 1], []⟩ := by
  apply PayloadCopyHistory.step (.retain ⟨aliasHeap, [[0]]⟩ [] 1 [2])
  apply PayloadCopyHistory.step (.release ⟨aliasHeap, [[0]]⟩ ⟨aliasHeap, [[]]⟩ [1] [2]
    (.worker aliasHeap aliasHeap [] [] [0] [] (by rfl)))
  exact .step (.retain ⟨aliasHeap, [[]]⟩ [1] 2 []) (.nil _)

theorem cow_missing_old_release_leaks :
    ¬ HeapLive (heapAlloc singleLeafHeap (.leaf aliasEntryZero)) [1] := by
  intro live
  have impossible : ¬ 0 < strongCount (heapAlloc singleLeafHeap (.leaf aliasEntryZero)) [1] 0 := by decide
  exact impossible (live 0 (.leaf aliasEntryZero) rfl)

theorem cow_missing_child_retain_rejected (tree : Tree) :
    ¬ PayloadCopyValid 0 [] aliasParent tree ⟨⟨aliasHeap, [[]]⟩, [2], []⟩ := by
  intro valid
  have wrong : ([2] : List NodeId) = [1, 2] := valid.progress
  cases wrong

end Kv9.Radix
