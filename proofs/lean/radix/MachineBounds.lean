import Cardinality

set_option autoImplicit false

namespace Kv9.Radix

theorem prefix_length_bound (path key : Key) (valid : HasPrefix path key) :
    path.length ≤ key.length := by
  obtain ⟨suffix, rfl⟩ := valid
  simp

theorem common_cut_bounds (key pfx : Key) (depth : Nat) (valid : depth ≤ key.length) :
    depth ≤ depth + (common pfx (key.drop depth)).length ∧
    depth + (common pfx (key.drop depth)).length ≤ key.length ∧
    (common pfx (key.drop depth)).length ≤ pfx.length := by
  have bounds := common_bounds pfx (key.drop depth)
  simp only [List.length_drop] at bounds
  omega

theorem matching_prefix_bounds (key pfx : Key) (depth : Nat) (valid : depth ≤ key.length)
    (matching : ¬ (common pfx (key.drop depth)).length < pfx.length) :
    (common pfx (key.drop depth)).length = pfx.length ∧ depth + pfx.length ≤ key.length := by
  have bounds := common_cut_bounds key pfx depth valid
  omega

theorem byte_descent_bounds (key : Key) (depth commonLength : Nat) (byte : UInt8)
    (found : key[depth + commonLength]? = some byte) :
    depth + commonLength + 1 ≤ key.length ∧ depth < depth + commonLength + 1 ∧
    key.length - (depth + commonLength + 1) < key.length - depth := by
  have bound := (List.getElem?_eq_some_iff.mp found).1
  omega

theorem leaf_split_cut_bounds (path : Key) (old fresh : Entry)
    (oldValid : HasPrefix path old.key) (freshValid : HasPrefix path fresh.key) :
    let cut := path.length + (common (old.key.drop path.length) (fresh.key.drop path.length)).length
    path.length ≤ cut ∧ cut ≤ old.key.length ∧ cut ≤ fresh.key.length := by
  have oldBound := prefix_length_bound path old.key oldValid
  have freshBound := prefix_length_bound path fresh.key freshValid
  have bounds := common_bounds (old.key.drop path.length) (fresh.key.drop path.length)
  simp only [List.length_drop] at bounds
  dsimp only
  omega

theorem branch_split_access_bounds (pfx : Key) (commonLength : Nat)
    (proper : commonLength < pfx.length) :
    commonLength ≤ pfx.length ∧ commonLength + 1 ≤ pfx.length := by omega

theorem detached_edge_restore_bound (oldLength index : Nat) (found : index < oldLength) :
    index ≤ oldLength - 1 ∧ (oldLength - 1) + 1 = oldLength := by omega

theorem seek_after_bound (length index : Nat) (found : Bool)
    (search : if found then index < length else index ≤ length) :
    index + (if found then 1 else 0) ≤ length := by
  cases found <;> simp_all <;> omega

theorem collapsed_prefix_bound (path pfx childPrefix : Key) (byte : UInt8)
    (terminal : Option Entry) (children : Forest) (entry : Entry)
    (valid : Valid ((path ++ pfx) ++ [byte]) (.branch childPrefix terminal children))
    (member : entry ∈ entries (.branch childPrefix terminal children)) :
    path.length + (pfx ++ byte :: childPrefix).length ≤ entry.key.length := by
  have extended := valid_branch_prefix _ _ _ _ valid entry member
  have bound := prefix_length_bound _ _ extended
  simp only [List.length_append, List.length_cons] at *
  omega

def edgeCount : Forest → Nat
  | .nil => 0
  | .cons _ _ tail => edgeCount tail + 1

def LabelFloor (lower : Nat) : Forest → Prop
  | .nil => True
  | .cons byte _ tail => lower ≤ byte.toNat ∧ LabelFloor lower tail

theorem above_label_floor (byte : UInt8) (forest : Forest) (above : Above byte forest) :
    LabelFloor (byte.toNat + 1) forest := by
  cases forest with
  | nil => trivial
  | cons next child tail =>
      exact ⟨by have bound := above.1; omega, above_label_floor byte tail above.2⟩

theorem ordered_edge_count_bound (forest : Forest) (lower : Nat)
    (ordered : OrderedForest forest) (floor : LabelFloor lower forest) (bound : lower ≤ 256) :
    edgeCount forest + lower ≤ 256 := by
  cases forest with
  | nil => simpa [edgeCount] using bound
  | cons byte child tail =>
      have byteBound := byte.toNat_lt
      have tailBound := ordered_edge_count_bound tail (byte.toNat + 1) ordered.2.1
        (above_label_floor byte tail ordered.2.2) (by omega)
      have first := floor.1
      simp only [edgeCount]
      omega

theorem label_floor_zero (forest : Forest) : LabelFloor 0 forest := by
  cases forest with
  | nil => trivial
  | cons byte child tail => exact ⟨Nat.zero_le _, label_floor_zero tail⟩

theorem ordered_edges_fit (forest : Forest) (ordered : OrderedForest forest) :
    edgeCount forest ≤ 256 ∧ edgeCount forest < USize.size := by
  have count := ordered_edge_count_bound forest 0 ordered (label_floor_zero forest) (by omega)
  have machine := USize.le_size
  omega

theorem word_add_exact (a b : USize) (fits : a.toNat + b.toNat < USize.size) :
    (a + b).toNat = a.toNat + b.toNat := by
  rw [USize.toNat_add, Nat.mod_eq_of_lt fits]

theorem seek_after_word_exact (index : USize) (found : Bool) (forest : Forest)
    (ordered : OrderedForest forest)
    (search : if found then index.toNat < edgeCount forest else index.toNat ≤ edgeCount forest) :
    (index + found.toUSize).toNat = index.toNat + (if found then 1 else 0) ∧
    (index + found.toUSize).toNat ≤ edgeCount forest := by
  have within := seek_after_bound (edgeCount forest) index.toNat found search
  have bound := (ordered_edges_fit forest ordered).2
  have bit : found.toUSize.toNat = (if found then 1 else 0) := by
    cases found <;> simp [Bool.toUSize]
  have fit : index.toNat + found.toUSize.toNat < USize.size := by rw [bit]; omega
  rw [word_add_exact _ _ fit, bit]
  exact ⟨rfl, within⟩

theorem word_sub_one_exact (size : USize) (positive : 0 < size.toNat) :
    (size - 1).toNat = size.toNat - 1 := by
  have enough : (1 : USize) ≤ size := by
    rw [USize.le_iff_toNat_le]
    simp only [USize.reduceToNat]
    omega
  simpa using USize.toNat_sub_of_le size 1 enough

theorem offset_word_add_exact (depth commonLength : USize) (keyLength : Nat)
    (within : depth.toNat + commonLength.toNat ≤ keyLength) (fits : keyLength < USize.size) :
    (depth + commonLength).toNat = depth.toNat + commonLength.toNat := by
  apply word_add_exact
  omega

theorem byte_descent_word_exact (depth commonLength : USize) (key : Key) (byte : UInt8)
    (found : key[depth.toNat + commonLength.toNat]? = some byte)
    (fits : key.length < USize.size) :
    (depth + commonLength + 1).toNat = depth.toNat + commonLength.toNat + 1 := by
  have bounds := byte_descent_bounds key depth.toNat commonLength.toNat byte found
  have addition := offset_word_add_exact depth commonLength key.length (by omega) fits
  rw [word_add_exact (depth + commonLength) 1 (by simp only [addition]; simpa using Nat.lt_of_le_of_lt bounds.1 fits)]
  simp only [addition]
  simp

-- USize models word arithmetic. Agreement with Rust assumes the same word
-- width and the ordinary Rust/Lean compiler and primitive contracts. The new
-- tree's resident entry count must fit; heap correspondence remains explicit.
structure WordCounted where
  root : Option Tree
  size : USize

def WordGood (state : WordCounted) : Prop :=
  Good state.root ∧ state.size.toNat = (optionalEntries state.root).length

def wordPut (state : WordCounted) (fresh : Entry) : WordCounted × Bool :=
  match state.root with
  | none => (⟨some (.leaf fresh), 1⟩, true)
  | some tree =>
      let result := insertTree [] fresh tree
      (⟨some result.1, state.size + result.2.toUSize⟩, result.2)

def wordErase (state : WordCounted) (query : Key) : WordCounted × Bool :=
  if optionalLookup query state.root = none then (state, false)
  else (⟨eraseRoot query state.root, state.size - 1⟩, true)

theorem word_empty_good : WordGood ⟨none, 0⟩ := ⟨empty_good, rfl⟩

theorem word_put_root (state : WordCounted) (fresh : Entry) :
    (wordPut state fresh).1.root = some (putRoot fresh state.root).1 := by
  cases state with
  | mk root size => cases root <;> rfl

theorem word_put_good (state : WordCounted) (fresh : Entry) (good : WordGood state)
    (resident : (entries (putRoot fresh state.root).1).length < USize.size) :
    WordGood (wordPut state fresh).1 := by
  refine ⟨?_, ?_⟩
  · rw [word_put_root]
    exact put_root_good fresh state.root good.1
  · have count := put_root_cardinality fresh state.root good.1
    cases state with
    | mk root size =>
        cases root with
        | none => simp [wordPut, optionalEntries, entries]
        | some tree =>
            have sizeEq : size.toNat = (entries tree).length := good.2
            change (size + (insertTree [] fresh tree).2.toUSize).toNat =
              (entries (insertTree [] fresh tree).1).length
            have bit : ((insertTree [] fresh tree).2.toUSize).toNat =
                (if (insertTree [] fresh tree).2 then 1 else 0) := by
              cases (insertTree [] fresh tree).2 <;> simp [Bool.toUSize]
            have fit : size.toNat + ((insertTree [] fresh tree).2.toUSize).toNat < USize.size := by
              rw [sizeEq, bit]
              exact count ▸ resident
            rw [word_add_exact _ _ fit, sizeEq, bit]
            exact count.symm

theorem word_erase_root (state : WordCounted) (query : Key) :
    (wordErase state query).1.root = eraseRoot query state.root := by
  by_cases absent : optionalLookup query state.root = none
  · simp only [wordErase, if_pos absent]
    cases rootEq : state.root with
    | none => rfl
    | some tree =>
        have treeAbsent : treeLookup query tree query = none := by
          simpa [optionalLookup, rootEq] using absent
        simp [eraseRoot, treeAbsent]
  · simp [wordErase, absent]

theorem word_erase_good (state : WordCounted) (query : Key) (good : WordGood state) :
    WordGood (wordErase state query).1 := by
  by_cases absent : optionalLookup query state.root = none
  · simpa [wordErase, absent] using good
  · refine ⟨?_, ?_⟩
    · rw [word_erase_root]
      exact erase_root_good query state.root good.1
    · have count := erase_root_cardinality query state.root good.1.1
      simp only [if_neg absent] at count
      have positive : 0 < state.size.toNat := by rw [good.2]; omega
      simp only [wordErase, if_neg absent]
      rw [word_sub_one_exact _ positive, good.2]
      omega

def wordMutation (state : WordCounted) : Mutation → WordCounted
  | .put entry => (wordPut state entry).1
  | .erase key => (wordErase state key).1

def wordRun (state : WordCounted) (history : List Mutation) : WordCounted :=
  history.foldl wordMutation state

def ResidentHistory (root : Option Tree) : List Mutation → Prop
  | [] => True
  | op :: tail =>
      (optionalEntries (applyMutation root op)).length < USize.size ∧
        ResidentHistory (applyMutation root op) tail

theorem word_mutation_root (state : WordCounted) (op : Mutation) :
    (wordMutation state op).root = applyMutation state.root op := by
  cases op with
  | put entry => exact word_put_root state entry
  | erase key => exact word_erase_root state key

theorem word_mutation_good (state : WordCounted) (op : Mutation) (good : WordGood state)
    (resident : (optionalEntries (applyMutation state.root op)).length < USize.size) :
    WordGood (wordMutation state op) := by
  cases op with
  | put entry => exact word_put_good state entry good resident
  | erase key => exact word_erase_good state key good

theorem word_history_good (state : WordCounted) (history : List Mutation) (good : WordGood state)
    (resident : ResidentHistory state.root history) : WordGood (wordRun state history) := by
  induction history generalizing state with
  | nil => exact good
  | cons op tail ih =>
      apply ih (wordMutation state op) (word_mutation_good state op good resident.1)
      simpa only [word_mutation_root] using resident.2

theorem word_history_root (state : WordCounted) (history : List Mutation) :
    (wordRun state history).root = run state.root history := by
  induction history generalizing state with
  | nil => rfl
  | cons op tail ih =>
      simpa only [wordRun, run, List.foldl_cons, word_mutation_root] using ih (wordMutation state op)

theorem word_history_size (state : WordCounted) (history : List Mutation) (good : WordGood state)
    (resident : ResidentHistory state.root history) :
    (wordRun state history).size.toNat = (optionalEntries (run state.root history)).length := by
  rw [← word_history_root]
  exact (word_history_good state history good resident).2

-- A representability bridge, not a proof of Rust allocator ownership. Each
-- actual resident entry needs distinct nonzero-sized storage. A later heap
-- simulation must discharge both premises instead of treating this as free.
theorem resident_count_fits (root : Option Tree) (allocatedBytes : Nat)
    (distinctEntryStorage : (optionalEntries root).length ≤ allocatedBytes)
    (addressable : allocatedBytes < USize.size) :
    (optionalEntries root).length < USize.size := by omega

end Kv9.Radix
