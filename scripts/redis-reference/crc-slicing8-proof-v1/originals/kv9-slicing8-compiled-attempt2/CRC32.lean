import Std

-- Repeated Boolean branches intentionally share the same simplifier arguments.
set_option linter.unusedSimpArgs false

namespace Kv9.CRC32

def polynomial : BitVec 32 := 0xEDB88320#32

def bitStep (crc : BitVec 32) : BitVec 32 :=
  (crc >>> 1) ^^^ (polynomial &&& (-(crc &&& 1#32)))

 theorem bit_mask (x : BitVec 32) : -(x &&& 1#32) = if x.getLsbD 0 then 0xFFFFFFFF#32 else 0#32 := by
  rw [BitVec.and_one_eq_setWidth_ofBool_getLsbD]
  cases h : x.getLsbD 0 <;> rfl
 theorem bit_step_xor (x y : BitVec 32) : bitStep (x ^^^ y) = bitStep x ^^^ bitStep y := by
  simp only [bitStep, polynomial, bit_mask, BitVec.getLsbD_xor, BitVec.ushiftRight_xor_distrib]
  cases hx : x.getLsbD 0 <;> cases hy : y.getLsbD 0
  all_goals
    simp only [Bool.false_xor, Bool.true_xor, Bool.xor_false, Bool.xor_true,
      Bool.not_false, Bool.not_true, Bool.false_eq_true,
      ite_true, ite_false]
    change ((x >>> 1) ^^^ (y >>> 1)) ^^^ _ = ((x >>> 1) ^^^ _) ^^^ ((y >>> 1) ^^^ _)
    apply BitVec.eq_of_getLsbD_eq
    intro i hi
    simp only [BitVec.getLsbD_xor, BitVec.getLsbD_and]
    cases (x >>> 1).getLsbD i <;> cases (y >>> 1).getLsbD i <;>
      cases (0xEDB88320 : BitVec 32).getLsbD i <;>
      simp [BitVec.getLsbD_allOnes, hi]

def bits : Nat → BitVec 32 → BitVec 32
 | 0,x => x
 | n+1,x => bits n (bitStep x)
theorem high_mask_bit (i : Nat) : (0xFFFFFF00#32).getLsbD i = decide (8 ≤ i ∧ i < 32) := by
 by_cases h : i < 32
 · have cases : i = 0 ∨ i = 1 ∨ i = 2 ∨ i = 3 ∨ i = 4 ∨ i = 5 ∨ i = 6 ∨ i = 7 ∨ i = 8 ∨ i = 9 ∨ i = 10 ∨ i = 11 ∨ i = 12 ∨ i = 13 ∨ i = 14 ∨ i = 15 ∨ i = 16 ∨ i = 17 ∨ i = 18 ∨ i = 19 ∨ i = 20 ∨ i = 21 ∨ i = 22 ∨ i = 23 ∨ i = 24 ∨ i = 25 ∨ i = 26 ∨ i = 27 ∨ i = 28 ∨ i = 29 ∨ i = 30 ∨ i = 31 := by omega
   rcases cases with rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl | rfl <;> decide
 · simp [BitVec.getLsbD_of_ge (0xFFFFFF00#32) i (by omega), h]
theorem high_step (x : BitVec 32) (n : Nat) (bound : n < 8) :
 bitStep ((x &&& 0xFFFFFF00#32) >>> n) = (x &&& 0xFFFFFF00#32) >>> (n+1) := by
 have low : ((x &&& 0xFFFFFF00#32) >>> n).getLsbD 0 = false := by
  simp [BitVec.getLsbD_ushiftRight, high_mask_bit, show ¬8 ≤ n by omega]
 simp only [bitStep, polynomial, bit_mask, low, Bool.false_eq_true, ite_false,
   BitVec.and_zero, BitVec.xor_zero, BitVec.shiftRight_add]
theorem high_byte_shift (x : BitVec 32) : bits 8 (x &&& 0xFFFFFF00#32) = x >>> 8 := by
 have h0 := high_step x 0 (by decide)
 have h1 := high_step x 1 (by decide)
 have h2 := high_step x 2 (by decide)
 have h3 := high_step x 3 (by decide)
 have h4 := high_step x 4 (by decide)
 have h5 := high_step x 5 (by decide)
 have h6 := high_step x 6 (by decide)
 have h7 := high_step x 7 (by decide)
 simp only [BitVec.ushiftRight_zero] at h0
 simp only [bits, h0, h1, h2, h3, h4, h5, h6, h7]
 apply BitVec.eq_of_getLsbD_eq
 intro i hi
 simp only [BitVec.getLsbD_ushiftRight, BitVec.getLsbD_and, high_mask_bit]
 by_cases upper : 8+i < 32
 · simp [upper]
 · simp [upper, BitVec.getLsbD_of_ge x (8+i) (by omega)]

def oldByte (crc : BitVec 32) (byte : BitVec 8) : BitVec 32 :=
  bits 8 (crc ^^^ byte.zeroExtend 32)

def tableEntry (index : BitVec 8) : BitVec 32 := bits 8 (index.zeroExtend 32)

def index (crc : BitVec 32) (byte : BitVec 8) : BitVec 8 :=
  (crc ^^^ byte.zeroExtend 32).truncate 8

theorem masked_index (crc : BitVec 32) (byte : BitVec 8) :
    ((crc ^^^ byte.zeroExtend 32) &&& 0xff#32) =
      (index crc byte).zeroExtend 32 := by
  change ((crc ^^^ byte.zeroExtend 32) &&& (BitVec.allOnes 8).setWidth (24 + 8)) =
    ((crc ^^^ byte.zeroExtend 32).setWidth 8).setWidth 32
  rw [BitVec.and_setWidth_allOnes 24 8]
  symm
  simpa using (BitVec.setWidth_eq_append
    (x := (crc ^^^ byte.zeroExtend 32).setWidth 8) (w := 32) (by decide))

theorem byte_split (crc : BitVec 32) (byte : BitVec 8) :
    (crc ^^^ byte.zeroExtend 32) =
      (crc &&& 0xFFFFFF00#32) ^^^ (index crc byte).zeroExtend 32 := by
  have low : (byte.zeroExtend 32 &&& 0xff#32) = byte.zeroExtend 32 := by
    simpa [index] using (masked_index 0#32 byte)
  rw [← masked_index]
  change (crc ^^^ byte.zeroExtend 32) =
    (crc &&& ~~~(0xff#32)) ^^^ ((crc ^^^ byte.zeroExtend 32) &&& 0xff#32)
  apply BitVec.eq_of_getElem_eq
  intro i hi
  have low_bit := congrArg (fun x : BitVec 32 => x[i]) low
  simp only [BitVec.getElem_and] at low_bit
  simp only [BitVec.getElem_xor, BitVec.getElem_and, BitVec.getElem_not]
  cases hc : crc[i] <;> cases hb : (byte.zeroExtend 32)[i] <;>
    cases hm : (0xff#32)[i] <;> simp_all

theorem bits_xor (count : Nat) (left right : (BitVec 32)) :
    bits count (left ^^^ right) = bits count left ^^^ bits count right := by
  induction count generalizing left right with
  | zero => rfl
  | succ count ih =>
    simp only [bits, bit_step_xor]
    exact ih _ _


def tableByte (crc : BitVec 32) (byte : BitVec 8) : BitVec 32 :=
  (crc >>> 8) ^^^ tableEntry (index crc byte)

/-- Universal equivalence over every 32-bit state and 8-bit byte. -/
theorem byte_transition (crc : BitVec 32) (byte : BitVec 8) :
    oldByte crc byte = tableByte crc byte := by
  unfold oldByte
  rw [byte_split, bits_xor, high_byte_shift]
  rfl

def oldFold (bytes : List (BitVec 8)) (crc : (BitVec 32)) : (BitVec 32) := bytes.foldl oldByte crc
def tableFold (bytes : List (BitVec 8)) (crc : (BitVec 32)) : (BitVec 32) := bytes.foldl tableByte crc

theorem list_equivalence (bytes : List (BitVec 8)) (crc : (BitVec 32)) :
    oldFold bytes crc = tableFold bytes crc := by
  induction bytes generalizing crc with
  | nil => rfl
  | cons byte bytes ih =>
    simp only [oldFold, tableFold, List.foldl_cons] at *
    rw [byte_transition]
    exact ih _

/-- The optimized nested parts loop keeps state between parts, including empty ones. -/
def tableParts (parts : List (List (BitVec 8))) (crc : (BitVec 32)) : (BitVec 32) :=
  parts.foldl (fun state part => tableFold part state) crc

theorem table_fold_append (left right : List (BitVec 8)) (crc : (BitVec 32)) :
    tableFold (left ++ right) crc = tableFold right (tableFold left crc) := by
  exact List.foldl_append

theorem parts_flatten (parts : List (List (BitVec 8))) (crc : (BitVec 32)) :
    tableParts parts crc = tableFold parts.flatten crc := by
  induction parts generalizing crc with
  | nil => rfl
  | cons part parts ih =>
    simp only [tableParts, List.foldl_cons, List.flatten_cons] at *
    rw [table_fold_append]
    exact ih _

def initial : (BitVec 32) := 0xFFFFFFFF
def oldChecksum (bytes : List (BitVec 8)) : (BitVec 32) := ~~~(oldFold bytes initial)
def tableChecksum (parts : List (List (BitVec 8))) : (BitVec 32) := ~~~(tableParts parts initial)

/-- Arbitrary finite fragments with the same initial FFFFFFFF and final complement. -/
theorem checksum_equivalence (parts : List (List (BitVec 8))) :
    oldChecksum parts.flatten = tableChecksum parts := by
  simp only [oldChecksum, tableChecksum, parts_flatten, list_equivalence]

theorem singleton_equivalence (bytes : List (BitVec 8)) :
    oldChecksum bytes = tableChecksum [bytes] := by
  simpa using checksum_equivalence [bytes]

/-- Any two fragmentations of the same bytes yield the same final checksum. -/
theorem fragmentation_invariant (left right : List (List (BitVec 8)))
    (same : left.flatten = right.flatten) : tableChecksum left = tableChecksum right := by
  rw [← checksum_equivalence, ← checksum_equivalence, same]

end Kv9.CRC32
