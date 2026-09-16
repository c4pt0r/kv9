import DeleteWordLoop

set_option autoImplicit false

namespace Kv9.Radix

-- The split helpers read byte options at an absolute cut. The both-exhausted
-- leaf case is an assertion failure, not an invented successful replacement.
def leafSplitBytes (pfx : Key) (old fresh : Entry) : Option UInt8 → Option UInt8 → Option Tree
  | none, none => none
  | none, some byte => some (.branch pfx (some old) (.cons byte (.leaf fresh) .nil))
  | some byte, none => some (.branch pfx (some fresh) (.cons byte (.leaf old) .nil))
  | some a, some b => some (.branch pfx none (twoEdges a (.leaf old) b (.leaf fresh)))

def splitLeafAt (depth sharedLength : Nat) (old fresh : Entry) : Option Tree :=
  let cut := depth + sharedLength
  if depth ≤ cut ∧ cut ≤ old.key.length then
    leafSplitBytes ((old.key.drop depth).take sharedLength) old fresh old.key[cut]? fresh.key[cut]?
  else none

def splitBranchAt (depth sharedLength : Nat) (pfx : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) : Option Tree :=
  match pfx[sharedLength]? with
  | none => none
  | some oldByte =>
      let oldChild := Tree.branch (pfx.drop (sharedLength + 1)) terminal children
      match fresh.key[depth + sharedLength]? with
      | none => some (.branch (pfx.take sharedLength) (some fresh) (.cons oldByte oldChild .nil))
      | some byte => some (.branch (pfx.take sharedLength) none (twoEdges oldByte oldChild byte (.leaf fresh)))

theorem split_bytes_refine (pfx : Key) (old fresh : Entry) (left right : Key)
    (distinct : ¬ (left = [] ∧ right = [])) :
    leafSplitBytes pfx old fresh left.head? right.head? = some (leafSplitCases pfx old fresh left right) := by
  cases left <;> cases right <;> simp_all [leafSplitBytes, leafSplitCases]

theorem absolute_cut_access (key : Key) (depth sharedLength : Nat) :
    key[depth + sharedLength]? = ((key.drop depth).drop sharedLength).head? := by
  rw [List.drop_drop, drop_head_access]

theorem split_leaf_at_refines (path : Key) (old fresh : Entry)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key)
    (different : old.key ≠ fresh.key) :
    splitLeafAt path.length (common (old.key.drop path.length) (fresh.key.drop path.length)).length old fresh =
      some (splitLeaf path old fresh) := by
  have bound := leaf_split_cut_bounds path old fresh oldValid freshValid
  have validCut : path.length ≤ path.length + (common (old.key.drop path.length) (fresh.key.drop path.length)).length ∧
      path.length + (common (old.key.drop path.length) (fresh.key.drop path.length)).length ≤ old.key.length :=
    ⟨bound.1, bound.2.1⟩
  have taken := prefix_take _ _ (common_left (old.key.drop path.length) (fresh.key.drop path.length))
  have distinct := split_leaf_cannot_exhaust_both path old fresh oldValid freshValid different
  rw [splitLeafAt, if_pos validCut, taken, absolute_cut_access, absolute_cut_access]
  exact split_bytes_refine _ old fresh _ _ distinct

theorem split_branch_at_refines (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (proper : (common pfx (fresh.key.drop path.length)).length < pfx.length) :
    splitBranchAt path.length (common pfx (fresh.key.drop path.length)).length pfx terminal children fresh =
      some (splitBranch path pfx terminal children fresh) := by
  let shared := common pfx (fresh.key.drop path.length)
  have taken : pfx.take shared.length = shared := prefix_take _ _ (common_left _ _)
  have oldSuffix : pfx.drop shared.length = pfx[shared.length] :: pfx.drop (shared.length + 1) :=
    List.drop_eq_getElem_cons proper
  have oldByte : pfx[shared.length]? = some pfx[shared.length] := by simp
  change splitBranchAt path.length shared.length pfx terminal children fresh = _
  rw [splitBranchAt, oldByte]
  simp only [taken]
  rw [absolute_cut_access]
  change (match ((fresh.key.drop path.length).drop shared.length).head? with
    | none => some (Tree.branch shared (some fresh) (.cons pfx[shared.length] (.branch (pfx.drop (shared.length + 1)) terminal children) .nil))
    | some byte => some (Tree.branch shared none (twoEdges pfx[shared.length] (.branch (pfx.drop (shared.length + 1)) terminal children) byte (.leaf fresh)))) = _
  have reference : splitBranch path pfx terminal children fresh =
      branchSplitCases shared pfx[shared.length] (pfx.drop (shared.length + 1)) terminal children fresh
        ((fresh.key.drop path.length).drop shared.length) := by
    unfold splitBranch
    change (match pfx.drop shared.length with
      | [] => Tree.branch pfx terminal children
      | byte :: rest => branchSplitCases shared byte rest terminal children fresh ((fresh.key.drop path.length).drop shared.length)) = _
    rw [oldSuffix]
  rw [reference]
  cases (fresh.key.drop path.length).drop shared.length <;> rfl

end Kv9.Radix
