import SplitLeaf

set_option autoImplicit false

namespace Kv9.Radix

def branchSplitCases (shared : Key) (oldByte : UInt8) (oldRest : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) : Key → Tree
  | [] => .branch shared (some fresh) (.cons oldByte (.branch oldRest terminal children) .nil)
  | newByte :: _ => .branch shared none
      (twoEdges oldByte (.branch oldRest terminal children) newByte (.leaf fresh))

def splitBranch (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry) : Tree :=
  let remaining := fresh.key.drop path.length
  let shared := common pfx remaining
  match pfx.drop shared.length with
  | [] => .branch pfx terminal children -- unreachable under the proper-split guard
  | byte :: rest => branchSplitCases shared byte rest terminal children fresh (remaining.drop shared.length)

theorem branch_split_cases_valid (path shared : Key) (oldByte : UInt8) (oldRest : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) (newRest : Key)
    (childValid : Valid ((path ++ shared) ++ [oldByte]) (.branch oldRest terminal children))
    (freshKey : fresh.key = (path ++ shared) ++ newRest)
    (separated : Separated (oldByte :: oldRest) newRest) :
    Valid path (branchSplitCases shared oldByte oldRest terminal children fresh newRest) := by
  cases newRest with
  | nil =>
      change (∀ entry, some fresh = some entry → entry.key = path ++ shared) ∧
        ValidForest (path ++ shared) (.cons oldByte (.branch oldRest terminal children) .nil)
      refine ⟨?_, childValid, trivial, trivial⟩
      simpa using freshKey
  | cons newByte rest =>
      change (∀ entry : Entry, none = some entry → entry.key = path ++ shared) ∧
        ValidForest (path ++ shared) (twoEdges oldByte (.branch oldRest terminal children) newByte (.leaf fresh))
      refine ⟨by simp, two_edges_valid _ _ _ _ _ childValid ?_ separated⟩
      simp [Valid, HasPrefix, freshKey, List.append_assoc]

theorem branch_split_cases_canonical (shared : Key) (oldByte : UInt8) (oldRest : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) (newRest : Key)
    (childCanonical : Canonical (.branch oldRest terminal children)) :
    Canonical (branchSplitCases shared oldByte oldRest terminal children fresh newRest) := by
  cases newRest with
  | nil =>
      exact ⟨by simp [alternatives], childCanonical, trivial⟩
  | cons byte rest =>
      exact ⟨by simp [two_edges_alternatives],
        two_edges_canonical _ _ _ _ childCanonical trivial⟩

theorem split_branch_valid (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (valid : Valid path (.branch pfx terminal children)) (freshValid : HasPrefix path fresh.key) :
    Valid path (splitBranch path pfx terminal children fresh) := by
  dsimp only [splitBranch]
  generalize oldTailEq : pfx.drop (common pfx (fresh.key.drop path.length)).length = oldTail
  cases oldTail with
  | nil => exact valid
  | cons byte rest =>
      apply branch_split_cases_valid path
      · have decomposition := common_decompose_left pfx (fresh.key.drop path.length)
        rw [oldTailEq] at decomposition
        have context : path ++ pfx =
            ((path ++ common pfx (fresh.key.drop path.length)) ++ [byte]) ++ rest := by
          simpa [List.append_assoc] using congrArg (List.append path) decomposition
        simpa only [Valid, context] using valid
      · simpa [List.append_assoc] using (prefix_drop path fresh.key freshValid).trans
          (congrArg (List.append path) (common_decompose_right pfx (fresh.key.drop path.length)))
      · simpa [oldTailEq] using common_separates pfx (fresh.key.drop path.length)

theorem split_branch_canonical (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (canonical : Canonical (.branch pfx terminal children)) :
    Canonical (splitBranch path pfx terminal children fresh) := by
  dsimp only [splitBranch]
  cases pfx.drop (common pfx (fresh.key.drop path.length)).length with
  | nil => exact canonical
  | cons byte rest => exact branch_split_cases_canonical _ _ _ _ _ _ _ canonical

theorem split_branch_fresh_absent (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (valid : Valid path (.branch pfx terminal children)) (freshValid : HasPrefix path fresh.key)
    (proper : (common pfx (fresh.key.drop path.length)).length < pfx.length) :
    linearLookup fresh.key (entries (.branch pfx terminal children)) = none := by
  apply linear_outside_prefix (path ++ pfx) fresh.key _ (valid_branch_prefix _ _ _ _ valid)
  intro present
  have suffixPresent : HasPrefix pfx (fresh.key.drop path.length) := by
    apply (prefix_cancel path pfx _).mp
    simpa only [← prefix_drop path fresh.key freshValid] using present
  exact proper_common_excludes_prefix pfx _ proper suffixPresent

theorem linear_append_fresh (rows : List Entry) (fresh : Entry) (query : Key)
    (absent : linearLookup fresh.key rows = none) :
    linearLookup query (rows ++ [fresh]) = put fresh.key fresh.value (fun key => linearLookup key rows) query := by
  by_cases hit : query = fresh.key
  · subst query
    simp [linear_append, absent, linearLookup, put]
  · simp [linear_append, linearLookup, put, hit]

theorem branch_split_cases_lookup (shared : Key) (oldByte : UInt8) (oldRest : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) (newRest query : Key)
    (absent : linearLookup fresh.key (entries (.branch oldRest terminal children)) = none) :
    linearLookup query (entries (branchSplitCases shared oldByte oldRest terminal children fresh newRest)) =
      put fresh.key fresh.value (fun key => linearLookup key (entries (.branch oldRest terminal children))) query := by
  cases newRest with
  | nil => simp [branchSplitCases, entries, forestEntries, linearLookup, put]
  | cons byte rest =>
      by_cases less : oldByte < byte
      · simpa [branchSplitCases, entries, forestEntries, twoEdges, less] using
          linear_append_fresh (entries (.branch oldRest terminal children)) fresh query absent
      · simp [branchSplitCases, entries, forestEntries, twoEdges, less, linearLookup, put]

theorem split_branch_entries_refine (path pfx : Key) (terminal : Option Entry) (children : Forest)
    (fresh : Entry) (query : Key)
    (valid : Valid path (.branch pfx terminal children)) (freshValid : HasPrefix path fresh.key)
    (proper : (common pfx (fresh.key.drop path.length)).length < pfx.length) :
    linearLookup query (entries (splitBranch path pfx terminal children fresh)) =
      put fresh.key fresh.value (fun key => linearLookup key (entries (.branch pfx terminal children))) query := by
  have absent := split_branch_fresh_absent path pfx terminal children fresh valid freshValid proper
  dsimp only [splitBranch]
  generalize oldTailEq : pfx.drop (common pfx (fresh.key.drop path.length)).length = oldTail
  cases oldTail with
  | nil =>
      have bound := List.drop_eq_nil_iff.mp oldTailEq
      omega
  | cons byte rest => exact branch_split_cases_lookup _ _ _ _ _ _ _ _ absent

theorem split_branch_lookup_refines (path pfx : Key) (terminal : Option Entry) (children : Forest)
    (fresh : Entry) (remaining : Key)
    (valid : Valid path (.branch pfx terminal children)) (freshValid : HasPrefix path fresh.key)
    (proper : (common pfx (fresh.key.drop path.length)).length < pfx.length) :
    treeLookup (path ++ remaining) (splitBranch path pfx terminal children fresh) remaining =
      put fresh.key fresh.value (fun query => linearLookup query (entries (.branch pfx terminal children)))
        (path ++ remaining) := by
  rw [tree_lookup_refines _ _ _ (split_branch_valid path pfx terminal children fresh valid freshValid),
    split_branch_entries_refine path pfx terminal children fresh _ valid freshValid proper]

end Kv9.Radix
