import Std

namespace Kv9.WalCapacity

abbrev Byte := UInt8

def u32le (n : Nat) : List Byte :=
  List.ofFn fun i : Fin 4 => UInt8.ofNat (n / 2 ^ (8 * i.val))

inductive Mutation where
  | put (cf : Byte) (key value : List Byte)
  | delete (cf : Byte) (key : List Byte)

def chunks : Mutation → List (List Byte)
  | .put cf key value => [[0], [cf], u32le key.length, key, u32le value.length, value]
  | .delete cf key => [[1], [cf], u32le key.length, key]

def mutationSize : Mutation → Nat
  | .put _ key value => 10 + key.length + value.length
  | .delete _ key => 6 + key.length

def batchChunks (batch : List Mutation) : List (List Byte) :=
  u32le batch.length :: batch.flatMap chunks

def payloadSize (batch : List Mutation) : Nat :=
  4 + (batch.map mutationSize).sum

theorem u32le_length (n : Nat) : (u32le n).length = 4 := by
  simp [u32le]

theorem mutation_size (m : Mutation) : (chunks m).flatten.length = mutationSize m := by
  cases m <;> simp [chunks, mutationSize, u32le_length] <;> omega

theorem body_size (batch : List Mutation) :
    (batch.flatMap chunks).flatten.length = (batch.map mutationSize).sum := by
  induction batch with
  | nil => rfl
  | cons m rest ih =>
      simp only [List.flatMap_cons, List.flatten_append, List.length_append,
        List.map_cons, List.sum_cons, mutation_size, ih]

theorem payload_size (batch : List Mutation) :
    (batchChunks batch).flatten.length = payloadSize batch := by
  simp only [batchChunks, List.flatten_cons, List.length_append,
    u32le_length, body_size, payloadSize]

-- Allocation growth is an arbitrary policy. Byte equivalence does not depend
-- on any growth factor. Successful Rust allocations/copies are the mapping
-- assumption; this is not a model of allocation failure or physical memory.
structure Buffer where
  bytes : List Byte
  capacity : Nat

def append (grow : Nat → Nat → Nat) (s : Buffer) (chunk : List Byte) : Buffer :=
  let bytes := s.bytes ++ chunk
  { bytes := bytes,
    capacity := if bytes.length ≤ s.capacity then s.capacity else grow s.capacity bytes.length }

def emit (grow : Nat → Nat → Nat) : Buffer → List (List Byte) → Buffer
  | s, [] => s
  | s, chunk :: rest => emit grow (append grow s chunk) rest

theorem emission_bytes (grow : Nat → Nat → Nat) (s : Buffer) (cs : List (List Byte)) :
    (emit grow s cs).bytes = s.bytes ++ cs.flatten := by
  induction cs generalizing s with
  | nil => simp [emit]
  | cons chunk rest ih => simp [emit, ih, append, List.append_assoc]

theorem capacity_independent_bytes (g1 g2 : Nat → Nat → Nat)
    (c1 c2 : Nat) (cs : List (List Byte)) :
    (emit g1 ⟨[], c1⟩ cs).bytes = (emit g2 ⟨[], c2⟩ cs).bytes := by
  simp [emission_bytes]

-- Stronger than checking only the final length: every append fits, hence the
-- growth policy is never invoked after a successful full reservation.
def noGrowth : Buffer → List (List Byte) → Prop
  | _, [] => True
  | s, chunk :: rest =>
      (s.bytes ++ chunk).length ≤ s.capacity ∧
      noGrowth ⟨s.bytes ++ chunk, s.capacity⟩ rest

theorem sufficient_reservation (s : Buffer) (cs : List (List Byte))
    (bound : s.bytes.length + cs.flatten.length ≤ s.capacity) : noGrowth s cs := by
  induction cs generalizing s with
  | nil => trivial
  | cons chunk rest ih =>
      simp only [List.flatten_cons, List.length_append] at bound
      constructor
      · simp only [List.length_append]; omega
      · apply ih
        simp only [List.length_append]
        omega

theorem no_capacity_growth (grow : Nat → Nat → Nat) (s : Buffer) (cs : List (List Byte))
    (bound : noGrowth s cs) : (emit grow s cs).capacity = s.capacity := by
  induction cs generalizing s with
  | nil => rfl
  | cons chunk rest ih =>
      rcases bound with ⟨fits, tail⟩
      simp only [emit, append, if_pos fits]
      exact ih ⟨s.bytes ++ chunk, s.capacity⟩ tail

theorem batch_reservation (grow : Nat → Nat → Nat) (batch : List Mutation) :
    (emit grow ⟨[], payloadSize batch⟩ (batchChunks batch)).bytes =
      (batchChunks batch).flatten ∧
    noGrowth ⟨[], payloadSize batch⟩ (batchChunks batch) ∧
    (emit grow ⟨[], payloadSize batch⟩ (batchChunks batch)).capacity = payloadSize batch := by
  have fits : noGrowth ⟨[], payloadSize batch⟩ (batchChunks batch) := by
    apply sufficient_reservation
    change 0 + (batchChunks batch).flatten.length ≤ payloadSize batch
    rw [payload_size]
    omega
  exact ⟨by simp [emission_bytes], fits, no_capacity_growth grow _ _ fits⟩

theorem checked_frame_subtraction (payload : Nat) :
    payload + 32 + 4 - 32 - 4 = payload := by omega

theorem validated_lengths (batch : List Mutation) (bound : payloadSize batch ≤ 67108864) :
    (batchChunks batch).flatten.length ≤ 67108864 ∧
    payloadSize batch + 32 + 4 < 4294967296 := by
  rw [payload_size]
  omega

end Kv9.WalCapacity
