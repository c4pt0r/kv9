import History

set_option autoImplicit false

namespace Kv9.Radix

def UniqueKeys (rows : List Entry) : Prop := (rows.map Entry.key).Nodup

theorem key_member_iff_lookup (query : Key) (rows : List Entry) :
    query ∈ rows.map Entry.key ↔ linearLookup query rows ≠ none := by
  induction rows with
  | nil => simp [linearLookup]
  | cons entry tail ih =>
      by_cases same : query = entry.key <;> simp [linearLookup, same, ih]

theorem forest_keys_disjoint (path : Key) (byte : UInt8) (child : Tree) (tail : Forest)
    (childValid : Valid (path ++ [byte]) child) (tailValid : ValidForest path tail)
    (missing : MissingEdge byte tail) :
    ∀ a ∈ (entries child).map Entry.key,
      ∀ b ∈ (forestEntries tail).map Entry.key, a ≠ b := by
  intro a aMember b bMember same
  obtain ⟨entry, entryMember, entryKey⟩ := List.mem_map.mp aMember
  obtain ⟨suffix, key⟩ := valid_tree_prefix _ _ childValid entry entryMember
  have query : b = path ++ byte :: suffix := by
    simpa [List.append_assoc] using same.symm.trans (entryKey.symm.trans key)
  have absent := forest_missing_route path suffix byte tail tailValid missing
  exact (key_member_iff_lookup b _).mp bMember (by simpa [← query] using absent)

mutual
  theorem valid_tree_unique (path : Key) (tree : Tree) (valid : Valid path tree) :
      UniqueKeys (entries tree) := by
    cases tree with
    | leaf entry => simp [UniqueKeys, entries]
    | branch pfx terminal children =>
        have childUnique := valid_forest_unique (path ++ pfx) children valid.2
        cases terminal with
        | none => simpa [UniqueKeys, entries] using childUnique
        | some entry =>
            have absent : linearLookup entry.key (forestEntries children) = none := by
              rw [valid.1 entry rfl]
              exact forest_terminal_absent _ _ valid.2
            have missing : entry.key ∉ (forestEntries children).map Entry.key := by
              intro member
              exact (key_member_iff_lookup _ _).mp member absent
            simpa [UniqueKeys, entries] using And.intro missing childUnique
  theorem valid_forest_unique (path : Key) (forest : Forest) (valid : ValidForest path forest) :
      UniqueKeys (forestEntries forest) := by
    cases forest with
    | nil => simp [UniqueKeys, forestEntries]
    | cons byte child tail =>
        rw [UniqueKeys, forestEntries, List.map_append, List.nodup_append]
        exact ⟨valid_tree_unique _ _ valid.1, valid_forest_unique _ _ valid.2.1,
          forest_keys_disjoint path byte child tail valid.1 valid.2.1 valid.2.2⟩
end

theorem valid_optional_unique (root : Option Tree) (valid : ValidOptional [] root) :
    UniqueKeys (optionalEntries root) := by
  cases root with
  | none => simp [UniqueKeys, optionalEntries]
  | some tree => exact valid_tree_unique [] tree valid

theorem drop_keys_unique (query : Key) (rows : List Entry) (unique : UniqueKeys rows) :
    UniqueKeys (dropKey query rows) := by
  induction rows with
  | nil => simp [dropKey, UniqueKeys]
  | cons entry tail ih =>
      obtain ⟨missing, tailUnique⟩ := (List.nodup_cons.mp unique)
      by_cases same : query = entry.key
      · simpa [drop_cons, same] using ih tailUnique
      · rw [drop_cons, if_neg same, UniqueKeys, List.map_cons, List.nodup_cons]
        refine ⟨?_, ih tailUnique⟩
        intro member
        obtain ⟨e, present, key⟩ := List.mem_map.mp member
        exact missing (List.mem_map.mpr ⟨e, (List.mem_filter.mp present).1, key⟩)

theorem dropped_key_absent (query : Key) (rows : List Entry) :
    query ∉ (dropKey query rows).map Entry.key := by
  intro member
  have hit := (key_member_iff_lookup query _).mp member
  exact hit (by simp [linear_drop_key])

theorem drop_length (query : Key) (rows : List Entry) (unique : UniqueKeys rows) :
    (dropKey query rows).length + (if linearLookup query rows = none then 0 else 1) = rows.length := by
  induction rows with
  | nil => simp [dropKey, linearLookup]
  | cons entry tail ih =>
      obtain ⟨missing, tailUnique⟩ := List.nodup_cons.mp unique
      by_cases same : query = entry.key
      · have absent : linearLookup query tail = none := by
          by_cases absent : linearLookup query tail = none
          · exact absent
          · exact False.elim (missing (by simpa [same] using (key_member_iff_lookup query tail).mpr absent))
        rw [drop_cons, if_pos same, drop_absent query tail absent]
        simp [linearLookup, same]
      · have smaller := ih tailUnique
        simp only [drop_cons, if_neg same, List.length_cons, linearLookup] at *
        omega

theorem put_keys_permutation (fresh : Entry) (root : Option Tree) (good : Good root) :
    ((entries (putRoot fresh root).1).map Entry.key).Perm
      (fresh.key :: (dropKey fresh.key (optionalEntries root)).map Entry.key) := by
  have resultValid := put_root_valid fresh root good.1 (fun tree eq => (good.2 tree eq).2)
  apply (List.perm_ext_iff_of_nodup (valid_tree_unique [] _ resultValid)
    (List.nodup_cons.mpr ⟨dropped_key_absent _ _, drop_keys_unique _ _ (valid_optional_unique root good.1)⟩)).mpr
  intro query
  rw [key_member_iff_lookup, put_root_entries fresh root query good.1 (fun tree eq => (good.2 tree eq).2)]
  simp only [List.mem_cons, key_member_iff_lookup, linear_drop_key, put]
  by_cases same : query = fresh.key <;> simp [same]

theorem put_root_cardinality (fresh : Entry) (root : Option Tree) (good : Good root) :
    (entries (putRoot fresh root).1).length = (optionalEntries root).length +
      (if (putRoot fresh root).2 then 1 else 0) := by
  have perm := (put_keys_permutation fresh root good).length_eq
  simp only [List.length_map, List.length_cons] at perm
  have removed := drop_length fresh.key (optionalEntries root) (valid_optional_unique root good.1)
  rw [put_root_flag fresh root good.1 (fun tree eq => (good.2 tree eq).2),
    optional_lookup_refines fresh.key root good.1]
  cases lookupEq : linearLookup fresh.key (optionalEntries root) <;> simp_all <;> omega

theorem erase_root_cardinality (query : Key) (root : Option Tree) (valid : ValidOptional [] root) :
    (optionalEntries (eraseRoot query root)).length +
      (if optionalLookup query root = none then 0 else 1) = (optionalEntries root).length := by
  rw [erase_root_entries query root valid, optional_lookup_refines query root valid]
  exact drop_length query _ (valid_optional_unique root valid)

-- These Nat fields mirror the Rust control flow, including the empty-root
-- initialization and the no-op absent-delete guard. Machine bounds are separate.
structure Counted where
  root : Option Tree
  size : Nat

def CountedGood (state : Counted) : Prop :=
  Good state.root ∧ state.size = (optionalEntries state.root).length

theorem counted_empty_good : CountedGood ⟨none, 0⟩ := ⟨empty_good, rfl⟩

def countedPut (state : Counted) (fresh : Entry) : Counted × Bool :=
  match state.root with
  | none => (⟨some (.leaf fresh), 1⟩, true)
  | some tree =>
      let result := insertTree [] fresh tree
      (⟨some result.1, state.size + (if result.2 then 1 else 0)⟩, result.2)

def countedErase (state : Counted) (query : Key) : Counted × Bool :=
  if optionalLookup query state.root = none then (state, false)
  else (⟨eraseRoot query state.root, state.size - 1⟩, true)

theorem counted_put_root (state : Counted) (fresh : Entry) :
    (countedPut state fresh).1.root = some (putRoot fresh state.root).1 := by
  cases state with
  | mk root size => cases root <;> rfl

theorem counted_put_good (state : Counted) (fresh : Entry) (good : CountedGood state) :
    CountedGood (countedPut state fresh).1 := by
  refine ⟨?_, ?_⟩
  · rw [counted_put_root]
    exact put_root_good fresh state.root good.1
  · have count := put_root_cardinality fresh state.root good.1
    cases state with
    | mk root size =>
        cases root with
        | none => rfl
        | some tree =>
            have sizeEq : size = (entries tree).length := good.2
            change size + (if (insertTree [] fresh tree).2 then 1 else 0) =
              (entries (insertTree [] fresh tree).1).length
            rw [sizeEq]
            exact count.symm

theorem counted_erase_root (state : Counted) (query : Key) :
    (countedErase state query).1.root = eraseRoot query state.root := by
  by_cases absent : optionalLookup query state.root = none
  · simp only [countedErase, if_pos absent]
    cases rootEq : state.root with
    | none => rfl
    | some tree =>
        have treeAbsent : treeLookup query tree query = none := by
          simpa [optionalLookup, rootEq] using absent
        simp [eraseRoot, treeAbsent]
  · simp [countedErase, absent]

theorem counted_erase_good (state : Counted) (query : Key) (good : CountedGood state) :
    CountedGood (countedErase state query).1 := by
  by_cases absent : optionalLookup query state.root = none
  · simpa [countedErase, absent] using good
  · refine ⟨?_, ?_⟩
    · rw [counted_erase_root]
      exact erase_root_good query state.root good.1
    · have count := erase_root_cardinality query state.root good.1.1
      simp only [if_neg absent] at count
      simp only [countedErase, if_neg absent]
      rw [good.2]
      omega

theorem present_count_positive (state : Counted) (query : Key) (good : CountedGood state)
    (present : optionalLookup query state.root ≠ none) : 0 < state.size := by
  have count := erase_root_cardinality query state.root good.1.1
  simp only [if_neg present] at count
  rw [good.2]
  omega

def countedMutation (state : Counted) : Mutation → Counted
  | .put entry => (countedPut state entry).1
  | .erase key => (countedErase state key).1

def countedRun (state : Counted) (history : List Mutation) : Counted :=
  history.foldl countedMutation state

theorem counted_mutation_good (state : Counted) (op : Mutation) (good : CountedGood state) :
    CountedGood (countedMutation state op) := by
  cases op with
  | put entry => exact counted_put_good state entry good
  | erase key => exact counted_erase_good state key good

theorem counted_history_good (state : Counted) (history : List Mutation) (good : CountedGood state) :
    CountedGood (countedRun state history) := by
  induction history generalizing state with
  | nil => exact good
  | cons op tail ih => exact ih (countedMutation state op) (counted_mutation_good state op good)

theorem counted_history_root (state : Counted) (history : List Mutation) :
    (countedRun state history).root = run state.root history := by
  induction history generalizing state with
  | nil => rfl
  | cons op tail ih =>
      have first : (countedMutation state op).root = applyMutation state.root op := by
        cases op with
        | put entry => exact counted_put_root state entry
        | erase key => exact counted_erase_root state key
      simpa only [countedRun, run, List.foldl_cons, first] using ih (countedMutation state op)

theorem counted_history_size (state : Counted) (history : List Mutation) (good : CountedGood state) :
    (countedRun state history).size = (optionalEntries (run state.root history)).length := by
  rw [← counted_history_root]
  exact (counted_history_good state history good).2

theorem entries_empty_iff (rows : List Entry) :
    rows.length = 0 ↔ ∀ query, linearLookup query rows = none := by
  cases rows with
  | nil => simp [linearLookup]
  | cons entry tail =>
      constructor
      · intro empty
        simp at empty
      · intro empty
        have impossible := empty entry.key
        simp [linearLookup] at impossible

theorem counted_empty_iff (state : Counted) (good : CountedGood state) :
    state.size = 0 ↔ ∀ query, optionalLookup query state.root = none := by
  rw [good.2, entries_empty_iff]
  simp only [optional_lookup_refines _ _ good.1.1]

end Kv9.Radix
