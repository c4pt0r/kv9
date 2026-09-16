import InsertLoop

set_option autoImplicit false

namespace Kv9.Radix

-- Actual additions are evaluated as machine words. Bounds below establish
-- their agreement with the checked absolute-offset split helpers.
def splitLeafWordAt (depth : USize) (sharedLength : Nat) (old fresh : Entry) : Option Tree :=
  let cut := depth + USize.ofNat sharedLength
  if depth.toNat ≤ cut.toNat ∧ cut.toNat ≤ old.key.length then
    leafSplitBytes ((old.key.drop depth.toNat).take (cut.toNat - depth.toNat)) old fresh
      old.key[cut.toNat]? fresh.key[cut.toNat]?
  else none

def splitBranchWordAt (depth : USize) (sharedLength : Nat) (pfx : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) : Option Tree :=
  let sharedWord := USize.ofNat sharedLength
  match pfx[sharedWord.toNat]? with
  | none => none
  | some oldByte =>
      let oldChild := Tree.branch (pfx.drop (sharedWord + 1).toNat) terminal children
      match fresh.key[(depth + sharedWord).toNat]? with
      | none => some (.branch (pfx.take sharedWord.toNat) (some fresh) (.cons oldByte oldChild .nil))
      | some byte => some (.branch (pfx.take sharedWord.toNat) none (twoEdges oldByte oldChild byte (.leaf fresh)))

theorem word_cut_exact (depth : USize) (sharedLength keyLength : Nat)
    (within : depth.toNat + sharedLength ≤ keyLength) (fits : keyLength < USize.size) :
    (USize.ofNat sharedLength).toNat = sharedLength ∧
      (depth + USize.ofNat sharedLength).toNat = depth.toNat + sharedLength := by
  have sharedFits : sharedLength < USize.size := by omega
  have conversion : (USize.ofNat sharedLength).toNat = sharedLength := USize.toNat_ofNat_of_lt sharedFits
  refine ⟨conversion, ?_⟩
  simpa only [conversion] using offset_word_add_exact depth (USize.ofNat sharedLength) keyLength
    (by simpa only [conversion] using within) fits

theorem split_leaf_word_at_refines (depth : USize) (sharedLength : Nat) (old fresh : Entry)
    (within : depth.toNat + sharedLength ≤ old.key.length) (fits : old.key.length < USize.size) :
    splitLeafWordAt depth sharedLength old fresh = splitLeafAt depth.toNat sharedLength old fresh := by
  have addition := (word_cut_exact depth sharedLength old.key.length within fits).2
  simp only [splitLeafWordAt, addition, Nat.add_sub_cancel_left, splitLeafAt]

theorem split_branch_word_at_refines (depth : USize) (sharedLength : Nat) (pfx : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (proper : sharedLength < pfx.length) (prefixFits : pfx.length < USize.size)
    (within : depth.toNat + sharedLength ≤ fresh.key.length) (freshFits : fresh.key.length < USize.size) :
    splitBranchWordAt depth sharedLength pfx terminal children fresh =
      splitBranchAt depth.toNat sharedLength pfx terminal children fresh := by
  obtain ⟨conversion, addition⟩ := word_cut_exact depth sharedLength fresh.key.length within freshFits
  have next : (USize.ofNat sharedLength + 1).toNat = sharedLength + 1 := by
    rw [word_edge_offset_exact _ pfx (by rw [conversion]; omega) prefixFits, conversion]
  simp only [splitBranchWordAt, conversion, addition, next, splitBranchAt]
  cases pfx[sharedLength]? <;> rfl

mutual
  def RoutingBuffersFit : Tree → Prop
    | .leaf entry => entry.key.length < USize.size
    | .branch pfx _ children => pfx.length < USize.size ∧ RoutingForestFits children
  def RoutingForestFits : Forest → Prop
    | .nil => True
    | .cons _ child tail => RoutingBuffersFit child ∧ RoutingForestFits tail
end

theorem edge_get_buffers_fit (forest : Forest) (index : Nat) (byte : UInt8) (child : Tree)
    (fits : RoutingForestFits forest) (found : edgeGet index forest = some (byte, child)) : RoutingBuffersFit child := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, RoutingForestFits]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons _ _ tail => exact ih tail fits.2 found

end Kv9.Radix
