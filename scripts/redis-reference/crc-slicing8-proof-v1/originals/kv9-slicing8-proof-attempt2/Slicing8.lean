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
  simp only [byteAt, Nat.mul_zero, BitVec.ushiftRight_zero, BitVec.shiftRight_add]
  have h : c >>> 32 = 0#32 := BitVec.ushiftRight_eq_zero (by decide)
  simp only [h, zero_zero, BitVec.zero_xor]
  ac_rfl

end Kv9.CRC32

#print axioms Kv9.CRC32.zero_xor
#print axioms Kv9.CRC32.zero_zero
#print axioms Kv9.CRC32.zero_peel
#print axioms Kv9.CRC32.zero_eight_decomposition
