import Std

namespace Kv9.FnvInterleave

variable {State Byte : Type}

def scalar (step : State → Byte → State) (bytes : List Byte) (hash : State) : State :=
  bytes.foldl step hash

def four (step : State → Byte → State) : List Byte → List Byte → List Byte → List Byte →
    State → State → State → State → State × State × State × State
  | a::as, b::bs, c::cs, d::ds, h0, h1, h2, h3 =>
      four step as bs cs ds (step h0 a) (step h1 b) (step h2 c) (step h3 d)
  | as, bs, cs, ds, h0, h1, h2, h3 =>
      (scalar step as h0, scalar step bs h1, scalar step cs h2, scalar step ds h3)

theorem scalar_append (step : State → Byte → State) (as bs : List Byte) (hash : State) :
    scalar step (as ++ bs) hash = scalar step bs (scalar step as hash) := by
  exact List.foldl_append

/-- Arbitrary initial states, all lengths, unequal tails and empty lanes. -/
theorem four_equivalence (step : State → Byte → State)
    (as bs cs ds : List Byte) (h0 h1 h2 h3 : State) :
    four step as bs cs ds h0 h1 h2 h3 =
      (scalar step as h0, scalar step bs h1, scalar step cs h2, scalar step ds h3) := by
  induction as generalizing bs cs ds h0 h1 h2 h3 with
  | nil => rfl
  | cons a as ih =>
    cases bs with
    | nil => rfl
    | cons b bs =>
      cases cs with
      | nil => rfl
      | cons c cs =>
        cases ds with
        | nil => rfl
        | cons d ds =>
          simpa only [four, scalar, List.foldl_cons] using
            ih bs cs ds (step h0 a) (step h1 b) (step h2 c) (step h3 d)

/-- A common prefix and each lane's scalar tail consume the same full list. -/
theorem split_equivalence (step : State → Byte → State)
    (a0 a1 b0 b1 c0 c1 d0 d1 : List Byte) (h0 h1 h2 h3 : State) :
    let states := four step a0 b0 c0 d0 h0 h1 h2 h3
    four step a1 b1 c1 d1 states.1 states.2.1 states.2.2.1 states.2.2.2 =
      four step (a0 ++ a1) (b0 ++ b1) (c0 ++ c1) (d0 ++ d1) h0 h1 h2 h3 := by
  simp only [four_equivalence, scalar_append]

/-- Changing the other lanes cannot affect lane zero, including ragged inputs. -/
theorem lane_independence (step : State → Byte → State)
    (as bs cs ds bs' cs' ds' : List Byte) (h0 h1 h2 h3 h1' h2' h3' : State) :
    (four step as bs cs ds h0 h1 h2 h3).1 =
      (four step as bs' cs' ds' h0 h1' h2' h3').1 := by
  simp only [four_equivalence]

theorem common_length_bound (a b c d : Nat) :
    min (min (min a b) c) d ≤ a ∧ min (min (min a b) c) d ≤ b ∧
    min (min (min a b) c) d ≤ c ∧ min (min (min a b) c) d ≤ d := by
  omega

abbrev Word := BitVec 32
abbrev Octet := BitVec 8

def initial : Word := 0x811c9dc5
def prime : Word := 0x01000193
def fnvStep (hash : Word) (byte : Octet) : Word :=
  (hash ^^^ byte.zeroExtend 32) * prime

def checksum4 (as bs cs ds : List Octet) : Word × Word × Word × Word :=
  four fnvStep as bs cs ds initial initial initial initial

theorem checksum_equivalence (as bs cs ds : List Octet) :
    checksum4 as bs cs ds =
      (scalar fnvStep as initial, scalar fnvStep bs initial,
       scalar fnvStep cs initial, scalar fnvStep ds initial) := by
  exact four_equivalence fnvStep as bs cs ds initial initial initial initial

end Kv9.FnvInterleave
