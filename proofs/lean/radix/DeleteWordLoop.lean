import DeleteLoop

set_option autoImplicit false

namespace Kv9.Radix

-- This machine performs the two depth updates with word arithmetic, as Rust
-- does. Valid key/slice lengths are explicit library/heap premises, not Nat
-- arithmetic silently substituted for potentially wrapping machine addition.
def deleteWordLoop (query : Key) (tree : Tree) (depth : USize) (frames : List DeleteFrame) : Option (Option Tree) :=
  match tree with
  | .leaf _ => unwindDeleteChecked frames none
  | .branch pfx terminal children =>
      let endOffset := depth + USize.ofNat pfx.length
      if endOffset.toNat = query.length then
        unwindDeleteChecked frames (normalize (.branch pfx none children))
      else
        match query[endOffset.toNat]? with
        | none => none
        | some byte =>
            match edgeSearch byte children with
            | .miss _ => none
            | .hit index =>
                match access : edgeGet index children with
                | none => none
                | some (storedByte, child) =>
                    deleteWordLoop query child (endOffset + 1)
                      (⟨pfx, terminal, edgeRemove index children, index, storedByte⟩ :: frames)
termination_by treeWork tree
decreasing_by
  have smaller := edge_get_work children index storedByte child access
  simp only [treeWork]
  omega

theorem word_prefix_offset_exact (depth : USize) (pfx query : Key)
    (within : depth.toNat + pfx.length ≤ query.length) (fits : query.length < USize.size) :
    (depth + USize.ofNat pfx.length).toNat = depth.toNat + pfx.length := by
  have prefixFits : pfx.length < USize.size := by omega
  have conversion : (USize.ofNat pfx.length).toNat = pfx.length := USize.toNat_ofNat_of_lt prefixFits
  have addition := offset_word_add_exact depth (USize.ofNat pfx.length) query.length
    (by simpa only [conversion] using within) fits
  simpa only [conversion] using addition

theorem word_edge_offset_exact (offset : USize) (query : Key)
    (within : offset.toNat + 1 ≤ query.length) (fits : query.length < USize.size) :
    (offset + 1).toNat = offset.toNat + 1 := by
  simpa using word_add_exact offset 1 (by simpa using Nat.lt_of_le_of_lt within fits)

theorem delete_word_loop_refines (query : Key) (tree : Tree) (depth : USize) (frames : List DeleteFrame)
    (ordered : OrderedTree tree) (bounded : depth.toNat ≤ query.length)
    (fits : query.length < USize.size)
    (present : treeLookup query tree (query.drop depth.toNat) ≠ none) :
    deleteWordLoop query tree depth frames = deleteLoop query tree depth.toNat frames := by
  cases treeEq : tree with
  | leaf entry => rw [deleteWordLoop, deleteLoop]
  | branch pfx terminal children =>
      rw [treeEq] at ordered present
      cases matched : strip pfx (query.drop depth.toNat) with
      | none => simp [treeLookup, matched] at present
      | some suffix =>
          obtain ⟨within, remaining, atEnd⟩ := deletion_offset query pfx suffix depth.toNat bounded matched
          have addition := word_prefix_offset_exact depth pfx query within fits
          cases suffix with
          | nil =>
              have done := atEnd.mpr rfl
              rw [deleteWordLoop, deleteLoop]
              simp only [addition, if_pos done]
          | cons byte rest =>
              have notDone : depth.toNat + pfx.length ≠ query.length := by
                intro equal
                have impossible := atEnd.mp equal
                cases impossible
              have access : query[depth.toNat + pfx.length]? = some byte := by
                rw [← drop_head_access, remaining]
                rfl
              have nextRemaining : query.drop (depth.toNat + pfx.length + 1) = rest := by
                rw [List.drop_add_one_eq_tail_drop, remaining]
                rfl
              have nextBound := (byte_descent_bounds query depth.toNat pfx.length byte access).1
              have nextOffset : (depth + USize.ofNat pfx.length + 1).toNat = depth.toNat + pfx.length + 1 := by
                rw [word_edge_offset_exact _ query (by simpa only [addition] using nextBound) fits, addition]
              have forestPresent : forestLookup query children byte rest ≠ none := by
                simpa only [treeLookup, matched] using present
              obtain ⟨index, child, search, childAccess, childPresent⟩ :=
                forest_present_search query rest children byte ordered forestPresent
              have childOrdered := edge_get_ordered children index byte child ordered childAccess
              rw [deleteWordLoop, deleteLoop]
              simp only [addition, if_neg notDone, access, search]
              split
              · rename_i absentAccess
                rw [childAccess] at absentAccess
                cases absentAccess
              · rename_i storedByte next accessEq
                have pair := Option.some.inj (accessEq.symm.trans childAccess)
                cases pair
                have recursive := delete_word_loop_refines query child (depth + USize.ofNat pfx.length + 1)
                  (⟨pfx, terminal, edgeRemove index children, index, byte⟩ :: frames) childOrdered
                  (by simpa only [nextOffset] using nextBound) fits
                  (by simpa only [nextOffset, nextRemaining] using childPresent)
                split
                · rename_i absentAccess
                  rw [childAccess] at absentAccess
                  cases absentAccess
                · rename_i storedByte next accessEq
                  have pair := Option.some.inj (accessEq.symm.trans childAccess)
                  cases pair
                  simpa only [nextOffset] using recursive
termination_by treeWork tree
decreasing_by
  have smaller := edge_get_work children index byte child childAccess
  simp only [treeEq, treeWork]
  omega

def eraseWordLoop (query : Key) (root : Option Tree) : Option (Option Tree) :=
  if optionalLookup query root = none then some root
  else match root with
    | none => none
    | some tree => deleteWordLoop query tree 0 []

theorem erase_word_loop_refines (query : Key) (root : Option Tree)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) (fits : query.length < USize.size) :
    eraseWordLoop query root = some (eraseRoot query root) := by
  cases root with
  | none => rfl
  | some tree =>
      by_cases absent : treeLookup query tree query = none
      · simp [eraseWordLoop, optionalLookup, eraseRoot, absent]
      · have simulation := delete_word_loop_refines query tree 0 [] (ordered tree rfl) (Nat.zero_le _) fits
          (by simpa using absent)
        have reference := erase_loop_refines query (some tree) ordered
        simpa [eraseWordLoop, eraseLoop, optionalLookup, absent, simulation] using reference

theorem erase_word_loop_no_assertion_failure (query : Key) (root : Option Tree)
    (good : Good root) (fits : query.length < USize.size) : eraseWordLoop query root ≠ none := by
  rw [erase_word_loop_refines query root (fun tree eq => (good.2 tree eq).2) fits]
  simp

theorem erase_word_loop_lookup (query removed : Key) (root : Option Tree)
    (good : Good root) (fits : removed.length < USize.size) :
    (eraseWordLoop removed root).map (optionalLookup query) =
      some (if query = removed then none else optionalLookup query root) := by
  rw [erase_word_loop_refines removed root (fun tree eq => (good.2 tree eq).2) fits]
  simp only [Option.map_some, erase_root_lookup query removed root good.1]

theorem erase_word_loop_good (query : Key) (root result : Option Tree)
    (good : Good root) (fits : query.length < USize.size)
    (completed : eraseWordLoop query root = some result) : Good result := by
  rw [erase_word_loop_refines query root (fun tree eq => (good.2 tree eq).2) fits] at completed
  have same := Option.some.inj completed
  simpa only [← same] using erase_root_good query root good

-- The public wrapper performs one presence guard, takes the root, runs the
-- deletion machine and decrements the stored word count only on success.
def eraseWordState (state : WordCounted) (query : Key) : Option (WordCounted × Bool) :=
  if optionalLookup query state.root = none then some (state, false)
  else match state.root with
    | none => none
    | some tree => (deleteWordLoop query tree 0 []).map
        (fun root => (⟨root, state.size - 1⟩, true))

theorem erase_word_state_refines (state : WordCounted) (query : Key)
    (good : WordGood state) (fits : query.length < USize.size) :
    eraseWordState state query = some (wordErase state query) := by
  by_cases absent : optionalLookup query state.root = none
  · simp [eraseWordState, wordErase, absent]
  · cases rootEq : state.root with
    | none => simp [optionalLookup, rootEq] at absent
    | some tree =>
        have simulation := erase_word_loop_refines query state.root
          (fun t eq => (good.1.2 t eq).2) fits
        have loop : deleteWordLoop query tree 0 [] = some (eraseRoot query state.root) := by
          rw [eraseWordLoop.eq_def, if_neg absent, rootEq] at simulation
          simpa only [rootEq] using simulation
        rw [eraseWordState.eq_def, if_neg absent, rootEq]
        simp only [loop, Option.map_some]
        rw [wordErase, if_neg absent]

theorem erase_word_state_good (state result : WordCounted) (query : Key) (removed : Bool)
    (good : WordGood state) (fits : query.length < USize.size)
    (completed : eraseWordState state query = some (result, removed)) : WordGood result := by
  rw [erase_word_state_refines state query good fits] at completed
  have pair := Option.some.inj completed
  have root := congrArg Prod.fst pair
  simpa only [root] using word_erase_good state query good

theorem erase_word_state_no_failure (state : WordCounted) (query : Key)
    (good : WordGood state) (fits : query.length < USize.size) : eraseWordState state query ≠ none := by
  rw [erase_word_state_refines state query good fits]
  simp

end Kv9.Radix
