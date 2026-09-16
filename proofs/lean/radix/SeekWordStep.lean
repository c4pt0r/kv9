import SeekLoop

set_option autoImplicit false

namespace Kv9.Radix

def mismatchWordAt (pfx suffix : Key) : Option Bool :=
  let sharedLength := USize.ofNat (common pfx suffix).length
  match pfx[sharedLength.toNat]? with
  | none => none
  | some oldByte => some (match suffix[sharedLength.toNat]? with
    | none => true
    | some byte => decide (byte < oldByte))

theorem mismatch_word_at_refines (pfx suffix : Key) (fits : (common pfx suffix).length < USize.size) :
    mismatchWordAt pfx suffix = mismatchAt pfx suffix := by
  have conversion : (USize.ofNat (common pfx suffix).length).toNat = (common pfx suffix).length :=
    USize.toNat_ofNat_of_lt fits
  simp only [mismatchWordAt, conversion, mismatchAt]
  cases pfx[(common pfx suffix).length]? with
  | none => rfl
  | some byte => cases suffix[(common pfx suffix).length]? <;> rfl

def seekWordPush (reverse : Bool) (terminal : Option Entry) (children : Forest)
    (index : USize) (found : Bool) : List CursorTask :=
  if reverse then terminalTasks terminal ++ forestTasks (edgeTake index.toNat children)
  else (forestTasks (edgeDrop (index + found.toUSize).toNat children)).reverse

theorem seek_word_push_refines (reverse : Bool) (terminal : Option Entry) (children : Forest)
    (index : USize) (found : Bool) (ordered : OrderedForest children)
    (safe : if found then index.toNat < edgeCount children else index.toNat ≤ edgeCount children) :
    seekWordPush reverse terminal children index found = seekPush reverse terminal children index.toNat found := by
  have addition := (seek_after_word_exact index found children ordered safe).1
  simp only [seekWordPush, seekPush, addition]

def seekWordIndex (reverse : Bool) (terminal : Option Entry) (children : Forest)
    (endOffset : USize) (byte : UInt8) : SeekStep :=
  match edgeSearch byte children with
  | .miss index =>
      let wordIndex := USize.ofNat index
      if wordIndex.toNat ≤ edgeCount children then .done (seekWordPush reverse terminal children wordIndex false)
      else .failed
  | .hit index =>
      let wordIndex := USize.ofNat index
      if wordIndex.toNat < edgeCount children then
        match edgeGet wordIndex.toNat children with
        | none => .failed
        | some (_, child) => .down (seekWordPush reverse terminal children wordIndex true) child (endOffset + 1).toNat
      else .failed

theorem seek_word_index_refines (reverse : Bool) (terminal : Option Entry) (children : Forest)
    (endOffset : USize) (byte : UInt8) (ordered : OrderedForest children)
    (nextExact : (endOffset + 1).toNat = endOffset.toNat + 1) :
    seekWordIndex reverse terminal children endOffset byte = seekIndex reverse terminal children endOffset.toNat byte := by
  have forestFits := (ordered_edges_fit children ordered).2
  cases search : edgeSearch byte children with
  | miss index =>
      have bound := edge_search_miss_bound byte children index search
      have conversion : (USize.ofNat index).toNat = index := USize.toNat_ofNat_of_lt (by omega)
      have pushes := seek_word_push_refines reverse terminal children (USize.ofNat index) false ordered
        (by simpa only [Bool.false_eq_true, if_false, conversion] using bound)
      simp only [seekWordIndex, seekIndex, search, conversion, if_pos bound, pushes]
  | hit index =>
      obtain ⟨child, access⟩ := edge_search_hit_get byte children index search
      have bound := edge_get_bound index children byte child access
      have conversion : (USize.ofNat index).toNat = index := USize.toNat_ofNat_of_lt (by omega)
      have pushes := seek_word_push_refines reverse terminal children (USize.ofNat index) true ordered
        (by simpa only [if_true, conversion] using bound)
      simp only [seekWordIndex, seekIndex, search, conversion, if_pos bound, access, pushes, nextExact]

def stepWordSeek (depth : USize) (query : Key) (inclusive reverse : Bool) : Tree → SeekStep
  | .leaf entry => .done (if accepts query inclusive reverse entry then [.entry entry] else [])
  | .branch pfx terminal children =>
      if depth.toNat ≤ query.length then
        let suffix := query.drop depth.toNat
        let sharedLength := USize.ofNat (common pfx suffix).length
        if sharedLength.toNat < pfx.length then
          match mismatchWordAt pfx suffix with
          | none => .failed
          | some greater => .done (if greater != reverse then [.node (.branch pfx terminal children)] else [])
        else
          let endOffset := depth + sharedLength
          match query[endOffset.toNat]? with
          | none => .done (endpointPush inclusive reverse terminal children)
          | some byte => seekWordIndex reverse terminal children endOffset byte
      else .failed

theorem step_word_seek_refines (depth : USize) (query : Key) (inclusive reverse : Bool) (tree : Tree)
    (ordered : OrderedTree tree) (bounded : depth.toNat ≤ query.length) (fits : query.length < USize.size) :
    stepWordSeek depth query inclusive reverse tree = stepSeek depth.toNat query inclusive reverse tree := by
  cases tree with
  | leaf entry => rfl
  | branch pfx terminal children =>
      have within := (common_cut_bounds query pfx depth.toNat bounded).2.1
      have sharedFits : (common pfx (query.drop depth.toNat)).length < USize.size := by omega
      have conversion := (word_cut_exact depth _ query.length within fits).1
      have addition := (word_cut_exact depth _ query.length within fits).2
      simp only [stepWordSeek, stepSeek, if_pos bounded, conversion]
      by_cases proper : (common pfx (query.drop depth.toNat)).length < pfx.length
      · rw [if_pos proper, if_pos proper, mismatch_word_at_refines pfx (query.drop depth.toNat) sharedFits]
        cases mismatchAt pfx (query.drop depth.toNat) <;> rfl
      · rw [if_neg proper, if_neg proper]
        simp only [addition]
        cases access : query[depth.toNat + (common pfx (query.drop depth.toNat)).length]? with
        | none => rfl
        | some byte =>
            have bound := (byte_descent_bounds query depth.toNat _ byte access).1
            have next := word_edge_offset_exact (depth + USize.ofNat (common pfx (query.drop depth.toNat)).length)
              query (by simpa only [addition] using bound) fits
            simpa only [addition] using seek_word_index_refines reverse terminal children _ byte ordered next

theorem seek_word_index_down_smaller (reverse : Bool) (pfx : Key) (terminal : Option Entry)
    (children : Forest) (endOffset : USize) (byte : UInt8) (pushes : List CursorTask)
    (child : Tree) (nextDepth : Nat)
    (down : seekWordIndex reverse terminal children endOffset byte = .down pushes child nextDepth) :
    treeWork child < treeWork (.branch pfx terminal children) := by
  cases search : edgeSearch byte children with
  | miss index =>
      simp only [seekWordIndex, search] at down
      split at down <;> cases down
  | hit index =>
      by_cases bound : (USize.ofNat index).toNat < edgeCount children
      · cases access : edgeGet (USize.ofNat index).toNat children with
        | none => simp only [seekWordIndex, search, if_pos bound, access] at down; cases down
        | some pair =>
            obtain ⟨storedByte, node⟩ := pair
            simp only [seekWordIndex, search, if_pos bound, access, SeekStep.down.injEq] at down
            have smaller := edge_get_work children (USize.ofNat index).toNat storedByte node access
            rw [down.2.1] at smaller
            simp only [treeWork]
            omega
      · simp only [seekWordIndex, search, if_neg bound] at down; cases down

theorem step_word_seek_down_smaller (depth : USize) (query : Key) (inclusive reverse : Bool)
    (tree : Tree) (pushes : List CursorTask) (child : Tree) (nextDepth : Nat)
    (down : stepWordSeek depth query inclusive reverse tree = .down pushes child nextDepth) :
    treeWork child < treeWork tree := by
  cases tree with
  | leaf entry => simp [stepWordSeek] at down
  | branch pfx terminal children =>
      by_cases bound : depth.toNat ≤ query.length
      · by_cases proper : (USize.ofNat (common pfx (query.drop depth.toNat)).length).toNat < pfx.length
        · cases mismatch : mismatchWordAt pfx (query.drop depth.toNat) <;>
            simp only [stepWordSeek, if_pos bound, if_pos proper, mismatch] at down <;> cases down
        · cases access : query[(depth + USize.ofNat (common pfx (query.drop depth.toNat)).length).toNat]? with
          | none => simp only [stepWordSeek, if_pos bound, if_neg proper, access] at down; cases down
          | some byte =>
              apply seek_word_index_down_smaller reverse pfx terminal children _ byte pushes child nextDepth
              simpa only [stepWordSeek, if_pos bound, if_neg proper, access] using down
      · simp only [stepWordSeek, if_neg bound] at down; cases down

end Kv9.Radix
