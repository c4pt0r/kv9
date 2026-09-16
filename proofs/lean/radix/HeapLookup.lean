import HeapDeleteLoop

set_option autoImplicit false

namespace Kv9.Radix

-- Borrowed get traversal: neither the heap nor the owning-root inventory is
-- modified. Outer none models a failed source slice/index or graph assertion;
-- inner none is an ordinary missing key. Returned bytes represent the value
-- observation, not an extra Arc owner.
def lookupHeap (heap : NodeHeap) (focus : NodeId) (query : Key) (depth : Nat) : Option (Option Value) :=
  match heapRead heap focus with
  | none => none
  | some (.leaf entry) => some (if query = entry.key then some entry.value else none)
  | some (.branch pfx terminal edges) =>
      if depth ≤ query.length then
        match strip pfx (query.drop depth) with
        | none => some none
        | some _ => match _access : query[depth + pfx.length]? with
          | none => some (terminal.map Entry.value)
          | some byte => match storedEdgeSearch byte edges with
            | .miss _ => some none
            | .hit index => match edges[index]? with
              | none => none
              | some (_, child) => lookupHeap heap child query (depth + pfx.length + 1)
      else none
termination_by query.length - depth
decreasing_by
  have within := (List.getElem?_eq_some_iff.mp _access).1
  omega

theorem lookup_heap_refines (heap : NodeHeap) (focus : NodeId) (query : Key) (depth : Nat) (tree : Tree)
    (rep : NodeRep heap focus tree) (ordered : OrderedTree tree) (bounded : depth ≤ query.length) :
    lookupHeap heap focus query depth = some (treeLookup query tree (query.drop depth)) := by
  cases tree with
  | leaf entry =>
      have read : heapRead heap focus = some (.leaf entry) := by cases rep; assumption
      rw [lookupHeap, read]
      rfl
  | branch pfx terminal children =>
      obtain ⟨edges, read, descendants⟩ := represented_branch_read heap focus pfx terminal children rep
      cases matched : strip pfx (query.drop depth) with
      | none => rw [lookupHeap, read]; simp [bounded, matched, treeLookup]
      | some suffix =>
          obtain ⟨within, remaining, atEnd⟩ := deletion_offset query pfx suffix depth bounded matched
          cases suffix with
          | nil =>
              have done := atEnd.mpr rfl
              rw [lookupHeap, read]
              simp only [if_pos bounded, treeLookup, matched]
              split
              · rfl
              · rename_i byte actual
                have absent : query[depth + pfx.length]? = none := by simp [done]
                rw [absent] at actual
                contradiction
          | cons byte rest =>
              have access : query[depth + pfx.length]? = some byte := by rw [← drop_head_access, remaining]; rfl
              have nextRemaining : query.drop (depth + pfx.length + 1) = rest := by rw [List.drop_add_one_eq_tail_drop, remaining]; rfl
              have nextBound := (byte_descent_bounds query depth pfx.length byte access).1
              have smaller := (byte_descent_bounds query depth pfx.length byte access).2.2
              have storedSearch := stored_edge_search_refines heap edges children byte descendants
              cases search : edgeSearch byte children with
              | miss index =>
                  have absent : forestLookup query children byte rest = none := by
                    by_cases missing : forestLookup query children byte rest = none
                    · exact missing
                    · obtain ⟨foundIndex, child, hit, _⟩ := forest_present_search query rest children byte ordered missing
                      rw [search] at hit
                      contradiction
                  rw [lookupHeap, read]
                  simp only [if_pos bounded, matched]
                  split
                  · rename_i missing
                    rw [access] at missing
                    contradiction
                  · rename_i actualByte actualAccess
                    have same := Option.some.inj (actualAccess.symm.trans access)
                    subst actualByte
                    simp only [storedSearch, search, treeLookup, matched, absent]
              | hit index =>
                  obtain ⟨childTree, found⟩ := edge_search_hit_get byte children index search
                  obtain ⟨child, actual, childRep⟩ := edges_rep_get_address heap edges children index byte childTree descendants found
                  have childOrdered := edge_get_ordered children index byte childTree ordered found
                  have next := lookup_heap_refines heap child query (depth + pfx.length + 1) childTree childRep childOrdered nextBound
                  have observed := edge_lookup_hit query rest children index byte childTree ordered found
                  rw [lookupHeap, read]
                  simp only [if_pos bounded, matched]
                  split
                  · rename_i missing
                    rw [access] at missing
                    contradiction
                  · rename_i actualByte actualAccess
                    have same := Option.some.inj (actualAccess.symm.trans access)
                    subst actualByte
                    simpa only [storedSearch, search, actual, treeLookup, matched, observed, nextRemaining] using next
termination_by query.length - depth
decreasing_by assumption

def lookupHeapRoot (heap : NodeHeap) (root : Option NodeId) (query : Key) : Option (Option Value) :=
  match root with
  | none => some none
  | some address => lookupHeap heap address query 0

theorem lookup_heap_root_refines (heap : NodeHeap) (root : Option NodeId) (query : Key) (tree : Option Tree)
    (rep : RootRep heap root tree) (ordered : ∀ value, tree = some value → OrderedTree value) :
    lookupHeapRoot heap root query = some (optionalLookup query tree) := by
  cases root with
  | none => cases tree with
    | none => rfl
    | some value => cases rep
  | some address => cases tree with
    | none => cases rep
    | some value => simpa only [lookupHeapRoot, optionalLookup, List.drop_zero] using
        lookup_heap_refines heap address query 0 value rep (ordered value rfl) (Nat.zero_le _)

end Kv9.Radix
