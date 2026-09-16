import SeekPartition

set_option autoImplicit false

namespace Kv9.Radix

inductive SeekStep where
  | failed
  | done (pushes : List CursorTask)
  | down (pushes : List CursorTask) (child : Tree) (nextDepth : Nat)

-- The checks expose Rust's slice/index obligations. Search still uses the
-- unique ordered-search contract, rather than a verified standard library.
def seekIndex (reverse : Bool) (terminal : Option Entry) (children : Forest)
    (endOffset : Nat) (byte : UInt8) : SeekStep :=
  match edgeSearch byte children with
  | .miss index =>
      if index ≤ edgeCount children then .done (seekPush reverse terminal children index false)
      else .failed
  | .hit index =>
      if index < edgeCount children then
        match edgeGet index children with
        | none => .failed
        | some (_, child) => .down (seekPush reverse terminal children index true) child (endOffset + 1)
      else .failed

def stepSeek (depth : Nat) (query : Key) (inclusive reverse : Bool) : Tree → SeekStep
  | .leaf entry => .done (if accepts query inclusive reverse entry then [.entry entry] else [])
  | .branch pfx terminal children =>
      if depth ≤ query.length then
        let suffix := query.drop depth
        let sharedLength := (common pfx suffix).length
        if sharedLength < pfx.length then
          match mismatchAt pfx suffix with
          | none => .failed
          | some greater => .done (if greater != reverse then [.node (.branch pfx terminal children)] else [])
        else
          let endOffset := depth + sharedLength
          match query[endOffset]? with
          | none => .done (endpointPush inclusive reverse terminal children)
          | some byte => seekIndex reverse terminal children endOffset byte
      else .failed

def SeekStepCorrect (path query : Key) (inclusive reverse : Bool) (tree : Tree) : SeekStep → Prop
  | .failed => False
  | .done pushes => pushes.reverse = seekTree path query inclusive reverse tree
  | .down pushes child nextDepth =>
      ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath query ∧
        nextDepth = childPath.length ∧
        seekTree path query inclusive reverse tree =
          seekTree childPath query inclusive reverse child ++ pushes.reverse

theorem seek_index_down_smaller (reverse : Bool) (pfx : Key) (terminal : Option Entry)
    (children : Forest) (endOffset : Nat) (byte : UInt8) (pushes : List CursorTask)
    (child : Tree) (nextDepth : Nat)
    (down : seekIndex reverse terminal children endOffset byte = .down pushes child nextDepth) :
    treeWork child < treeWork (.branch pfx terminal children) := by
  cases search : edgeSearch byte children with
  | miss index =>
      by_cases bound : index ≤ edgeCount children <;> simp [seekIndex, search, bound] at down
  | hit index =>
      by_cases bound : index < edgeCount children
      · cases access : edgeGet index children with
        | none => simp [seekIndex, search, bound, access] at down
        | some pair =>
            obtain ⟨storedByte, node⟩ := pair
            simp only [seekIndex, search, if_pos bound, access, SeekStep.down.injEq] at down
            have smaller := edge_get_work children index storedByte node access
            rw [down.2.1] at smaller
            simp only [treeWork]
            omega
      · simp [seekIndex, search, bound] at down

theorem step_seek_down_smaller (depth : Nat) (query : Key) (inclusive reverse : Bool)
    (tree : Tree) (pushes : List CursorTask) (child : Tree) (nextDepth : Nat)
    (down : stepSeek depth query inclusive reverse tree = .down pushes child nextDepth) :
    treeWork child < treeWork tree := by
  cases tree with
  | leaf entry => simp [stepSeek] at down
  | branch pfx terminal children =>
      by_cases bound : depth ≤ query.length
      · by_cases proper : (common pfx (query.drop depth)).length < pfx.length
        · cases mismatch : mismatchAt pfx (query.drop depth) <;> simp [stepSeek, bound, proper, mismatch] at down
        · cases access : query[depth + (common pfx (query.drop depth)).length]? with
          | none => simp [stepSeek, bound, proper, access] at down
          | some byte =>
              apply seek_index_down_smaller reverse pfx terminal children _ byte pushes child nextDepth
              simpa only [stepSeek, if_pos bound, if_neg proper, access] using down
      · simp [stepSeek, bound] at down

theorem step_seek_refines (path query : Key) (inclusive reverse : Bool) (tree : Tree)
    (valid : Valid path tree) (ordered : OrderedTree tree) (queryValid : HasPrefix path query) :
    SeekStepCorrect path query inclusive reverse tree (stepSeek path.length query inclusive reverse tree) := by
  cases tree with
  | leaf entry =>
      cases accepted : accepts query inclusive reverse entry <;> simp [stepSeek, SeekStepCorrect, seekTree, accepted]
  | branch pfx terminal children =>
      have bound := prefix_length_bound path query queryValid
      by_cases proper : (common pfx (query.drop path.length)).length < pfx.length
      · have mismatch := mismatch_at_refines pfx (query.drop path.length) proper
        simp only [stepSeek, if_pos bound, if_pos proper, mismatch, SeekStepCorrect, seekTree]
        cases (mismatchGreater pfx (query.drop path.length) != reverse) <;> rfl
      · have full := (matching_prefix_bounds query pfx path.length bound proper).1
        have queryKey := matching_key_decomposition path pfx query queryValid proper
        simp only [stepSeek, if_pos bound, full, Nat.lt_irrefl, if_false, absolute_cut_access]
        generalize remaining : (query.drop path.length).drop pfx.length = suffix
        rw [remaining] at queryKey
        cases suffix with
        | nil =>
            simpa only [List.head?_nil, SeekStepCorrect, seekTree, if_neg proper, remaining] using
              endpoint_push_reverse inclusive reverse terminal children
        | cons byte rest =>
            have childPrefix : HasPrefix ((path ++ pfx) ++ [byte]) query :=
              ⟨rest, by simpa [List.append_assoc] using queryKey⟩
            dsimp only [List.head?_cons]
            cases search : edgeSearch byte children with
            | miss index =>
                have indexBound := edge_search_miss_bound byte children index search
                simp only [seekIndex, search, if_pos indexBound, SeekStepCorrect, seekTree, if_neg proper, remaining]
                rw [seek_push_reverse, seek_forest_indexed_miss (path ++ pfx) query inclusive reverse children byte index search]
                simp only [Bool.false_eq_true, ↓reduceIte, Nat.add_zero]
            | hit index =>
                obtain ⟨child, access⟩ := edge_search_hit_get byte children index search
                have indexBound := edge_get_bound index children byte child access
                simp only [seekIndex, search, if_pos indexBound, access, SeekStepCorrect]
                refine ⟨(path ++ pfx) ++ [byte], edge_get_valid (path ++ pfx) children index byte child valid.2 access,
                  edge_get_ordered children index byte child ordered access, childPrefix, ?_, ?_⟩
                · simp only [List.length_append, List.length_singleton]
                · simp only [seekTree, if_neg proper, remaining]
                  rw [seek_forest_indexed_hit (path ++ pfx) query inclusive reverse children byte index child ordered access,
                    seek_push_reverse]
                  simp only [↓reduceIte, List.append_assoc]

end Kv9.Radix
