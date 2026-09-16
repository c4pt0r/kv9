import Ordered

set_option autoImplicit false

namespace Kv9.Radix

mutual
  def insertTree (path : Key) (fresh : Entry) : Tree → Tree × Bool
    | .leaf old =>
        if old.key = fresh.key then (.leaf fresh, false) else (splitLeaf path old fresh, true)
    | .branch pfx terminal children =>
        let remaining := fresh.key.drop path.length
        if (common pfx remaining).length < pfx.length then
          (splitBranch path pfx terminal children fresh, true)
        else
          match remaining.drop pfx.length with
          | [] => (.branch pfx (some fresh) children, terminal.isNone)
          | byte :: _ =>
              let result := insertForest (path ++ pfx) fresh children byte
              (.branch pfx terminal result.1, result.2)
  def insertForest (path : Key) (fresh : Entry) : Forest → UInt8 → Forest × Bool
    | .nil, byte => (.cons byte (.leaf fresh) .nil, true)
    | .cons byte child tail, target =>
        if target = byte then
          let result := insertTree (path ++ [byte]) fresh child
          (.cons byte result.1 tail, result.2)
        else if target < byte then (.cons target (.leaf fresh) (.cons byte child tail), true)
        else
          let result := insertForest path fresh tail target
          (.cons byte child result.1, result.2)
end

theorem matching_key_decomposition (path pfx key : Key) (valid : HasPrefix path key)
    (matching : ¬ (common pfx (key.drop path.length)).length < pfx.length) :
    key = (path ++ pfx) ++ (key.drop path.length).drop pfx.length := by
  have shared : common pfx (key.drop path.length) = pfx := by
    apply common_full_left
    have bound := (common_bounds pfx (key.drop path.length)).1
    omega
  have pfxValid : HasPrefix pfx (key.drop path.length) := by
    simpa only [shared] using common_right pfx (key.drop path.length)
  simpa [List.append_assoc] using (prefix_drop path key valid).trans
    (congrArg (List.append path) (prefix_drop pfx (key.drop path.length) pfxValid))

theorem byte_reverse_lt (a b : UInt8) (different : a ≠ b) (notLess : ¬ a < b) : b.toNat < a.toNat := by
  have differentNat : a.toNat ≠ b.toNat := fun same => different (UInt8.toNat_inj.mp same)
  have notLessNat : ¬ a.toNat < b.toNat := notLess
  omega

theorem insert_forest_missing (path : Key) (fresh : Entry) (forest : Forest) (target absent : UInt8)
    (different : absent ≠ target) (missing : MissingEdge absent forest) :
    MissingEdge absent (insertForest path fresh forest target).1 := by
  cases forest with
  | nil => exact ⟨different, trivial⟩
  | cons byte child tail =>
      by_cases same : target = byte
      · simpa only [insertForest, if_pos same, MissingEdge] using missing
      · by_cases less : target < byte
        · simpa only [insertForest, if_neg same, if_pos less, MissingEdge] using And.intro different missing
        · rw [insertForest, if_neg same, if_neg less]
          exact ⟨missing.1, insert_forest_missing path fresh tail target absent different missing.2⟩

theorem insert_forest_above (path : Key) (fresh : Entry) (forest : Forest) (target lower : UInt8)
    (less : lower.toNat < target.toNat) (above : Above lower forest) :
    Above lower (insertForest path fresh forest target).1 := by
  cases forest with
  | nil => exact ⟨less, trivial⟩
  | cons byte child tail =>
      by_cases same : target = byte
      · simpa only [insertForest, if_pos same, Above] using above
      · by_cases before : target < byte
        · simpa only [insertForest, if_neg same, if_pos before, Above] using And.intro less above
        · rw [insertForest, if_neg same, if_neg before]
          exact ⟨above.1, insert_forest_above path fresh tail target lower less above.2⟩

mutual
  theorem insert_tree_valid (path : Key) (fresh : Entry) (tree : Tree)
      (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
      Valid path (insertTree path fresh tree).1 := by
    cases tree with
    | leaf old =>
        by_cases same : old.key = fresh.key
        · simpa only [insertTree, if_pos same, Valid] using freshValid
        · simpa only [insertTree, if_neg same] using split_leaf_valid path old fresh valid freshValid
    | branch pfx terminal children =>
        by_cases split : (common pfx (fresh.key.drop path.length)).length < pfx.length
        · simpa only [insertTree, if_pos split] using split_branch_valid path pfx terminal children fresh valid freshValid
        · have key := matching_key_decomposition path pfx fresh.key freshValid split
          dsimp only [insertTree]
          rw [if_neg split]
          generalize suffixEq : (fresh.key.drop path.length).drop pfx.length = suffix
          rw [suffixEq] at key
          cases suffix with
          | nil =>
              refine ⟨?_, valid.2⟩
              simpa using key
          | cons byte rest =>
              refine ⟨valid.1, insert_forest_valid (path ++ pfx) fresh children byte valid.2 ordered ?_⟩
              exact ⟨rest, by simpa [List.append_assoc] using key⟩
  theorem insert_forest_valid (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8)
      (valid : ValidForest path forest) (ordered : OrderedForest forest)
      (freshValid : HasPrefix (path ++ [target]) fresh.key) :
      ValidForest path (insertForest path fresh forest target).1 := by
    cases forest with
    | nil => exact ⟨freshValid, trivial, trivial⟩
    | cons byte child tail =>
        by_cases same : target = byte
        · subst target
          rw [insertForest, if_pos rfl]
          exact ⟨insert_tree_valid _ _ _ valid.1 ordered.1 freshValid, valid.2⟩
        · by_cases less : target < byte
          · rw [insertForest, if_neg same, if_pos less]
            refine ⟨freshValid, valid, ?_⟩
            exact above_missing target (.cons byte child tail)
              ⟨less, above_trans target byte tail less ordered.2.2⟩
          · rw [insertForest, if_neg same, if_neg less]
            exact ⟨valid.1, insert_forest_valid path fresh tail target valid.2.1 ordered.2.1 freshValid,
              insert_forest_missing path fresh tail target byte (Ne.symm same) valid.2.2⟩
end

mutual
  theorem insert_tree_ordered (path : Key) (fresh : Entry) (tree : Tree) (ordered : OrderedTree tree) :
      OrderedTree (insertTree path fresh tree).1 := by
    cases tree with
    | leaf old =>
        by_cases same : old.key = fresh.key
        · simp [insertTree, same, OrderedTree]
        · simpa [insertTree, same] using split_leaf_ordered path old fresh
    | branch pfx terminal children =>
        by_cases split : (common pfx (fresh.key.drop path.length)).length < pfx.length
        · simpa only [insertTree, if_pos split] using split_branch_ordered path pfx terminal children fresh ordered
        · dsimp only [insertTree]
          rw [if_neg split]
          cases (fresh.key.drop path.length).drop pfx.length with
          | nil => exact ordered
          | cons byte rest => exact insert_forest_ordered (path ++ pfx) fresh children byte ordered
  theorem insert_forest_ordered (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8)
      (ordered : OrderedForest forest) : OrderedForest (insertForest path fresh forest target).1 := by
    cases forest with
    | nil => exact ⟨trivial, trivial, trivial⟩
    | cons byte child tail =>
        by_cases same : target = byte
        · subst target
          rw [insertForest, if_pos rfl]
          exact ⟨insert_tree_ordered _ _ _ ordered.1, ordered.2⟩
        · by_cases less : target < byte
          · rw [insertForest, if_neg same, if_pos less]
            exact ⟨trivial, ordered, less, above_trans target byte tail less ordered.2.2⟩
          · rw [insertForest, if_neg same, if_neg less]
            exact ⟨ordered.1, insert_forest_ordered path fresh tail target ordered.2.1,
              insert_forest_above path fresh tail target byte (byte_reverse_lt target byte same less) ordered.2.2⟩
end

theorem insert_forest_alternatives (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8)
    (terminal : Option Entry) : alternatives terminal forest ≤ alternatives terminal (insertForest path fresh forest target).1 := by
  cases forest with
  | nil => simp [insertForest, alternatives]
  | cons byte child tail =>
      by_cases same : target = byte
      · simp [insertForest, same, alternatives]
      · by_cases less : target < byte
        · simp [insertForest, same, less, alternatives]
        · simp only [insertForest, if_neg same, if_neg less, alternatives]
          exact Nat.add_le_add_left (insert_forest_alternatives path fresh tail target terminal) 1

theorem alternatives_terminal (terminal : Option Entry) (forest : Forest) (fresh : Entry) :
    alternatives terminal forest ≤ alternatives (some fresh) forest := by
  cases forest with
  | nil => cases terminal <;> simp [alternatives]
  | cons byte child tail => exact Nat.add_le_add_left (alternatives_terminal terminal tail fresh) 1

mutual
  theorem insert_tree_canonical (path : Key) (fresh : Entry) (tree : Tree) (canonical : Canonical tree) :
      Canonical (insertTree path fresh tree).1 := by
    cases tree with
    | leaf old =>
        by_cases same : old.key = fresh.key
        · simp [insertTree, same, Canonical]
        · simpa [insertTree, same] using split_leaf_canonical path old fresh
    | branch pfx terminal children =>
        by_cases split : (common pfx (fresh.key.drop path.length)).length < pfx.length
        · simpa only [insertTree, if_pos split] using split_branch_canonical path pfx terminal children fresh canonical
        · dsimp only [insertTree]
          rw [if_neg split]
          cases (fresh.key.drop path.length).drop pfx.length with
          | nil => exact ⟨Nat.le_trans canonical.1 (alternatives_terminal terminal children fresh), canonical.2⟩
          | cons byte rest => exact ⟨Nat.le_trans canonical.1 (insert_forest_alternatives (path ++ pfx) fresh children byte terminal),
              insert_forest_canonical (path ++ pfx) fresh children byte canonical.2⟩
  theorem insert_forest_canonical (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8)
      (canonical : CanonicalForest forest) : CanonicalForest (insertForest path fresh forest target).1 := by
    cases forest with
    | nil => exact ⟨trivial, trivial⟩
    | cons byte child tail =>
        by_cases same : target = byte
        · subst target
          rw [insertForest, if_pos rfl]
          exact ⟨insert_tree_canonical _ _ _ canonical.1, canonical.2⟩
        · by_cases less : target < byte
          · rw [insertForest, if_neg same, if_pos less]
            exact ⟨trivial, canonical⟩
          · rw [insertForest, if_neg same, if_neg less]
            exact ⟨canonical.1, insert_forest_canonical path fresh tail target canonical.2⟩
end

theorem linear_put_left (query key : Key) (value : Value) (old fresh tail : List Entry)
    (changed : linearLookup query fresh = put key value (fun q => linearLookup q old) query) :
    linearLookup query (fresh ++ tail) = put key value (fun q => linearLookup q (old ++ tail)) query := by
  rw [linear_append, changed]
  by_cases hit : query = key <;> simp [put, hit, linear_append]

theorem linear_put_right (query key : Key) (value : Value) (head old fresh : List Entry)
    (absent : linearLookup key head = none)
    (changed : linearLookup query fresh = put key value (fun q => linearLookup q old) query) :
    linearLookup query (head ++ fresh) = put key value (fun q => linearLookup q (head ++ old)) query := by
  rw [linear_append, changed]
  by_cases hit : query = key
  · subst query
    simp [put, absent]
  · simp [put, hit, linear_append]

theorem linear_replace_terminal (query : Key) (fresh : Entry) (terminal : Option Entry) (tail : List Entry)
    (sameKey : ∀ entry, terminal = some entry → entry.key = fresh.key) :
    linearLookup query (fresh :: tail) =
      put fresh.key fresh.value (fun q => linearLookup q (terminal.toList ++ tail)) query := by
  cases terminal with
  | none => rfl
  | some old =>
      by_cases hit : query = fresh.key <;> simp [linearLookup, put, sameKey old rfl, hit]

mutual
  theorem insert_tree_entries (path : Key) (fresh : Entry) (tree : Tree) (query : Key)
      (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
      linearLookup query (entries (insertTree path fresh tree).1) =
        put fresh.key fresh.value (fun q => linearLookup q (entries tree)) query := by
    cases tree with
    | leaf old =>
        by_cases same : old.key = fresh.key
        · by_cases hit : query = fresh.key <;> simp [insertTree, same, entries, linearLookup, put, hit]
        · simpa only [insertTree, if_neg same, entries, linearLookup, singleton, put] using
            split_leaf_entries_refine path old fresh query valid freshValid same
    | branch pfx terminal children =>
        by_cases split : (common pfx (fresh.key.drop path.length)).length < pfx.length
        · simpa only [insertTree, if_pos split] using
            split_branch_entries_refine path pfx terminal children fresh query valid freshValid split
        · have key := matching_key_decomposition path pfx fresh.key freshValid split
          dsimp only [insertTree]
          rw [if_neg split]
          generalize suffixEq : (fresh.key.drop path.length).drop pfx.length = suffix
          rw [suffixEq] at key
          cases suffix with
          | nil =>
              apply linear_replace_terminal query fresh terminal (forestEntries children)
              intro entry present
              exact (valid.1 entry present).trans (by simpa using key.symm)
          | cons byte rest =>
              have freshChild : HasPrefix ((path ++ pfx) ++ [byte]) fresh.key :=
                ⟨rest, by simpa [List.append_assoc] using key⟩
              have terminalAbsent : linearLookup fresh.key terminal.toList = none := by
                cases terminal with
                | none => rfl
                | some entry => simp [linearLookup, valid.1 entry rfl, key]
              exact linear_put_right query fresh.key fresh.value terminal.toList _ _ terminalAbsent
                (insert_forest_entries (path ++ pfx) fresh children byte query valid.2 ordered freshChild)
  theorem insert_forest_entries (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8) (query : Key)
      (valid : ValidForest path forest) (ordered : OrderedForest forest)
      (freshValid : HasPrefix (path ++ [target]) fresh.key) :
      linearLookup query (forestEntries (insertForest path fresh forest target).1) =
        put fresh.key fresh.value (fun q => linearLookup q (forestEntries forest)) query := by
    cases forest with
    | nil => rfl
    | cons byte child tail =>
        by_cases same : target = byte
        · subst target
          rw [insertForest, if_pos rfl]
          exact linear_put_left query fresh.key fresh.value _ _ (forestEntries tail)
            (insert_tree_entries (path ++ [byte]) fresh child query valid.1 ordered.1 freshValid)
        · by_cases less : target < byte
          · rw [insertForest, if_neg same, if_pos less]
            rfl
          · rw [insertForest, if_neg same, if_neg less]
            have childAbsent : linearLookup fresh.key (entries child) = none := by
              apply linear_outside_prefix (path ++ [byte]) fresh.key _ (valid_tree_prefix _ _ valid.1)
              obtain ⟨suffix, freshKey⟩ := freshValid
              simpa [freshKey, List.append_assoc] using different_edge_prefix path suffix byte target (Ne.symm same)
            exact linear_put_right query fresh.key fresh.value (entries child) _ _ childAbsent
              (insert_forest_entries path fresh tail target query valid.2.1 ordered.2.1 freshValid)
end

def putRoot (fresh : Entry) : Option Tree → Tree × Bool
  | none => (.leaf fresh, true)
  | some tree => insertTree [] fresh tree

theorem put_root_valid (fresh : Entry) (root : Option Tree) (valid : ValidOptional [] root)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) : Valid [] (putRoot fresh root).1 := by
  cases root with
  | none => exact ⟨fresh.key, rfl⟩
  | some tree => exact insert_tree_valid [] fresh tree valid (ordered tree rfl) ⟨fresh.key, rfl⟩

theorem put_root_entries (fresh : Entry) (root : Option Tree) (query : Key) (valid : ValidOptional [] root)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) :
    linearLookup query (entries (putRoot fresh root).1) =
      put fresh.key fresh.value (fun q => linearLookup q (optionalEntries root)) query := by
  cases root with
  | none => rfl
  | some tree => exact insert_tree_entries [] fresh tree query valid (ordered tree rfl) ⟨fresh.key, rfl⟩

theorem put_root_lookup (fresh : Entry) (root : Option Tree) (query : Key) (valid : ValidOptional [] root)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) :
    treeLookup query (putRoot fresh root).1 query =
      if query = fresh.key then some fresh.value else optionalLookup query root := by
  rw [root_lookup_refines _ _ (put_root_valid fresh root valid ordered), put_root_entries fresh root query valid ordered]
  simp only [put, optional_lookup_refines query root valid]

theorem matching_strip (pfx remaining : Key)
    (matching : ¬ (common pfx remaining).length < pfx.length) :
    strip pfx remaining = some (remaining.drop pfx.length) := by
  have shared : common pfx remaining = pfx := by
    apply common_full_left
    have bound := (common_bounds pfx remaining).1
    omega
  have valid : HasPrefix pfx remaining := by simpa only [shared] using common_right pfx remaining
  exact (strip_some_iff _ _ _).mpr (prefix_drop pfx remaining valid)

theorem tree_lookup_dropped_key (path key : Key) (tree : Tree) (valid : Valid path tree)
    (keyValid : HasPrefix path key) : treeLookup key tree (key.drop path.length) = linearLookup key (entries tree) := by
  simpa only [← prefix_drop path key keyValid] using tree_lookup_refines path tree (key.drop path.length) valid

mutual
  theorem insert_tree_flag (path : Key) (fresh : Entry) (tree : Tree)
      (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
      (insertTree path fresh tree).2 = (linearLookup fresh.key (entries tree)).isNone := by
    rw [← tree_lookup_dropped_key path fresh.key tree valid freshValid]
    cases tree with
    | leaf old =>
        by_cases same : old.key = fresh.key
        · simp [insertTree, treeLookup, same]
        · simp [insertTree, treeLookup, same, Ne.symm same]
    | branch pfx terminal children =>
        by_cases split : (common pfx (fresh.key.drop path.length)).length < pfx.length
        · have absent : strip pfx (fresh.key.drop path.length) = none :=
            (strip_none_iff _ _).mpr (proper_common_excludes_prefix pfx _ split)
          simp [insertTree, split, treeLookup, absent]
        · have key := matching_key_decomposition path pfx fresh.key freshValid split
          dsimp only [insertTree, treeLookup]
          rw [if_neg split, matching_strip pfx _ split]
          generalize suffixEq : (fresh.key.drop path.length).drop pfx.length = suffix
          rw [suffixEq] at key
          cases suffix with
          | nil => cases terminal <;> rfl
          | cons byte rest =>
              have childFresh : HasPrefix ((path ++ pfx) ++ [byte]) fresh.key :=
                ⟨rest, by simpa [List.append_assoc] using key⟩
              have routed : forestLookup fresh.key children byte rest = linearLookup fresh.key (forestEntries children) := by
                simpa only [← key] using forest_lookup_refines (path ++ pfx) children byte rest valid.2
              change (insertForest (path ++ pfx) fresh children byte).2 =
                (forestLookup fresh.key children byte rest).isNone
              rw [routed]
              exact insert_forest_flag (path ++ pfx) fresh children byte valid.2 ordered childFresh
  theorem insert_forest_flag (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8)
      (valid : ValidForest path forest) (ordered : OrderedForest forest)
      (freshValid : HasPrefix (path ++ [target]) fresh.key) :
      (insertForest path fresh forest target).2 = (linearLookup fresh.key (forestEntries forest)).isNone := by
    cases forest with
    | nil => rfl
    | cons byte child tail =>
        obtain ⟨suffix, freshKey⟩ := freshValid
        have key : fresh.key = path ++ target :: suffix := by simpa [List.append_assoc] using freshKey
        by_cases same : target = byte
        · subst target
          have tailAbsent : linearLookup fresh.key (forestEntries tail) = none := by
            simpa only [← key] using forest_missing_route path suffix byte tail valid.2.1 valid.2.2
          rw [insertForest, if_pos rfl, forestEntries, linear_append, tailAbsent]
          simpa using insert_tree_flag (path ++ [byte]) fresh child valid.1 ordered.1 ⟨suffix, freshKey⟩
        · by_cases less : target < byte
          · have missing : MissingEdge target (.cons byte child tail) :=
              above_missing _ _ ⟨less, above_trans target byte tail less ordered.2.2⟩
            have absent : linearLookup fresh.key (forestEntries (.cons byte child tail)) = none := by
              simpa only [← key] using forest_missing_route path suffix target (.cons byte child tail) valid missing
            simp [insertForest, same, less, absent]
          · have childAbsent : linearLookup fresh.key (entries child) = none := by
              apply linear_outside_prefix (path ++ [byte]) fresh.key _ (valid_tree_prefix _ _ valid.1)
              simpa only [key] using different_edge_prefix path suffix byte target (Ne.symm same)
            rw [insertForest, if_neg same, if_neg less, forestEntries, linear_append, childAbsent]
            simpa using insert_forest_flag path fresh tail target valid.2.1 ordered.2.1 ⟨suffix, freshKey⟩
end

theorem put_root_flag (fresh : Entry) (root : Option Tree) (valid : ValidOptional [] root)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) :
    (putRoot fresh root).2 = (optionalLookup fresh.key root).isNone := by
  cases root with
  | none => rfl
  | some tree =>
      rw [optional_lookup_refines fresh.key _ valid]
      exact insert_tree_flag [] fresh tree valid (ordered tree rfl) ⟨fresh.key, rfl⟩

end Kv9.Radix
