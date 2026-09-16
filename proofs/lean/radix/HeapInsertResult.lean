import HeapInsertLoop

set_option autoImplicit false

namespace Kv9.Radix

theorem put_heap_completed (heap next : NodeHeap) (root : Option NodeId) (address : NodeId)
    (others : List NodeId) (tree : Option Tree) (fresh : Entry) (inserted : Bool)
    (owned : HeapOwned heap (root.toList ++ others)) (rep : RootRep heap root tree) (good : Good tree)
    (completed : putHeap heap root others fresh = some (next, address, inserted)) :
    HeapOwned next (address :: others) ∧ NodeRep next address (putRoot fresh tree).1 ∧ inserted = (putRoot fresh tree).2 ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨actual, actualAddress, result, nextOwned, nextRep, preserve⟩ := put_heap_refines heap root others tree fresh owned rep good
  have same := Option.some.inj (result.symm.trans completed)
  rcases Prod.mk.inj same with ⟨rfl, rest⟩
  rcases Prod.mk.inj rest with ⟨rfl, rfl⟩
  exact ⟨nextOwned, nextRep, rfl, preserve⟩

-- Observable finite-map behavior of the completed physical graph mutation.
-- The count is mathematical cardinality; its native size field/word bounds
-- are intentionally not inferred from a graph allocation identity.
theorem put_heap_observables (heap next : NodeHeap) (root : Option NodeId) (address : NodeId)
    (others : List NodeId) (tree : Option Tree) (fresh : Entry) (inserted : Bool)
    (owned : HeapOwned heap (root.toList ++ others)) (rep : RootRep heap root tree) (good : Good tree)
    (completed : putHeap heap root others fresh = some (next, address, inserted)) :
    ∃ result, NodeRep next address result ∧ Good (some result) ∧
      (∀ query, treeLookup query result query = if query = fresh.key then some fresh.value else optionalLookup query tree) ∧
      inserted = (optionalLookup fresh.key tree).isNone ∧
      (entries result).length = (optionalEntries tree).length + (if inserted then 1 else 0) ∧
      HeapOwned next (address :: others) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨nextOwned, nextRep, flag, preserve⟩ := put_heap_completed heap next root address others tree fresh inserted owned rep good completed
  refine ⟨(putRoot fresh tree).1, nextRep, put_root_good fresh tree good, ?_, ?_, ?_, nextOwned, preserve⟩
  · intro query
    exact put_root_lookup fresh tree query good.1 (fun value same => (good.2 value same).2)
  · exact flag.trans (put_root_flag fresh tree good.1 (fun value same => (good.2 value same).2))
  · simpa only [flag] using put_root_cardinality fresh tree good

end Kv9.Radix
