import InsertWordSplits

set_option autoImplicit false

namespace Kv9.Radix

def selectWordInsert (depth : USize) (fresh : Entry) : Tree → Option InsertAction
  | .leaf old =>
      if old.key = fresh.key then some .replaceLeaf
      else if depth.toNat ≤ old.key.length ∧ depth.toNat ≤ fresh.key.length then
        some (.splitLeaf (common (old.key.drop depth.toNat) (fresh.key.drop depth.toNat)).length)
      else none
  | .branch pfx _ children =>
      if depth.toNat ≤ fresh.key.length then
        let sharedLength := (common pfx (fresh.key.drop depth.toNat)).length
        if sharedLength < pfx.length then some (.splitBranch sharedLength)
        else
          let endOffset := depth + USize.ofNat sharedLength
          match fresh.key[endOffset.toNat]? with
          | none => some .terminal
          | some byte => match edgeSearch byte children with
            | .hit index => some (.descend index (endOffset + 1).toNat)
            | .miss index => some (.addEdge index)
      else none

def executeWordInsert (depth : USize) (fresh : Entry) (tree : Tree) : InsertAction → InsertStep
  | .replaceLeaf => match tree with
    | .leaf _ => .done (.leaf fresh) false
    | _ => .failed
  | .splitLeaf sharedLength => match tree with
    | .leaf old => match splitLeafWordAt depth sharedLength old fresh with
      | none => .failed
      | some result => .done result true
    | _ => .failed
  | .splitBranch sharedLength => match tree with
    | .branch pfx terminal children => match splitBranchWordAt depth sharedLength pfx terminal children fresh with
      | none => .failed
      | some result => .done result true
    | _ => .failed
  | .terminal => match tree with
    | .branch pfx terminal children => .done (.branch pfx (some fresh) children) terminal.isNone
    | _ => .failed
  | .addEdge index => match tree with
    | .branch pfx terminal children => match fresh.key[(depth + USize.ofNat pfx.length).toNat]? with
      | none => .failed
      | some byte =>
          if index ≤ edgeCount children then .done (.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)) true
          else .failed
    | _ => .failed
  | .descend index nextDepth => executeInsert depth.toNat fresh tree (.descend index nextDepth)

def stepWordInsert (depth : USize) (fresh : Entry) (tree : Tree) : InsertStep :=
  match selectWordInsert depth fresh tree with
  | none => .failed
  | some action => executeWordInsert depth fresh tree action

theorem select_word_insert_refines (depth : USize) (fresh : Entry) (tree : Tree)
    (bounded : depth.toNat ≤ fresh.key.length) (fits : fresh.key.length < USize.size) :
    selectWordInsert depth fresh tree = selectInsert depth.toNat fresh tree := by
  cases tree with
  | leaf old => rfl
  | branch pfx terminal children =>
      have within := (common_cut_bounds fresh.key pfx depth.toNat bounded).2.1
      have addition := (word_cut_exact depth _ fresh.key.length within fits).2
      rw [selectWordInsert, selectInsert, if_pos bounded, if_pos bounded]
      by_cases proper : (common pfx (fresh.key.drop depth.toNat)).length < pfx.length
      · rw [if_pos proper, if_pos proper]
      · rw [if_neg proper, if_neg proper]
        simp only [addition]
        cases access : fresh.key[depth.toNat + (common pfx (fresh.key.drop depth.toNat)).length]? with
        | none => rfl
        | some byte =>
            have bound := (byte_descent_bounds fresh.key depth.toNat _ byte access).1
            have next : (depth + USize.ofNat (common pfx (fresh.key.drop depth.toNat)).length + 1).toNat =
                depth.toNat + (common pfx (fresh.key.drop depth.toNat)).length + 1 := by
              rw [word_edge_offset_exact _ fresh.key (by simpa only [addition] using bound) fits, addition]
            simp only [next]
            cases edgeSearch byte children <;> rfl

theorem step_word_insert_refines (path : Key) (depth : USize) (fresh : Entry) (tree : Tree)
    (aligned : depth.toNat = path.length) (valid : Valid path tree) (freshValid : HasPrefix path fresh.key)
    (buffers : RoutingBuffersFit tree) (fits : fresh.key.length < USize.size) :
    stepWordInsert depth fresh tree = stepInsert depth.toNat fresh tree := by
  have depthBound := prefix_length_bound path fresh.key freshValid
  rw [stepWordInsert, select_word_insert_refines depth fresh tree (by simpa only [aligned] using depthBound) fits]
  rw [aligned]
  cases tree with
  | leaf old =>
      by_cases same : old.key = fresh.key
      · simp [selectInsert, stepInsert, same, executeWordInsert, executeInsert]
      · have oldBound := prefix_length_bound path old.key valid
        have within := (leaf_split_cut_bounds path old fresh valid freshValid).2.1
        have split := split_leaf_word_at_refines depth
          (common (old.key.drop path.length) (fresh.key.drop path.length)).length old fresh
          (by simpa only [aligned] using within) buffers
        simp [selectInsert, stepInsert, same, oldBound, depthBound, executeWordInsert, executeInsert, split, aligned]
        cases splitLeafAt path.length (common (old.key.drop path.length) (fresh.key.drop path.length)).length old fresh <;> rfl
  | branch pfx terminal children =>
      by_cases proper : (common pfx (fresh.key.drop path.length)).length < pfx.length
      · have within := (common_cut_bounds fresh.key pfx path.length depthBound).2.1
        have split := split_branch_word_at_refines depth (common pfx (fresh.key.drop path.length)).length
          pfx terminal children fresh proper buffers.1 (by simpa only [aligned] using within) fits
        simp [selectInsert, stepInsert, depthBound, proper, executeWordInsert, executeInsert, split, aligned]
        cases splitBranchAt path.length (common pfx (fresh.key.drop path.length)).length pfx terminal children fresh <;> rfl
      · have full := (matching_prefix_bounds fresh.key pfx path.length depthBound proper).1
        have within := (matching_prefix_bounds fresh.key pfx path.length depthBound proper).2
        have addition := word_prefix_offset_exact depth pfx fresh.key (by simpa only [aligned] using within) fits
        simp only [selectInsert, stepInsert, if_pos depthBound, full, Nat.lt_irrefl, if_false]
        cases access : fresh.key[path.length + pfx.length]? with
        | none => rfl
        | some byte =>
            dsimp only
            cases search : edgeSearch byte children with
            | hit index => simp only [executeWordInsert, aligned]
            | miss index => simp only [executeWordInsert, executeInsert, addition, aligned, access]

end Kv9.Radix
