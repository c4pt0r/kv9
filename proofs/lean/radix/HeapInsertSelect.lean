import HeapSplitBranch

set_option autoImplicit false

namespace Kv9.Radix

-- The ordinary sorted-slice search contract has a unique result. This
-- executable label-only model represents that result; it is not a proof of
-- the standard library's binary-search implementation or running time.
def storedEdgeSearch (target : UInt8) : List StoredEdge → EdgeSearchResult
  | [] => .miss 0
  | (byte, _) :: tail =>
      if target = byte then .hit 0 else if target < byte then .miss 0 else bumpSearch (storedEdgeSearch target tail)

theorem stored_edge_search_refines (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (target : UInt8) (rep : EdgesRep heap edges children) : storedEdgeSearch target edges = edgeSearch target children := by
  cases rep with
  | nil => rfl
  | cons _ _ tail _ rest _ remaining =>
      simp only [storedEdgeSearch, edgeSearch, stored_edge_search_refines heap tail rest target remaining]

def selectStoredInsert (depth : Nat) (fresh : Entry) : StoredNode → Option InsertAction
  | .leaf old =>
      if old.key = fresh.key then some .replaceLeaf
      else if depth ≤ old.key.length ∧ depth ≤ fresh.key.length then
        some (.splitLeaf (common (old.key.drop depth) (fresh.key.drop depth)).length)
      else none
  | .branch pfx _ edges =>
      if depth ≤ fresh.key.length then
        let sharedLength := (common pfx (fresh.key.drop depth)).length
        if sharedLength < pfx.length then some (.splitBranch sharedLength)
        else match fresh.key[depth + sharedLength]? with
          | none => some .terminal
          | some byte => match storedEdgeSearch byte edges with
            | .hit index => some (.descend index (depth + sharedLength + 1))
            | .miss index => some (.addEdge index)
      else none

def selectPlaceInsert (state : MutHeap) (depth : Nat) (fresh : Entry) : Option InsertAction := do
  let focus ← placeTarget state
  let node ← heapRead state.heap focus
  selectStoredInsert depth fresh node

theorem select_place_insert_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (depth : Nat) (fresh : Entry)
    (inv : MutationInvariant state others focus tree frames) :
    selectPlaceInsert state depth fresh = selectInsert depth fresh tree := by
  have target := mutation_invariant_target state others focus tree frames inv
  cases inv.focused with
  | leaf _ entry read => simp [selectPlaceInsert, target, read, selectStoredInsert, selectInsert]
  | branch _ pfx terminal edges children read descendants =>
      simp only [selectPlaceInsert, target, read, bind, Option.bind_some, selectStoredInsert, selectInsert,
        stored_edge_search_refines state.heap edges children _ descendants]
      rfl

theorem select_stored_descend_bounds (depth : Nat) (fresh : Entry) (node : StoredNode) (index nextDepth : Nat)
    (selected : selectStoredInsert depth fresh node = some (.descend index nextDepth)) :
    nextDepth ≤ fresh.key.length ∧ depth < nextDepth ∧ fresh.key.length - nextDepth < fresh.key.length - depth := by
  cases node with
  | leaf old =>
      by_cases same : old.key = fresh.key
      · simp [selectStoredInsert, same] at selected
      · by_cases bounded : depth ≤ old.key.length ∧ depth ≤ fresh.key.length
        · rw [selectStoredInsert, if_neg same, if_pos bounded] at selected
          simp at selected
        · rw [selectStoredInsert, if_neg same, if_neg bounded] at selected
          contradiction
  | branch pfx terminal edges =>
      by_cases bounded : depth ≤ fresh.key.length
      · by_cases proper : (common pfx (fresh.key.drop depth)).length < pfx.length
        · simp [selectStoredInsert, bounded, proper] at selected
        · cases access : fresh.key[depth + (common pfx (fresh.key.drop depth)).length]? with
          | none => simp [selectStoredInsert, bounded, proper, access] at selected
          | some byte =>
              cases search : storedEdgeSearch byte edges with
              | miss slot => simp [selectStoredInsert, bounded, proper, access, search] at selected
              | hit slot =>
                  have same : slot = index ∧ depth + (common pfx (fresh.key.drop depth)).length + 1 = nextDepth := by
                    simpa only [selectStoredInsert, if_pos bounded, if_neg proper, access, search,
                      Option.some.injEq, InsertAction.descend.injEq] using selected
                  simpa only [same.2] using byte_descent_bounds fresh.key depth (common pfx (fresh.key.drop depth)).length byte access
      · simp [selectStoredInsert, bounded] at selected

theorem select_place_descend_bounds (state : MutHeap) (depth : Nat) (fresh : Entry) (index nextDepth : Nat)
    (selected : selectPlaceInsert state depth fresh = some (.descend index nextDepth)) :
    nextDepth ≤ fresh.key.length ∧ depth < nextDepth ∧ fresh.key.length - nextDepth < fresh.key.length - depth := by
  cases target : placeTarget state with
  | none => simp [selectPlaceInsert, target] at selected
  | some focus =>
      cases read : heapRead state.heap focus with
      | none => simp [selectPlaceInsert, target, read] at selected
      | some node =>
          exact select_stored_descend_bounds depth fresh node index nextDepth (by simpa [selectPlaceInsert, target, read] using selected)

end Kv9.Radix
