import HeapSubstitute

set_option autoImplicit false

namespace Kv9.Radix

def aliasEntryZero : Entry := ⟨[0], [7]⟩
def aliasEntryOne : Entry := ⟨[1], [8]⟩
def aliasParent : StoredNode := .branch [] none [(0, 1), (1, 2)]
def aliasHeap : NodeHeap := [some aliasParent, some (.leaf aliasEntryZero), some (.leaf aliasEntryOne)]

-- Two roots share a parent whose children each have only one incoming edge.
-- This is a counterexample to treating a child's strong count alone as proof
-- that mutating it cannot affect another snapshot.
theorem shared_parent_unique_child :
    strongCount aliasHeap [0, 0] 0 = 2 ∧ strongCount aliasHeap [0, 0] 1 = 1 := by
  decide

theorem shared_parent_reaches_child : HeapReach aliasHeap 0 1 := by
  exact .child 0 1 aliasParent 0 .root rfl (by simp [aliasParent, storedChildren])

theorem unique_child_not_snapshot_private : ¬ HeapSeparated aliasHeap [0] 1 := by
  intro separate
  exact separate 0 (by simp) shared_parent_reaches_child

def aliasCopiedParent : NodeHeap := heapAlloc aliasHeap aliasParent
def aliasCopiedChild : NodeHeap := cloneChildAt aliasCopiedParent 3 [] none [(0, 1), (1, 2)] 0 0 (.leaf aliasEntryZero)

theorem shared_parent_copy_result : makeRootUnique aliasHeap 0 [0] = some (aliasCopiedParent, 3) := by
  decide

theorem copied_parent_shares_children :
    strongCount aliasCopiedParent [3, 0] 3 = 1 ∧ strongCount aliasCopiedParent [3, 0] 1 = 2 := by
  decide

theorem shared_child_copy_result : makeChildUnique aliasCopiedParent [3, 0] 3 0 = some (aliasCopiedChild, 4) := by
  decide

theorem copied_child_write_isolated :
    let updated := heapWrite aliasCopiedChild 4 (some (.leaf ⟨[0], [9]⟩))
    heapRead updated 1 = some (.leaf aliasEntryZero) ∧ heapRead updated 4 = some (.leaf ⟨[0], [9]⟩) ∧
      arcSlotRead updated [3, 0] (.edge 0 0) = some 1 ∧ arcSlotRead updated [3, 0] (.edge 3 0) = some 4 ∧
      strongCount updated [3, 0] 4 = 1 := by
  decide

end Kv9.Radix
