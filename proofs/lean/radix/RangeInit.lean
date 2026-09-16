import RangeStep

set_option autoImplicit false

namespace Kv9.Radix

theorem upper_accepts_closed (query : Key) (inclusive : Bool) (a b : Entry)
    (less : a.key < b.key) (accepted : accepts query inclusive true b = true) :
    accepts query inclusive true a = true := by
  have cases : (inclusive = true ∧ b.key = query) ∨ b.key < query := by simpa [accepts] using accepted
  have before : a.key < query := by
    rcases cases with ⟨_, same⟩ | before
    · simpa only [same] using less
    · exact List.lt_trans less before
  exact accepts_less query inclusive true a before

theorem lower_accepts_closed (query : Key) (inclusive : Bool) (a b : Entry)
    (less : a.key < b.key) (accepted : accepts query inclusive false a = true) :
    accepts query inclusive false b = true := by
  have cases : (inclusive = true ∧ a.key = query) ∨ query < a.key := by simpa [accepts] using accepted
  have after : query < b.key := by
    rcases cases with ⟨_, same⟩ | after
    · simpa only [same] using less
    · exact List.lt_trans after less
  exact accepts_greater query inclusive false b after

def acceptsBound (bound : Option (Key × Bool)) (reverse : Bool) (entry : Entry) : Bool :=
  match bound with
  | none => true
  | some (query, inclusive) => accepts query inclusive reverse entry

theorem upper_bound_closed (bound : Option (Key × Bool)) (a b : Entry)
    (less : a.key < b.key) (accepted : acceptsBound bound true b = true) :
    acceptsBound bound true a = true := by
  cases bound with
  | none => rfl
  | some pair => exact upper_accepts_closed pair.1 pair.2 a b less accepted

theorem lower_bound_closed (bound : Option (Key × Bool)) (a b : Entry)
    (less : a.key < b.key) (accepted : acceptsBound bound false a = true) :
    acceptsBound bound false b = true := by
  cases bound with
  | none => rfl
  | some pair => exact lower_accepts_closed pair.1 pair.2 a b less accepted

theorem below_lower_closed (bound : Option (Key × Bool)) (a b : Entry)
    (less : a.key < b.key) (rejected : (!acceptsBound bound false b) = true) :
    (!acceptsBound bound false a) = true := by
  cases accepted : acceptsBound bound false a with
  | false => rfl
  | true =>
      have later := lower_bound_closed bound a b less accepted
      simp [later] at rejected

theorem bound_same_key (bound : Option (Key × Bool)) (reverse : Bool) (a b : Entry) (same : a.key = b.key) :
    acceptsBound bound reverse a = acceptsBound bound reverse b := by
  cases bound with
  | none => rfl
  | some pair => simp [acceptsBound, accepts, same]

theorem filter_sorted_partition (rows : List Entry) (predicate : Entry → Bool) (sorted : SortedEntries rows)
    (closed : ∀ a b, a.key < b.key → predicate b = true → predicate a = true) :
    rows.filter predicate ++ rows.filter (fun e => !predicate e) = rows := by
  induction rows with
  | nil => rfl
  | cons entry tail ih =>
      have parts := List.pairwise_cons.mp sorted
      cases head : predicate entry with
      | false =>
          have all : ∀ e ∈ entry :: tail, predicate e = false := by
            intro e member
            rcases List.mem_cons.mp member with same | later
            · simpa only [same] using head
            · cases value : predicate e with
              | false => rfl
              | true =>
                  have forced := closed entry e (parts.1 e later) value
                  simp [head] at forced
          rw [filter_constant _ _ false all,
            filter_constant _ _ true (fun e member => by rw [all e member]; rfl)]
          simp
      | true => simpa [head] using congrArg (List.cons entry) (ih parts.2)

theorem bound_rows_filter (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) :
    boundRows root bound reverse = orient reverse ((optionalEntries root).filter (acceptsBound bound reverse)) := by
  cases bound with
  | none =>
      change orient reverse (optionalEntries root) = orient reverse ((optionalEntries root).filter (fun _ => true))
      rw [List.filter_eq_self.mpr (fun _ _ => rfl)]
  | some pair => rfl

def selectedRows (root : Option Tree) (lower upper : Option (Key × Bool)) : List Entry :=
  (optionalEntries root).filter (fun e => acceptsBound lower false e && acceptsBound upper true e)

theorem selected_rows_sorted (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    SortedEntries (selectedRows root lower upper) := (good_root_sorted root good).filter _

theorem selected_front_split (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    ∃ after, (optionalEntries root).filter (acceptsBound lower false) = selectedRows root lower upper ++ after := by
  let front := (optionalEntries root).filter (acceptsBound lower false)
  refine ⟨front.filter (fun e => !acceptsBound upper true e), ?_⟩
  have partition := filter_sorted_partition front (acceptsBound upper true)
    ((good_root_sorted root good).filter _) (upper_bound_closed upper)
  simpa [front, selectedRows, List.filter_filter, Bool.and_comm] using partition.symm

theorem selected_back_split (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    ∃ before, (optionalEntries root).filter (acceptsBound upper true) = before ++ selectedRows root lower upper := by
  let back := (optionalEntries root).filter (acceptsBound upper true)
  refine ⟨back.filter (fun e => !acceptsBound lower false e), ?_⟩
  have partition := filter_sorted_partition back (fun e => !acceptsBound lower false e)
    ((good_root_sorted root good).filter _) (below_lower_closed lower)
  simpa [back, selectedRows, List.filter_filter, Bool.and_comm] using partition.symm

def initRange (root : Option Tree) (lower upper : Option (Key × Bool)) : RangeState :=
  ⟨loadCursor false (seekRoot root lower false), loadCursor true (seekRoot root upper true)⟩

theorem init_front_rows (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    frontierRows false (initRange root lower upper).front = (optionalEntries root).filter (acceptsBound lower false) := by
  simp [initRange, load_cursor_rows, seek_root_rows root lower false good, bound_rows_filter, orient]

theorem init_back_rows (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    frontierRows true (initRange root lower upper).back = ((optionalEntries root).filter (acceptsBound upper true)).reverse := by
  simp only [initRange, load_cursor_rows, seek_root_rows root upper true good, bound_rows_filter, orient, if_true]

theorem init_empty_crossed (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root)
    (empty : selectedRows root lower upper = []) : EmptyRange (initRange root lower upper) := by
  generalize stateEq : initRange root lower upper = state
  cases frontEq : state.front.current with
  | none => exact Or.inl frontEq
  | some first =>
      cases backEq : state.back.current with
      | none => exact Or.inr (Or.inl backEq)
      | some last =>
          refine Or.inr (Or.inr ⟨first, last, frontEq, backEq, ?_⟩)
          have frontRows := init_front_rows root lower upper good
          have backRows := init_back_rows root lower upper good
          rw [stateEq, frontierRows, frontEq] at frontRows
          rw [stateEq, frontierRows, backEq] at backRows
          have firstMember : first ∈ (optionalEntries root).filter (acceptsBound lower false) :=
            frontRows ▸ List.mem_cons_self
          have lastMember : last ∈ (optionalEntries root).filter (acceptsBound upper true) := by
            apply List.mem_reverse.mp
            exact backRows ▸ List.mem_cons_self
          obtain ⟨_, firstAccepted⟩ := List.mem_filter.mp firstMember
          obtain ⟨lastPresent, lastAccepted⟩ := List.mem_filter.mp lastMember
          have notBoth : acceptsBound lower false last ≠ true := by
            intro low
            have inRange : last ∈ selectedRows root lower upper :=
              List.mem_filter.mpr ⟨lastPresent, by simp [low, lastAccepted]⟩
            simp [empty] at inRange
          by_cases crossed : last.key < first.key
          · exact crossed
          · have forward : first.key < last.key ∨ first.key = last.key :=
              List.le_iff_lt_or_eq.mp crossed
            rcases forward with before | same
            · exact False.elim (notBoth (lower_bound_closed lower first last before firstAccepted))
            · exact False.elim (notBoth ((bound_same_key lower false first last same).symm.trans firstAccepted))

theorem init_range_refines (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root) :
    RangeRep (initRange root lower upper) (selectedRows root lower upper) := by
  by_cases empty : selectedRows root lower upper = []
  · simpa [RangeRep, empty] using init_empty_crossed root lower upper good empty
  · rw [RangeRep, if_neg empty]
    obtain ⟨after, front⟩ := selected_front_split root lower upper good
    obtain ⟨before, back⟩ := selected_back_split root lower upper good
    refine ⟨selected_rows_sorted root lower upper good, after, before.reverse, ?_, ?_⟩
    · simpa only [init_front_rows root lower upper good] using front
    · rw [init_back_rows root lower upper good, back, List.reverse_append]

def ValidRangeBounds (lower upper : Option (Key × Bool)) : Prop :=
  match lower, upper with
  | some (lo, loInclusive), some (hi, hiInclusive) =>
      lo ≤ hi ∧ (lo ≠ hi ∨ loInclusive = true ∨ hiInclusive = true)
  | _, _ => True

instance (lower upper : Option (Key × Bool)) : Decidable (ValidRangeBounds lower upper) := by
  cases lower <;> cases upper <;> simp only [ValidRangeBounds] <;> infer_instance

theorem range_bounds_match_source_guard (lo hi : Key) (loInclusive hiInclusive : Bool) :
    ValidRangeBounds (some (lo, loInclusive)) (some (hi, hiInclusive)) ↔
      ¬ hi < lo ∧ ¬ (lo = hi ∧ loInclusive = false ∧ hiInclusive = false) := by
  cases loInclusive <;> cases hiInclusive <;> simp [ValidRangeBounds] <;> rfl

-- None models the public invalid-bounds panic, not an empty successful range.
def openRange (root : Option Tree) (lower upper : Option (Key × Bool)) : Option RangeState :=
  if ValidRangeBounds lower upper then some (initRange root lower upper) else none

theorem open_range_valid (root : Option Tree) (lower upper : Option (Key × Bool)) (good : Good root)
    (valid : ValidRangeBounds lower upper) :
    openRange root lower upper = some (initRange root lower upper) ∧
      RangeRep (initRange root lower upper) (selectedRows root lower upper) := by
  exact ⟨by simp [openRange, valid], init_range_refines root lower upper good⟩

theorem open_range_invalid (root : Option Tree) (lower upper : Option (Key × Bool))
    (invalid : ¬ ValidRangeBounds lower upper) : openRange root lower upper = none := by simp [openRange, invalid]

theorem root_range_history (root : Option Tree) (lower upper : Option (Key × Bool))
    (directions : List Bool) (good : Good root) :
    (rangeRun (initRange root lower upper) directions).1 =
      (rangeSpecRun (selectedRows root lower upper) directions).1 :=
  (range_history_refines _ _ directions (init_range_refines root lower upper good)).1

end Kv9.Radix
