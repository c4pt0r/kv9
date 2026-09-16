import Common
import Normalize

set_option autoImplicit false

namespace Kv9.Radix

def twoEdges (a : UInt8) (left : Tree) (b : UInt8) (right : Tree) : Forest :=
  if a < b then .cons a left (.cons b right .nil) else .cons b right (.cons a left .nil)

theorem two_edges_valid (path : Key) (a : UInt8) (left : Tree) (b : UInt8) (right : Tree)
    (leftValid : Valid (path ++ [a]) left) (rightValid : Valid (path ++ [b]) right) (different : a ≠ b) :
    ValidForest path (twoEdges a left b right) := by
  by_cases less : a < b <;> simp [twoEdges, less, ValidForest, MissingEdge, leftValid, rightValid, different, Ne.symm different]

theorem two_edges_canonical (a : UInt8) (left : Tree) (b : UInt8) (right : Tree)
    (leftCanonical : Canonical left) (rightCanonical : Canonical right) :
    CanonicalForest (twoEdges a left b right) := by
  by_cases less : a < b <;> simp [twoEdges, less, CanonicalForest, leftCanonical, rightCanonical]

theorem two_edges_alternatives (terminal : Option Entry) (a : UInt8) (left : Tree) (b : UInt8) (right : Tree) :
    alternatives terminal (twoEdges a left b right) = 2 + terminal.toList.length := by
  by_cases less : a < b <;> simp [twoEdges, less, alternatives] <;> omega

def leafSplitCases (pfx : Key) (old fresh : Entry) : Key → Key → Tree
  | [], [] => .leaf fresh -- unreachable in split_leaf after its distinct-key guard
  | [], byte :: _ => .branch pfx (some old) (.cons byte (.leaf fresh) .nil)
  | byte :: _, [] => .branch pfx (some fresh) (.cons byte (.leaf old) .nil)
  | a :: _, b :: _ => .branch pfx none (twoEdges a (.leaf old) b (.leaf fresh))

def splitLeaf (path : Key) (old fresh : Entry) : Tree :=
  let left := old.key.drop path.length
  let right := fresh.key.drop path.length
  let shared := common left right
  leafSplitCases shared old fresh (left.drop shared.length) (right.drop shared.length)

theorem leaf_split_cases_valid (path pfx : Key) (old fresh : Entry) (left right : Key)
    (oldKey : old.key = (path ++ pfx) ++ left) (freshKey : fresh.key = (path ++ pfx) ++ right)
    (separated : Separated left right) : Valid path (leafSplitCases pfx old fresh left right) := by
  cases left with
  | nil =>
      cases right with
      | nil => simp [leafSplitCases, Valid, HasPrefix, freshKey]
      | cons byte rest =>
          simp [leafSplitCases, Valid, ValidForest, MissingEdge, HasPrefix, oldKey, freshKey, List.append_assoc]
  | cons a left =>
      cases right with
      | nil =>
          simp [leafSplitCases, Valid, ValidForest, MissingEdge, HasPrefix, oldKey, freshKey, List.append_assoc]
      | cons b right =>
          change (∀ entry : Entry, none = some entry → entry.key = path ++ pfx) ∧
            ValidForest (path ++ pfx) (twoEdges a (.leaf old) b (.leaf fresh))
          refine ⟨by simp, two_edges_valid _ _ _ _ _ ?_ ?_ separated⟩
          · simp [Valid, HasPrefix, oldKey, List.append_assoc]
          · simp [Valid, HasPrefix, freshKey, List.append_assoc]

theorem leaf_split_cases_canonical (pfx : Key) (old fresh : Entry) (left right : Key) :
    Canonical (leafSplitCases pfx old fresh left right) := by
  cases left with
  | nil => cases right <;> simp [leafSplitCases, Canonical, alternatives, CanonicalForest]
  | cons a left =>
      cases right with
      | nil => simp [leafSplitCases, Canonical, alternatives, CanonicalForest]
      | cons b right =>
          exact ⟨by simp [two_edges_alternatives], two_edges_canonical a (.leaf old) b (.leaf fresh) trivial trivial⟩

theorem split_leaf_valid (path : Key) (old fresh : Entry)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key) :
    Valid path (splitLeaf path old fresh) := by
  unfold splitLeaf
  apply leaf_split_cases_valid path
  · simpa [List.append_assoc] using (prefix_drop path old.key oldValid).trans
      (congrArg (List.append path) (common_decompose_left (old.key.drop path.length) (fresh.key.drop path.length)))
  · simpa [List.append_assoc] using (prefix_drop path fresh.key freshValid).trans
      (congrArg (List.append path) (common_decompose_right (old.key.drop path.length) (fresh.key.drop path.length)))
  · exact common_separates _ _

theorem split_leaf_canonical (path : Key) (old fresh : Entry) : Canonical (splitLeaf path old fresh) :=
  leaf_split_cases_canonical _ _ _ _ _

theorem split_leaf_cannot_exhaust_both (path : Key) (old fresh : Entry)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key) (different : old.key ≠ fresh.key) :
    let shared := common (old.key.drop path.length) (fresh.key.drop path.length)
    ¬ ((old.key.drop path.length).drop shared.length = [] ∧
       (fresh.key.drop path.length).drop shared.length = []) := by
  intro _ ⟨left, right⟩
  have same := common_exhausted_both _ _ left right
  have oldRebuild := prefix_drop path old.key oldValid
  have freshRebuild := prefix_drop path fresh.key freshValid
  exact different (oldRebuild.trans ((congrArg (List.append path) same).trans freshRebuild.symm))

theorem linear_two_old_first (old fresh : Entry) (query : Key) (different : old.key ≠ fresh.key) :
    linearLookup query [old, fresh] = put fresh.key fresh.value (singleton old.key old.value) query := by
  by_cases oldHit : query = old.key <;> by_cases freshHit : query = fresh.key <;>
    simp_all [linearLookup, put, singleton]

theorem linear_two_new_first (old fresh : Entry) (query : Key) :
    linearLookup query [fresh, old] = put fresh.key fresh.value (singleton old.key old.value) query := by
  by_cases oldHit : query = old.key <;> by_cases freshHit : query = fresh.key <;>
    simp_all [linearLookup, put, singleton]

theorem leaf_split_cases_lookup (path pfx : Key) (old fresh : Entry) (left right query : Key)
    (oldKey : old.key = (path ++ pfx) ++ left) (freshKey : fresh.key = (path ++ pfx) ++ right)
    (different : old.key ≠ fresh.key) :
    linearLookup query (entries (leafSplitCases pfx old fresh left right)) =
      put fresh.key fresh.value (singleton old.key old.value) query := by
  cases left with
  | nil =>
      cases right with
      | nil => exact False.elim (different (oldKey.trans freshKey.symm))
      | cons byte rest => exact linear_two_old_first old fresh query different
  | cons a left =>
      cases right with
      | nil => exact linear_two_new_first old fresh query
      | cons b right =>
          by_cases less : a < b
          · simpa [leafSplitCases, entries, twoEdges, less, forestEntries] using linear_two_old_first old fresh query different
          · simpa [leafSplitCases, entries, twoEdges, less, forestEntries] using linear_two_new_first old fresh query

theorem split_leaf_entries_refine (path : Key) (old fresh : Entry) (query : Key)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key) (different : old.key ≠ fresh.key) :
    linearLookup query (entries (splitLeaf path old fresh)) =
      put fresh.key fresh.value (singleton old.key old.value) query := by
  unfold splitLeaf
  apply leaf_split_cases_lookup path
  · simpa [List.append_assoc] using (prefix_drop path old.key oldValid).trans
      (congrArg (List.append path) (common_decompose_left (old.key.drop path.length) (fresh.key.drop path.length)))
  · simpa [List.append_assoc] using (prefix_drop path fresh.key freshValid).trans
      (congrArg (List.append path) (common_decompose_right (old.key.drop path.length) (fresh.key.drop path.length)))
  · exact different

theorem split_leaf_lookup_refines (path : Key) (old fresh : Entry) (remaining : Key)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key) (different : old.key ≠ fresh.key) :
    treeLookup (path ++ remaining) (splitLeaf path old fresh) remaining =
      put fresh.key fresh.value (singleton old.key old.value) (path ++ remaining) := by
  rw [tree_lookup_refines _ _ _ (split_leaf_valid path old fresh oldValid freshValid),
    split_leaf_entries_refine path old fresh _ oldValid freshValid different]

end Kv9.Radix
