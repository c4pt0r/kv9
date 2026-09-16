import RangeInit

set_option autoImplicit false

namespace Kv9.Radix

theorem consume_rows_conservation (reverse : Bool) (rows : List Entry) :
    ((consumeRows reverse rows).1.toList ++ (consumeRows reverse rows).2).Perm rows := by
  cases reverse with
  | false => cases rows <;> simp [consumeRows]
  | true =>
      cases reverseEq : rows.reverse with
      | nil =>
          have empty : rows = [] := List.reverse_eq_nil_iff.mp reverseEq
          simp [consumeRows, empty]
      | cons entry tail =>
          have rowsEq : rows = tail.reverse ++ [entry] := by
            simpa using congrArg List.reverse reverseEq
          simp only [consumeRows, if_true, reverseEq, List.head?_cons, List.tail_cons, Option.toList_some]
          rw [rowsEq]
          exact List.perm_append_comm

theorem consume_rows_length (reverse : Bool) (rows : List Entry) :
    (consumeRows reverse rows).2.length = rows.length - 1 := by
  cases reverse <;> simp [consumeRows]

theorem spec_range_conservation (rows : List Entry) (directions : List Bool) :
    (((rangeSpecRun rows directions).1.filterMap id) ++ (rangeSpecRun rows directions).2).Perm rows := by
  induction directions generalizing rows with
  | nil => simp [rangeSpecRun]
  | cons reverse tail ih =>
      have rest := ih (consumeRows reverse rows).2
      have lifted := List.Perm.append_left (consumeRows reverse rows).1.toList rest
      have combined := lifted.trans (consume_rows_conservation reverse rows)
      dsimp only [rangeSpecRun]
      cases stepEq : (consumeRows reverse rows).1 <;>
        simpa only [stepEq, List.filterMap_cons, id_eq, Option.toList_none, Option.toList_some,
          List.nil_append, List.singleton_append, List.cons_append] using combined

theorem spec_range_remaining_length (rows : List Entry) (directions : List Bool) :
    (rangeSpecRun rows directions).2.length = rows.length - directions.length := by
  induction directions generalizing rows with
  | nil => simp [rangeSpecRun]
  | cons reverse tail ih =>
      change (rangeSpecRun (consumeRows reverse rows).2 tail).2.length = rows.length - (tail.length + 1)
      rw [ih, consume_rows_length]
      omega

theorem sorted_unique_keys (rows : List Entry) (sorted : SortedEntries rows) : UniqueKeys rows := by
  rw [UniqueKeys, List.Nodup, List.pairwise_map]
  exact sorted.imp (fun {a b} less => key_lt_ne a.key b.key less)

theorem spec_range_no_duplicate_keys (rows : List Entry) (directions : List Bool) (unique : UniqueKeys rows) :
    UniqueKeys ((rangeSpecRun rows directions).1.filterMap id) := by
  have full := (List.Perm.map Entry.key (spec_range_conservation rows directions)).nodup_iff.mpr unique
  rw [List.map_append, List.nodup_append] at full
  exact full.1

theorem root_range_no_duplicate_keys (root : Option Tree) (lower upper : Option (Key × Bool))
    (directions : List Bool) (good : Good root) :
    UniqueKeys ((rangeRun (initRange root lower upper) directions).1.filterMap id) := by
  rw [root_range_history root lower upper directions good]
  exact spec_range_no_duplicate_keys _ directions (sorted_unique_keys _ (selected_rows_sorted root lower upper good))

theorem root_range_exhaustive (root : Option Tree) (lower upper : Option (Key × Bool))
    (directions : List Bool) (good : Good root)
    (enough : (selectedRows root lower upper).length ≤ directions.length) :
    (((rangeRun (initRange root lower upper) directions).1.filterMap id)).Perm (selectedRows root lower upper) := by
  rw [root_range_history root lower upper directions good]
  have remaining := spec_range_remaining_length (selectedRows root lower upper) directions
  have empty : (rangeSpecRun (selectedRows root lower upper) directions).2 = [] := by
    apply List.eq_nil_of_length_eq_zero
    omega
  simpa only [empty, List.append_nil] using spec_range_conservation (selectedRows root lower upper) directions

theorem spec_range_empty (directions : List Bool) :
    (rangeSpecRun [] directions).1 = List.replicate directions.length none ∧
      (rangeSpecRun [] directions).2 = [] := by
  induction directions with
  | nil => exact ⟨rfl, rfl⟩
  | cons reverse tail ih =>
      cases reverse <;> simp [rangeSpecRun, consumeRows, ih.1, ih.2, List.replicate_succ]

theorem range_fused (state : RangeState) (directions : List Bool) (empty : EmptyRange state) :
    (rangeRun state directions).1 = List.replicate directions.length none := by
  have rep : RangeRep state [] := empty
  rw [(range_history_refines state [] directions rep).1]
  exact (spec_range_empty directions).1

theorem accepts_upper_inclusive (query : Key) (entry : Entry) :
    accepts query true true entry = true ↔ entry.key ≤ query := by
  simp only [accepts, Bool.true_and, if_true, Bool.or_eq_true, decide_eq_true_eq]
  exact or_comm.trans List.le_iff_lt_or_eq.symm

theorem predecessor_greatest (root : Option Tree) (query : Key) (entry : Entry) (tail : List CursorTask)
    (good : Good root) (found : nextTask true (seekRoot root (some (query, true)) true) = some (entry, tail)) :
    entry ∈ optionalEntries root ∧ entry.key ≤ query ∧
      ∀ other ∈ optionalEntries root, other.key ≤ query → other.key ≤ entry.key := by
  have rows := seek_next_yield root (some (query, true)) true good entry tail found
  change (((optionalEntries root).filter (accepts query true true)).reverse) = entry :: pendingRows true tail at rows
  have member : entry ∈ (optionalEntries root).filter (accepts query true true) := by
    apply List.mem_reverse.mp
    rw [rows]
    exact List.mem_cons_self
  obtain ⟨present, accepted⟩ := List.mem_filter.mp member
  refine ⟨present, (accepts_upper_inclusive query entry).mp accepted, ?_⟩
  intro other otherMember within
  have filtered : other ∈ (optionalEntries root).filter (accepts query true true) :=
    List.mem_filter.mpr ⟨otherMember, (accepts_upper_inclusive query other).mpr within⟩
  have sorted := filtered_entries_sorted query true true _ (good_root_sorted root good)
  have descending := List.pairwise_reverse.mpr sorted
  rw [rows] at descending
  have reversedMember : other ∈ entry :: pendingRows true tail := by
    rw [← rows]
    exact List.mem_reverse.mpr filtered
  rcases List.mem_cons.mp reversedMember with same | later
  · simp [same]
  · exact List.le_of_lt ((List.pairwise_cons.mp descending).1 other later)

theorem predecessor_absent (root : Option Tree) (query : Key) (good : Good root)
    (absent : nextTask true (seekRoot root (some (query, true)) true) = none) :
    ∀ entry ∈ optionalEntries root, ¬ entry.key ≤ query := by
  have rows := seek_next_empty root (some (query, true)) true good absent
  change (((optionalEntries root).filter (accepts query true true)).reverse) = [] at rows
  have empty := List.reverse_eq_nil_iff.mp rows
  intro entry member within
  have filtered : entry ∈ (optionalEntries root).filter (accepts query true true) :=
    List.mem_filter.mpr ⟨member, (accepts_upper_inclusive query entry).mpr within⟩
  simp [empty] at filtered

def predecessor (root : Option Tree) (query : Key) : Option Entry :=
  (loadCursor true (seekRoot root (some (query, true)) true)).current

theorem predecessor_refines (root : Option Tree) (query : Key) (good : Good root) :
    match predecessor root query with
    | none => ∀ entry ∈ optionalEntries root, ¬ entry.key ≤ query
    | some entry => entry ∈ optionalEntries root ∧ entry.key ≤ query ∧
        ∀ other ∈ optionalEntries root, other.key ≤ query → other.key ≤ entry.key := by
  cases found : nextTask true (seekRoot root (some (query, true)) true) with
  | none => simpa [predecessor, loadCursor, found] using predecessor_absent root query good found
  | some result =>
      obtain ⟨entry, tail⟩ := result
      simpa [predecessor, loadCursor, found] using predecessor_greatest root query entry tail good found

end Kv9.Radix
