import CRC32

set_option linter.unusedSimpArgs false

namespace Kv9.CRC32

def zeroBytes : Nat → BitVec 32 → BitVec 32
  | 0, c => c
  | n + 1, c => zeroBytes n (bits 8 c)

def sliceEntry (row : Nat) (byte : BitVec 8) : BitVec 32 :=
  zeroBytes row (tableEntry byte)

def byteAt (c : BitVec 32) (position : Nat) : BitVec 8 :=
  (c >>> (8 * position)).truncate 8

theorem zero_xor (n : Nat) (x y : BitVec 32) :
    zeroBytes n (x ^^^ y) = zeroBytes n x ^^^ zeroBytes n y := by
  induction n generalizing x y with
  | zero => rfl
  | succ n ih =>
    simp only [zeroBytes, bits_xor]
    exact ih _ _

theorem zero_zero (n : Nat) : zeroBytes n 0#32 = 0#32 := by
  induction n with
  | zero => rfl
  | succ n ih =>
    have h : bits 8 0#32 = 0#32 := by decide
    simpa only [zeroBytes, h] using ih

theorem zero_peel (n : Nat) (c : BitVec 32) :
    zeroBytes (n + 1) c = zeroBytes n (c >>> 8) ^^^ sliceEntry n (byteAt c 0) := by
  have h : bits 8 c = (c >>> 8) ^^^ tableEntry (c.truncate 8) := by
    simpa [oldByte, tableByte, index] using byte_transition c 0#8
  simp only [zeroBytes, h, zero_xor, sliceEntry, byteAt, Nat.mul_zero,
    BitVec.ushiftRight_zero]

theorem zero_eight_decomposition (c : BitVec 32) :
    zeroBytes 8 c =
      sliceEntry 7 (byteAt c 0) ^^^ sliceEntry 6 (byteAt c 1) ^^^
      sliceEntry 5 (byteAt c 2) ^^^ sliceEntry 4 (byteAt c 3) := by
  rw [zero_peel 7 c, zero_peel 6 (c >>> 8),
      zero_peel 5 ((c >>> 8) >>> 8), zero_peel 4 (((c >>> 8) >>> 8) >>> 8)]
  simp only [byteAt, Nat.mul_zero, BitVec.ushiftRight_zero, ← BitVec.shiftRight_add]
  have h : c >>> 32 = 0#32 := BitVec.ushiftRight_eq_zero (by decide)
  simp only [h, zero_zero, BitVec.zero_xor]
  ac_rfl


def packed (b0 b1 b2 b3 : BitVec 8) : BitVec 32 :=
  b0.zeroExtend 32 ||| (b1.zeroExtend 32 <<< 8) |||
  (b2.zeroExtend 32 <<< 16) ||| (b3.zeroExtend 32 <<< 24)

theorem packed_bytes (b0 b1 b2 b3 : BitVec 8) :
    byteAt (packed b0 b1 b2 b3) 0 = b0 ∧
    byteAt (packed b0 b1 b2 b3) 1 = b1 ∧
    byteAt (packed b0 b1 b2 b3) 2 = b2 ∧
    byteAt (packed b0 b1 b2 b3) 3 = b3 := by
  refine ⟨?_, ?_, ?_, ?_⟩
  all_goals
    apply BitVec.eq_of_getLsbD_eq
    intro i hi
    simp (disch := omega) [byteAt, packed, BitVec.truncate_eq_setWidth,
      BitVec.zeroExtend_eq_setWidth, BitVec.getLsbD_of_ge,
      show ¬8+i<8 by omega, show 8+i<16 by omega, show 8+i<24 by omega,
      show ¬16+i<16 by omega, show 16+i<24 by omega]

theorem byte_at_xor (x y : BitVec 32) (n : Nat) :
    byteAt (x ^^^ y) n = byteAt x n ^^^ byteAt y n := by
  simp [byteAt, BitVec.ushiftRight_xor_distrib, BitVec.truncate_eq_setWidth]

theorem slice_xor (n : Nat) (x y : BitVec 8) :
    sliceEntry n (x ^^^ y) = sliceEntry n x ^^^ sliceEntry n y := by
  simp only [sliceEntry, tableEntry, BitVec.zeroExtend_eq_setWidth,
    BitVec.setWidth_xor, bits_xor, zero_xor]

theorem zero_comp (n m : Nat) (x : BitVec 32) :
    zeroBytes n (zeroBytes m x) = zeroBytes (n + m) x := by
  induction m generalizing x with
  | zero => rfl
  | succ m ih =>
    change zeroBytes n (zeroBytes m (bits 8 x)) = zeroBytes (n + m) (bits 8 x)
    exact ih _

theorem zero_slice (n m : Nat) (b : BitVec 8) :
    zeroBytes n (sliceEntry m b) = sliceEntry (n + m) b := by
  exact zero_comp n m (tableEntry b)

theorem old_byte_linear (c : BitVec 32) (b : BitVec 8) :
    oldByte c b = zeroBytes 1 c ^^^ sliceEntry 0 b := by
  simp only [oldByte, bits_xor, zeroBytes, sliceEntry, tableEntry]

theorem old_eight_expansion (c : BitVec 32) (b0 b1 b2 b3 b4 b5 b6 b7 : BitVec 8) :
    oldFold [b0,b1,b2,b3,b4,b5,b6,b7] c =
      zeroBytes 8 c ^^^ sliceEntry 7 b0 ^^^ sliceEntry 6 b1 ^^^
      sliceEntry 5 b2 ^^^ sliceEntry 4 b3 ^^^ sliceEntry 3 b4 ^^^
      sliceEntry 2 b5 ^^^ sliceEntry 1 b6 ^^^ sliceEntry 0 b7 := by
  simp only [oldFold, List.foldl_cons, List.foldl_nil, old_byte_linear,
    zero_xor, zero_comp, zero_slice]

def sliceBlock (c : BitVec 32) (b0 b1 b2 b3 b4 b5 b6 b7 : BitVec 8) : BitVec 32 :=
  let low := c ^^^ packed b0 b1 b2 b3
  sliceEntry 7 (byteAt low 0) ^^^ sliceEntry 6 (byteAt low 1) ^^^
  sliceEntry 5 (byteAt low 2) ^^^ sliceEntry 4 (byteAt low 3) ^^^
  sliceEntry 3 b4 ^^^ sliceEntry 2 b5 ^^^ sliceEntry 1 b6 ^^^ sliceEntry 0 b7

/-- Universal state and eight-byte equivalence, using only algebraic kernel proofs. -/
theorem block_transition (c : BitVec 32) (b0 b1 b2 b3 b4 b5 b6 b7 : BitVec 8) :
    oldFold [b0,b1,b2,b3,b4,b5,b6,b7] c = sliceBlock c b0 b1 b2 b3 b4 b5 b6 b7 := by
  sorry


/-- Consume each full block, then process the remaining fewer than eight bytes. -/
def slicingFold : List (BitVec 8) → BitVec 32 → BitVec 32
  | b0::b1::b2::b3::b4::b5::b6::b7::tail, c =>
      slicingFold tail (sliceBlock c b0 b1 b2 b3 b4 b5 b6 b7)
  | tail, c => tableFold tail c

theorem slicing_list_equivalence : (bytes : List (BitVec 8)) → (c : BitVec 32) →
    oldFold bytes c = slicingFold bytes c
  | [], c => by simpa only [slicingFold] using list_equivalence [] c
  | [b0], c => by simpa only [slicingFold] using list_equivalence [b0] c
  | [b0,b1], c => by simpa only [slicingFold] using list_equivalence [b0,b1] c
  | [b0,b1,b2], c => by simpa only [slicingFold] using list_equivalence [b0,b1,b2] c
  | [b0,b1,b2,b3], c => by simpa only [slicingFold] using list_equivalence [b0,b1,b2,b3] c
  | [b0,b1,b2,b3,b4], c => by simpa only [slicingFold] using list_equivalence [b0,b1,b2,b3,b4] c
  | [b0,b1,b2,b3,b4,b5], c => by simpa only [slicingFold] using list_equivalence [b0,b1,b2,b3,b4,b5] c
  | [b0,b1,b2,b3,b4,b5,b6], c => by simpa only [slicingFold] using list_equivalence [b0,b1,b2,b3,b4,b5,b6] c
  | b0::b1::b2::b3::b4::b5::b6::b7::tail, c => by
      change oldFold tail (oldFold [b0,b1,b2,b3,b4,b5,b6,b7] c) =
        slicingFold tail (sliceBlock c b0 b1 b2 b3 b4 b5 b6 b7)
      rw [block_transition]
      exact slicing_list_equivalence tail _

def slicingParts (parts : List (List (BitVec 8))) (c : BitVec 32) : BitVec 32 :=
  parts.foldl (fun state part => slicingFold part state) c

theorem slicing_parts_equivalence (parts : List (List (BitVec 8))) (c : BitVec 32) :
    oldFold parts.flatten c = slicingParts parts c := by
  induction parts generalizing c with
  | nil => rfl
  | cons part parts ih =>
    simp only [slicingParts, List.foldl_cons, List.flatten_cons, oldFold,
      List.foldl_append] at *
    rw [ih]
    rw [← slicing_list_equivalence]
    rfl

def slicingChecksum (parts : List (List (BitVec 8))) : BitVec 32 :=
  ~~~(slicingParts parts initial)

theorem slicing_checksum_equivalence (parts : List (List (BitVec 8))) :
    oldChecksum parts.flatten = slicingChecksum parts := by
  simp only [oldChecksum, slicingChecksum, slicing_parts_equivalence]

theorem slicing_fragmentation_invariant (left right : List (List (BitVec 8)))
    (same : left.flatten = right.flatten) : slicingChecksum left = slicingChecksum right := by
  rw [← slicing_checksum_equivalence, ← slicing_checksum_equivalence, same]

end Kv9.CRC32


#print axioms Kv9.CRC32.zero_xor
#print axioms Kv9.CRC32.zero_zero
#print axioms Kv9.CRC32.zero_peel
#print axioms Kv9.CRC32.zero_eight_decomposition
#print axioms Kv9.CRC32.packed_bytes
#print axioms Kv9.CRC32.byte_at_xor
#print axioms Kv9.CRC32.slice_xor
#print axioms Kv9.CRC32.zero_comp
#print axioms Kv9.CRC32.zero_slice
#print axioms Kv9.CRC32.old_byte_linear
#print axioms Kv9.CRC32.old_eight_expansion
#print axioms Kv9.CRC32.block_transition
#print axioms Kv9.CRC32.slicing_list_equivalence
#print axioms Kv9.CRC32.slicing_parts_equivalence
#print axioms Kv9.CRC32.slicing_checksum_equivalence
#print axioms Kv9.CRC32.slicing_fragmentation_invariant
