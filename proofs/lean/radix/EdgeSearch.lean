import EdgeVector

set_option autoImplicit false

namespace Kv9.Radix

inductive EdgeSearchResult where
  | hit (index : Nat)
  | miss (index : Nat)
deriving DecidableEq, Repr

def bumpSearch : EdgeSearchResult → EdgeSearchResult
  | .hit index => .hit (index + 1)
  | .miss index => .miss (index + 1)

def edgeSearch (target : UInt8) : Forest → EdgeSearchResult
  | .nil => .miss 0
  | .cons byte _ tail =>
      if target = byte then .hit 0 else if target < byte then .miss 0 else bumpSearch (edgeSearch target tail)

def Below (upper : UInt8) : Forest → Prop
  | .nil => True
  | .cons byte _ tail => byte.toNat < upper.toNat ∧ Below upper tail

-- The standard library returns a hit at the equal label, or an in-bounds
-- insertion index with smaller labels before it and larger labels after it.
-- Strict ordering excludes duplicate labels and makes this result unique.
def EdgeSearchContract (target : UInt8) (forest : Forest) : EdgeSearchResult → Prop
  | .hit index => ∃ child, edgeGet index forest = some (target, child)
  | .miss index => index ≤ edgeCount forest ∧ Below target (edgeTake index forest) ∧ Above target (edgeDrop index forest)

theorem byte_less_excludes (a b : UInt8) (less : a.toNat < b.toNat) : b ≠ a ∧ ¬ b < a := by
  constructor
  · intro same
    simp [same] at less
  · change ¬ b.toNat < a.toNat
    omega

theorem above_get (lower : UInt8) (forest : Forest) (index : Nat) (byte : UInt8) (child : Tree)
    (above : Above lower forest) (found : edgeGet index forest = some (byte, child)) : lower.toNat < byte.toNat := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, Above]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons _ _ tail => exact ih tail above.2 found

theorem edge_search_hit_complete (target : UInt8) (forest : Forest) (index : Nat) (child : Tree)
    (ordered : OrderedForest forest) (found : edgeGet index forest = some (target, child)) :
    edgeSearch target forest = .hit index := by
  induction index generalizing forest with
  | zero => cases forest <;> simp_all [edgeGet, edgeSearch]
  | succ n ih =>
      cases forest with
      | nil => simp [edgeGet] at found
      | cons byte node tail =>
          have less := above_get byte tail n target child ordered.2.2 found
          obtain ⟨different, notBefore⟩ := byte_less_excludes byte target less
          simp [edgeSearch, different, notBefore, ih tail ordered.2.1 found, bumpSearch]

theorem edge_search_above (target : UInt8) (forest : Forest) (above : Above target forest) :
    edgeSearch target forest = .miss 0 := by
  cases forest with
  | nil => rfl
  | cons byte child tail =>
      have less : target < byte := above.1
      have different : target ≠ byte := by
        intro same
        have bound := above.1
        simp [same] at bound
      simp [edgeSearch, different, less]

theorem edge_search_miss_complete (target : UInt8) (forest : Forest) (index : Nat)
    (bounded : index ≤ edgeCount forest) (below : Below target (edgeTake index forest))
    (above : Above target (edgeDrop index forest)) : edgeSearch target forest = .miss index := by
  induction index generalizing forest with
  | zero => exact edge_search_above target forest above
  | succ n ih =>
      cases forest with
      | nil => simp [edgeCount] at bounded
      | cons byte child tail =>
          have bound : n ≤ edgeCount tail := by simpa [edgeCount] using bounded
          obtain ⟨different, notBefore⟩ := byte_less_excludes byte target below.1
          simp [edgeSearch, different, notBefore, ih tail bound below.2 above, bumpSearch]

theorem edge_search_contract_unique (target : UInt8) (forest : Forest) (result : EdgeSearchResult)
    (ordered : OrderedForest forest) (contract : EdgeSearchContract target forest result) :
    result = edgeSearch target forest := by
  cases result with
  | hit index =>
      obtain ⟨child, found⟩ := contract
      exact (edge_search_hit_complete target forest index child ordered found).symm
  | miss index => exact (edge_search_miss_complete target forest index contract.1 contract.2.1 contract.2.2).symm

theorem edge_search_contract (target : UInt8) (forest : Forest) (ordered : OrderedForest forest) :
    EdgeSearchContract target forest (edgeSearch target forest) := by
  cases forest with
  | nil => exact ⟨by omega, trivial, trivial⟩
  | cons byte child tail =>
      by_cases same : target = byte
      · subst target
        simp only [edgeSearch, EdgeSearchContract]
        exact ⟨child, rfl⟩
      · by_cases before : target < byte
        · simp only [edgeSearch, if_neg same, if_pos before, EdgeSearchContract, edgeTake, edgeDrop]
          exact ⟨by omega, trivial, before, above_trans target byte tail before ordered.2.2⟩
        · have previous := edge_search_contract target tail ordered.2.1
          have earlier := byte_reverse_lt target byte same before
          rw [edgeSearch, if_neg same, if_neg before]
          cases resultEq : edgeSearch target tail with
          | hit index =>
              obtain ⟨found, hit⟩ := (by simpa [resultEq, EdgeSearchContract] using previous : ∃ found, edgeGet index tail = some (target, found))
              exact ⟨found, hit⟩
          | miss index =>
              have rest : index ≤ edgeCount tail ∧ Below target (edgeTake index tail) ∧ Above target (edgeDrop index tail) := by
                simpa [resultEq, EdgeSearchContract] using previous
              exact ⟨by simp only [edgeCount]; omega, ⟨earlier, rest.2.1⟩, rest.2.2⟩

theorem edge_search_hit_get (target : UInt8) (forest : Forest) (index : Nat)
    (hit : edgeSearch target forest = .hit index) : ∃ child, edgeGet index forest = some (target, child) := by
  cases forest with
  | nil => simp [edgeSearch] at hit
  | cons byte child tail =>
      by_cases same : target = byte
      · simp only [edgeSearch, if_pos same, EdgeSearchResult.hit.injEq] at hit
        subst index
        exact ⟨child, by simp [edgeGet, same]⟩
      · by_cases before : target < byte
        · simp [edgeSearch, same, before] at hit
        · cases resultEq : edgeSearch target tail with
          | miss n => simp [edgeSearch, same, before, resultEq, bumpSearch] at hit
          | hit n =>
              have indexEq : n + 1 = index := by simpa [edgeSearch, same, before, resultEq, bumpSearch] using hit
              obtain ⟨found, childEq⟩ := edge_search_hit_get target tail n resultEq
              exact ⟨found, by simpa [← indexEq, edgeGet] using childEq⟩

theorem edge_search_miss_bound (target : UInt8) (forest : Forest) (index : Nat)
    (miss : edgeSearch target forest = .miss index) : index ≤ edgeCount forest := by
  cases forest with
  | nil => simp [edgeSearch] at miss; simp [← miss, edgeCount]
  | cons byte child tail =>
      by_cases same : target = byte
      · simp [edgeSearch, same] at miss
      · by_cases before : target < byte
        · simp only [edgeSearch, if_neg same, if_pos before, EdgeSearchResult.miss.injEq] at miss
          simp [← miss]
        · cases resultEq : edgeSearch target tail with
          | hit n => simp [edgeSearch, same, before, resultEq, bumpSearch] at miss
          | miss n =>
              have indexEq : n + 1 = index := by simpa [edgeSearch, same, before, resultEq, bumpSearch] using miss
              have bound := edge_search_miss_bound target tail n resultEq
              simp only [edgeCount]
              omega

end Kv9.Radix
