import EdgeSearch

set_option autoImplicit false

namespace Kv9.Radix

-- Indexed vector operations used by the Rust mutation loops, related to the
-- earlier recursive forest model. Search's ordinary library contract remains
-- a premise; these results do not verify the standard-library implementation.
theorem edge_lookup_hit (query suffix : Key) (forest : Forest) (index : Nat)
    (target : UInt8) (child : Tree) (ordered : OrderedForest forest)
    (found : edgeGet index forest = some (target, child)) :
    forestLookup query forest target suffix = treeLookup query child suffix := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, forestLookup]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have less := above_get byte tail n target child ordered.2.2 found
          have different := (byte_less_excludes byte target less).1
          simpa only [forestLookup, if_neg different] using ih tail ordered.2.1 found

theorem forest_present_index (query suffix : Key) (forest : Forest) (target : UInt8)
    (present : forestLookup query forest target suffix ≠ none) :
    ∃ index child, edgeGet index forest = some (target, child) ∧ treeLookup query child suffix ≠ none := by
  cases forest with
  | nil => simp [forestLookup] at present
  | cons byte child tail =>
      by_cases same : target = byte
      · subst target
        exact ⟨0, child, rfl, by simpa [forestLookup] using present⟩
      · have tailPresent : forestLookup query tail target suffix ≠ none := by
          simpa only [forestLookup, if_neg same] using present
        obtain ⟨index, found, access, childPresent⟩ := forest_present_index query suffix tail target tailPresent
        exact ⟨index + 1, found, access, childPresent⟩

theorem forest_present_search (query suffix : Key) (forest : Forest) (target : UInt8)
    (ordered : OrderedForest forest) (present : forestLookup query forest target suffix ≠ none) :
    ∃ index child, edgeSearch target forest = .hit index ∧
      edgeGet index forest = some (target, child) ∧ treeLookup query child suffix ≠ none := by
  obtain ⟨index, child, access, childPresent⟩ := forest_present_index query suffix forest target present
  exact ⟨index, child, edge_search_hit_complete target forest index child ordered access, access, childPresent⟩

def edgeReplace (index : Nat) (replacement : Option Tree) (forest : Forest) : Forest :=
  match replacement with
  | none => edgeRemove index forest
  | some child => edgeSet index child forest

theorem edge_delete_hit (query suffix : Key) (forest : Forest) (index : Nat)
    (target : UInt8) (child : Tree) (ordered : OrderedForest forest)
    (found : edgeGet index forest = some (target, child)) :
    deleteForest query forest target suffix = edgeReplace index (deleteTree query child suffix) forest := by
  induction index generalizing forest with
  | zero =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have pair : byte = target ∧ node = child := by simpa [edgeGet] using found
          rcases pair with ⟨rfl, rfl⟩
          cases result : deleteTree query node suffix <;> simp [deleteForest, result, edgeReplace, edgeRemove, edgeSet]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have less := above_get byte tail n target child ordered.2.2 found
          have different := (byte_less_excludes byte target less).1
          rw [deleteForest, if_neg different, ih tail ordered.2.1 found]
          cases deleteTree query child suffix <;> rfl

theorem detached_restore_refines (forest : Forest) (index : Nat) (byte : UInt8)
    (child : Tree) (replacement : Option Tree) (found : edgeGet index forest = some (byte, child)) :
    (match replacement with
      | none => edgeRemove index forest
      | some result => edgeInsert index byte result (edgeRemove index forest)) = edgeReplace index replacement forest := by
  cases replacement with
  | none => rfl
  | some result => exact edge_remove_restore index forest byte child result found

theorem edge_insert_hit (path : Key) (fresh : Entry) (forest : Forest) (index : Nat)
    (target : UInt8) (child : Tree) (ordered : OrderedForest forest)
    (found : edgeGet index forest = some (target, child)) :
    insertForest path fresh forest target =
      (edgeSet index (insertTree (path ++ [target]) fresh child).1 forest,
        (insertTree (path ++ [target]) fresh child).2) := by
  induction index generalizing forest with
  | zero =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have pair : byte = target ∧ node = child := by simpa [edgeGet] using found
          rcases pair with ⟨rfl, rfl⟩
          simp [insertForest, edgeSet]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have less := above_get byte tail n target child ordered.2.2 found
          obtain ⟨different, notBefore⟩ := byte_less_excludes byte target less
          simp only [insertForest, if_neg different, if_neg notBefore, ih tail ordered.2.1 found, edgeSet]

theorem edge_insert_miss (path : Key) (fresh : Entry) (forest : Forest) (target : UInt8) (index : Nat)
    (miss : edgeSearch target forest = .miss index) :
    insertForest path fresh forest target = (edgeInsert index target (.leaf fresh) forest, true) := by
  cases forest with
  | nil =>
      have eq : index = 0 := by simpa [edgeSearch] using miss.symm
      subst index
      rfl
  | cons byte child tail =>
      by_cases same : target = byte
      · simp [edgeSearch, same] at miss
      · by_cases before : target < byte
        · have eq : index = 0 := by simpa [edgeSearch, same, before] using miss.symm
          subst index
          simp [insertForest, same, before, edgeInsert]
        · cases search : edgeSearch target tail with
          | hit n => simp [edgeSearch, same, before, search, bumpSearch] at miss
          | miss n =>
              have eq : index = n + 1 := by simpa [edgeSearch, same, before, search, bumpSearch] using miss.symm
              subst index
              simp only [insertForest, if_neg same, if_neg before,
                edge_insert_miss path fresh tail target n search, edgeInsert]

end Kv9.Radix
