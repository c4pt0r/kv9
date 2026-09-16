import Normalize

set_option autoImplicit false

namespace Kv9.Radix

def dropKey (query : Key) (rows : List Entry) : List Entry :=
  rows.filter (fun entry => decide (query ≠ entry.key))

theorem linear_none_iff (query : Key) (rows : List Entry) :
    linearLookup query rows = none ↔ ∀ entry ∈ rows, query ≠ entry.key := by
  induction rows with
  | nil => simp [linearLookup]
  | cons entry tail ih =>
      by_cases same : query = entry.key <;> simp [linearLookup, same, ih]

theorem drop_absent (query : Key) (rows : List Entry) (absent : linearLookup query rows = none) :
    dropKey query rows = rows := by
  apply List.filter_eq_self.mpr
  intro entry member
  exact decide_eq_true ((linear_none_iff _ _).mp absent entry member)

theorem drop_append (query : Key) (a b : List Entry) :
    dropKey query (a ++ b) = dropKey query a ++ dropKey query b := by
  simp [dropKey]

theorem drop_cons (query : Key) (entry : Entry) (tail : List Entry) :
    dropKey query (entry :: tail) =
      if query = entry.key then dropKey query tail else entry :: dropKey query tail := by
  by_cases same : query = entry.key <;> simp [dropKey, same]

mutual
  def deleteTree (query : Key) : Tree → Key → Option Tree
    | .leaf _, _ => none
    | .branch pfx terminal children, remaining =>
        match strip pfx remaining with
        | none => some (.branch pfx terminal children)
        | some [] => normalize (.branch pfx none children)
        | some (byte :: suffix) => normalize (.branch pfx terminal (deleteForest query children byte suffix))
  def deleteForest (query : Key) : Forest → UInt8 → Key → Forest
    | .nil, _, _ => .nil
    | .cons byte child tail, target, suffix =>
        if target = byte then
          match deleteTree query child suffix with
          | none => tail
          | some result => .cons byte result tail
        else .cons byte child (deleteForest query tail target suffix)
end

theorem delete_forest_missing (query : Key) (forest : Forest) (target absent : UInt8) (suffix : Key)
    (missing : MissingEdge absent forest) :
    MissingEdge absent (deleteForest query forest target suffix) := by
  cases forest with
  | nil => trivial
  | cons byte child tail =>
      obtain ⟨different, tailMissing⟩ := missing
      by_cases same : target = byte
      · rw [deleteForest, if_pos same]
        cases deleteTree query child suffix with
        | none => exact tailMissing
        | some result => exact ⟨different, tailMissing⟩
      · rw [deleteForest, if_neg same]
        exact ⟨different, delete_forest_missing query tail target absent suffix tailMissing⟩

mutual
  theorem delete_tree_valid (path : Key) (tree : Tree) (query remaining : Key)
      (valid : Valid path tree) : ValidOptional path (deleteTree query tree remaining) := by
    cases tree with
    | leaf entry => trivial
    | branch pfx terminal children =>
        obtain ⟨terminalValid, childrenValid⟩ := valid
        cases h : strip pfx remaining with
        | none =>
            rw [deleteTree, h]
            exact ⟨terminalValid, childrenValid⟩
        | some suffix =>
            cases suffix with
            | nil =>
                rw [deleteTree, h]
                exact normalize_valid _ _ ⟨by simp, childrenValid⟩
            | cons byte rest =>
                rw [deleteTree, h]
                exact normalize_valid _ _ ⟨terminalValid,
                  delete_forest_valid (path ++ pfx) children query byte rest childrenValid⟩
  theorem delete_forest_valid (path : Key) (forest : Forest) (query : Key) (target : UInt8) (suffix : Key)
      (valid : ValidForest path forest) : ValidForest path (deleteForest query forest target suffix) := by
    cases forest with
    | nil => trivial
    | cons byte child tail =>
        obtain ⟨childValid, tailValid, missing⟩ := valid
        by_cases same : target = byte
        · rw [deleteForest, if_pos same]
          have resultValid := delete_tree_valid (path ++ [byte]) child query suffix childValid
          cases h : deleteTree query child suffix with
          | none => exact tailValid
          | some result => exact ⟨by simpa [h, ValidOptional] using resultValid, tailValid, missing⟩
        · rw [deleteForest, if_neg same]
          exact ⟨childValid, delete_forest_valid path tail query target suffix tailValid,
            delete_forest_missing query tail target byte suffix missing⟩
end

mutual
  theorem delete_tree_canonical (tree : Tree) (query remaining : Key) (canonical : Canonical tree) :
      ∀ result, deleteTree query tree remaining = some result → Canonical result := by
    cases tree with
    | leaf entry => simp [deleteTree]
    | branch pfx terminal children =>
        cases h : strip pfx remaining with
        | none =>
            intro result eq
            simp only [deleteTree, h, Option.some.injEq] at eq
            subst result
            exact canonical
        | some suffix =>
            cases suffix with
            | nil =>
                rw [deleteTree, h]
                exact normalize_canonical _ canonical.2
            | cons byte rest =>
                rw [deleteTree, h]
                exact normalize_canonical _ (delete_forest_canonical children query byte rest canonical.2)
  theorem delete_forest_canonical (forest : Forest) (query : Key) (target : UInt8) (suffix : Key)
      (canonical : CanonicalForest forest) : CanonicalForest (deleteForest query forest target suffix) := by
    cases forest with
    | nil => trivial
    | cons byte child tail =>
        by_cases same : target = byte
        · rw [deleteForest, if_pos same]
          cases h : deleteTree query child suffix with
          | none => exact canonical.2
          | some result => exact ⟨delete_tree_canonical child query suffix canonical.1 result h, canonical.2⟩
        · rw [deleteForest, if_neg same]
          exact ⟨canonical.1, delete_forest_canonical tail query target suffix canonical.2⟩
end

theorem rebuilt_forest_entries (byte : UInt8) (replacement : Option Tree) (tail : Forest) :
    forestEntries (match replacement with | none => tail | some child => .cons byte child tail) =
      optionalEntries replacement ++ forestEntries tail := by
  cases replacement <;> simp [optionalEntries, forestEntries]

mutual
  theorem delete_tree_entries (path : Key) (tree : Tree) (remaining : Key)
      (valid : Valid path tree) (present : treeLookup (path ++ remaining) tree remaining ≠ none) :
      optionalEntries (deleteTree (path ++ remaining) tree remaining) =
        dropKey (path ++ remaining) (entries tree) := by
    cases tree with
    | leaf entry =>
        have same : path ++ remaining = entry.key := by
          by_cases same : path ++ remaining = entry.key
          · exact same
          · simp [treeLookup, same] at present
        simp [deleteTree, optionalEntries, entries, dropKey, same]
    | branch pfx terminal children =>
        obtain ⟨terminalValid, childrenValid⟩ := valid
        cases h : strip pfx remaining with
        | none => simp [treeLookup, h] at present
        | some suffix =>
            cases suffix with
            | nil =>
                have remainingEq := (strip_empty_suffix _ _).mp h
                cases terminal with
                | none => simp [treeLookup, h] at present
                | some entry =>
                    rw [deleteTree, h, normalize_entries]
                    change forestEntries children =
                      dropKey (path ++ remaining) (entry :: forestEntries children)
                    have same : path ++ remaining = entry.key := by
                      rw [remainingEq, terminalValid entry rfl]
                    rw [drop_cons, if_pos same, remainingEq]
                    exact (drop_absent _ _ (forest_terminal_absent _ _ childrenValid)).symm
            | cons byte rest =>
                have remainingEq := (strip_some_iff _ _ _).mp h
                have queryEq : path ++ remaining = (path ++ pfx) ++ byte :: rest := by
                  simp [remainingEq, List.append_assoc]
                have childPresent : forestLookup ((path ++ pfx) ++ byte :: rest) children byte rest ≠ none := by
                  simpa [treeLookup, h, queryEq] using present
                have terminalAbsent : linearLookup (path ++ remaining) terminal.toList = none := by
                  cases terminal with
                  | none => rfl
                  | some entry => simp [linearLookup, terminalValid entry rfl, queryEq]
                rw [deleteTree, h, normalize_entries, entries, entries, drop_append,
                  drop_absent _ _ terminalAbsent, queryEq,
                  delete_forest_entries (path ++ pfx) children byte rest childrenValid childPresent]
  theorem delete_forest_entries (path : Key) (forest : Forest) (target : UInt8) (suffix : Key)
      (valid : ValidForest path forest)
      (present : forestLookup (path ++ target :: suffix) forest target suffix ≠ none) :
      forestEntries (deleteForest (path ++ target :: suffix) forest target suffix) =
        dropKey (path ++ target :: suffix) (forestEntries forest) := by
    cases forest with
    | nil => simp [forestLookup] at present
    | cons byte child tail =>
        obtain ⟨childValid, tailValid, missing⟩ := valid
        by_cases same : target = byte
        · subst target
          have childPresent : treeLookup ((path ++ [byte]) ++ suffix) child suffix ≠ none := by
            simpa [forestLookup, List.append_assoc] using present
          have changed := delete_tree_entries (path ++ [byte]) child suffix childValid childPresent
          simp only [List.append_assoc, List.singleton_append] at changed
          rw [deleteForest, if_pos rfl, rebuilt_forest_entries, forestEntries, drop_append,
            drop_absent _ _ (forest_missing_route path suffix byte tail tailValid missing), changed]
        · have tailPresent : forestLookup (path ++ target :: suffix) tail target suffix ≠ none := by
            simpa [forestLookup, same] using present
          have childAbsent := linear_outside_prefix (path ++ [byte]) (path ++ target :: suffix) (entries child)
            (valid_tree_prefix _ _ childValid) (different_edge_prefix path suffix byte target (Ne.symm same))
          rw [deleteForest, if_neg same, forestEntries, forestEntries, drop_append,
            drop_absent _ _ childAbsent, delete_forest_entries path tail target suffix tailValid tailPresent]
end

def eraseRoot (query : Key) : Option Tree → Option Tree
  | none => none
  | some tree => if treeLookup query tree query = none then some tree else deleteTree query tree query

theorem erase_root_valid (query : Key) (root : Option Tree) (valid : ValidOptional [] root) :
    ValidOptional [] (eraseRoot query root) := by
  cases root with
  | none => trivial
  | some tree =>
      by_cases absent : treeLookup query tree query = none
      · simpa [eraseRoot, absent] using valid
      · simpa [eraseRoot, absent] using delete_tree_valid [] tree query query valid

theorem erase_root_entries (query : Key) (root : Option Tree) (valid : ValidOptional [] root) :
    optionalEntries (eraseRoot query root) = dropKey query (optionalEntries root) := by
  cases root with
  | none => rfl
  | some tree =>
      by_cases absent : treeLookup query tree query = none
      · have linearAbsent : linearLookup query (entries tree) = none := by
          rwa [← root_lookup_refines tree query valid]
        simpa [eraseRoot, absent, optionalEntries] using (drop_absent query _ linearAbsent).symm
      · simpa [eraseRoot, absent, optionalEntries] using delete_tree_entries [] tree query valid absent

theorem linear_drop_key (query removed : Key) (rows : List Entry) :
    linearLookup query (dropKey removed rows) =
      if query = removed then none else linearLookup query rows := by
  induction rows with
  | nil => simp [dropKey, linearLookup]
  | cons entry tail ih =>
      by_cases removedSame : removed = entry.key <;>
        by_cases queryRemoved : query = removed <;>
        by_cases queryEntry : query = entry.key <;>
        simp_all [drop_cons, linearLookup]

theorem optional_lookup_refines (query : Key) (root : Option Tree) (valid : ValidOptional [] root) :
    optionalLookup query root = linearLookup query (optionalEntries root) := by
  cases root with
  | none => rfl
  | some tree => exact root_lookup_refines tree query valid

theorem erase_root_lookup (query removed : Key) (root : Option Tree) (valid : ValidOptional [] root) :
    optionalLookup query (eraseRoot removed root) =
      if query = removed then none else optionalLookup query root := by
  rw [optional_lookup_refines _ _ (erase_root_valid removed root valid), erase_root_entries removed root valid,
    linear_drop_key, optional_lookup_refines query root valid]

end Kv9.Radix
