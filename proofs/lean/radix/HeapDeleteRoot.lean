import HeapLookup

set_option autoImplicit false

namespace Kv9.Radix

-- root.take moves its owning token to current, without cloning it. The map's
-- root is empty during the loop. Returning the final root moves that token
-- back into the map. Native size-field arithmetic is a separate obligation.
def eraseHeap (heap : NodeHeap) (root : Option NodeId) (others : List NodeId) (query : Key) :
    Option (NodeHeap × Option NodeId × Bool) := do
  let found ← lookupHeapRoot heap root query
  if found = none then some (heap, root, false)
  else match root with
    | none => none
    | some focus => (deleteOwnedLoop heap focus others query 0 []).map (fun (next, address) => (next, address, true))

theorem erase_heap_absent_identity (heap : NodeHeap) (root : Option NodeId) (others : List NodeId) (query : Key)
    (absent : lookupHeapRoot heap root query = some none) : eraseHeap heap root others query = some (heap, root, false) := by
  simp [eraseHeap, absent]

theorem erase_heap_refines (heap : NodeHeap) (root : Option NodeId) (others : List NodeId) (query : Key) (tree : Option Tree)
    (owned : HeapOwned heap (root.toList ++ others)) (rep : RootRep heap root tree) (good : Good tree) :
    ∃ next address, eraseHeap heap root others query = some (next, address, (optionalLookup query tree).isSome) ∧
      OwnedResult heap others (eraseRoot query tree) next address := by
  have lookup := lookup_heap_root_refines heap root query tree rep (fun value same => (good.2 value same).2)
  cases root with
  | none =>
      cases tree with
      | some value => cases rep
      | none => exact ⟨heap, none, rfl, ⟨owned, trivial, fun _ _ _ original => original⟩⟩
  | some focus =>
      cases tree with
      | none => cases rep
      | some value =>
          by_cases absent : treeLookup query value query = none
          · refine ⟨heap, some focus, ?_, ?_⟩
            · simp [eraseHeap, lookup, optionalLookup, absent]
            · simpa only [eraseRoot, if_pos absent] using owned_result_identity heap focus others value owned rep
          · have present : treeLookup query value (query.drop 0) ≠ none := by simpa only [List.drop_zero] using absent
            have tokens : HeapOwned heap (focus :: (deleteFrameRoots [] ++ others)) := by simpa [deleteFrameRoots] using owned
            obtain ⟨next, address, completed, result⟩ := delete_owned_loop_refines heap focus others query 0 [] [] value
              tokens .nil rep (good.2 value rfl).2 (Nat.zero_le _) trivial present
            refine ⟨next, address, ?_, ?_⟩
            · simp [eraseHeap, lookup, optionalLookup, absent, completed, Option.isSome_iff_ne_none]
            · simpa only [eraseRoot, if_neg absent, unwindDelete, List.drop_zero] using result

theorem erase_heap_completed (heap next : NodeHeap) (root address : Option NodeId) (others : List NodeId)
    (query : Key) (tree : Option Tree) (removed : Bool)
    (owned : HeapOwned heap (root.toList ++ others)) (rep : RootRep heap root tree) (good : Good tree)
    (completed : eraseHeap heap root others query = some (next, address, removed)) :
    OwnedResult heap others (eraseRoot query tree) next address ∧ removed = (optionalLookup query tree).isSome := by
  obtain ⟨actual, actualRoot, result, correct⟩ := erase_heap_refines heap root others query tree owned rep good
  have same := Option.some.inj (result.symm.trans completed)
  rcases Prod.mk.inj same with ⟨rfl, rest⟩
  rcases Prod.mk.inj rest with ⟨rfl, rfl⟩
  exact ⟨correct, rfl⟩

theorem erase_heap_observables (heap next : NodeHeap) (root address : Option NodeId) (others : List NodeId)
    (query : Key) (tree : Option Tree) (removed : Bool)
    (owned : HeapOwned heap (root.toList ++ others)) (rep : RootRep heap root tree) (good : Good tree)
    (completed : eraseHeap heap root others query = some (next, address, removed)) :
    ∃ result, RootRep next address result ∧ Good result ∧
      (∀ key, optionalLookup key result = if key = query then none else optionalLookup key tree) ∧
      removed = (optionalLookup query tree).isSome ∧
      (optionalEntries result).length + (if removed then 1 else 0) = (optionalEntries tree).length ∧
      HeapOwned next (address.toList ++ others) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨correct, flag⟩ := erase_heap_completed heap next root address others query tree removed owned rep good completed
  refine ⟨eraseRoot query tree, correct.represented, erase_root_good query tree good,
    fun key => erase_root_lookup key query tree good.1, flag, ?_, correct.owned, correct.saved⟩
  have count := erase_root_cardinality query tree good.1
  rw [flag]
  cases found : optionalLookup query tree <;> simpa only [found, Option.isSome_none, Option.isSome_some,
    Bool.false_eq_true, if_false, if_true, reduceCtorEq] using count

end Kv9.Radix
