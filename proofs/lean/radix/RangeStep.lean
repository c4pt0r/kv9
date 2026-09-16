import CursorSeek

set_option autoImplicit false

namespace Kv9.Radix

structure Frontier where
  current : Option Entry
  pending : List CursorTask

def loadCursor (reverse : Bool) (pending : List CursorTask) : Frontier :=
  match nextTask reverse pending with
  | none => ⟨none, []⟩
  | some (entry, tail) => ⟨some entry, tail⟩

def frontierRows (reverse : Bool) (cursor : Frontier) : List Entry :=
  match cursor.current with
  | none => []
  | some entry => entry :: pendingRows reverse cursor.pending

theorem load_cursor_rows (reverse : Bool) (pending : List CursorTask) :
    frontierRows reverse (loadCursor reverse pending) = pendingRows reverse pending := by
  cases nextEq : nextTask reverse pending with
  | none => simpa [loadCursor, nextEq, frontierRows] using (next_task_empty reverse pending nextEq).symm
  | some result =>
      obtain ⟨entry, tail⟩ := result
      simpa [loadCursor, nextEq, frontierRows] using (next_task_yield reverse pending tail entry nextEq).symm

theorem frontier_cons (reverse : Bool) (cursor : Frontier) (entry : Entry) (tail : List Entry)
    (rows : frontierRows reverse cursor = entry :: tail) :
    cursor.current = some entry ∧ pendingRows reverse cursor.pending = tail := by
  cases currentEq : cursor.current with
  | none => simp [frontierRows, currentEq] at rows
  | some found =>
      have eqs : found = entry ∧ pendingRows reverse cursor.pending = tail := by
        simpa [frontierRows, currentEq] using rows
      exact ⟨by simp [eqs.1], eqs.2⟩

theorem frontier_member_prefix (reverse : Bool) (cursor : Frontier) (rows extra : List Entry)
    (nonempty : rows ≠ []) (same : frontierRows reverse cursor = rows ++ extra) :
    ∃ entry, cursor.current = some entry ∧ entry ∈ rows := by
  cases rows with
  | nil => exact False.elim (nonempty rfl)
  | cons entry tail =>
      exact ⟨entry, (frontier_cons reverse cursor entry (tail ++ extra) same).1, by simp⟩

structure RangeState where
  front : Frontier
  back : Frontier

def clearRange (state : RangeState) : RangeState :=
  ⟨⟨none, state.front.pending⟩, ⟨none, state.back.pending⟩⟩

def rangeStep (reverse : Bool) (state : RangeState) : Option Entry × RangeState :=
  match state.front.current, state.back.current with
  | some first, some last =>
      if last.key < first.key then (none, clearRange state)
      else if first.key = last.key then
        (some (if reverse then last else first), clearRange state)
      else if reverse then
        (some last, ⟨state.front, loadCursor true state.back.pending⟩)
      else (some first, ⟨loadCursor false state.front.pending, state.back⟩)
  | _, _ => (none, state)

def EmptyRange (state : RangeState) : Prop :=
  state.front.current = none ∨ state.back.current = none ∨
    ∃ first last, state.front.current = some first ∧ state.back.current = some last ∧ last.key < first.key

def LiveRange (state : RangeState) (rows : List Entry) : Prop :=
  SortedEntries rows ∧ ∃ after before,
    frontierRows false state.front = rows ++ after ∧
    frontierRows true state.back = rows.reverse ++ before

def RangeRep (state : RangeState) (rows : List Entry) : Prop :=
  if rows = [] then EmptyRange state else LiveRange state rows

theorem clear_range_empty (state : RangeState) : EmptyRange (clearRange state) := Or.inl rfl

theorem empty_range_step (reverse : Bool) (state : RangeState) (empty : EmptyRange state) :
    (rangeStep reverse state).1 = none ∧ EmptyRange (rangeStep reverse state).2 := by
  rcases empty with front | back | ⟨first, last, front, back, crossed⟩
  · have stopped : EmptyRange state := Or.inl front
    simp [rangeStep, front, stopped]
  · have stopped : EmptyRange state := Or.inr (Or.inl back)
    cases frontEq : state.front.current <;> simp [rangeStep, frontEq, back, stopped]
  · simp [rangeStep, front, back, crossed, clear_range_empty]

theorem range_front_refines (state : RangeState) (rows : List Entry) (rep : RangeRep state rows) :
    (rangeStep false state).1 = rows.head? ∧ RangeRep (rangeStep false state).2 rows.tail := by
  cases rows with
  | nil => simpa [RangeRep] using empty_range_step false state rep
  | cons first tail =>
      obtain ⟨sorted, after, before, frontRows, backRows⟩ := (by simpa [RangeRep, LiveRange] using rep : LiveRange state (first :: tail))
      have front := frontier_cons false state.front first (tail ++ after) frontRows
      cases tail with
      | nil =>
          have back := frontier_cons true state.back first before (by simpa using backRows)
          simp [rangeStep, front.1, back.1, List.lt_irrefl, RangeRep, clear_range_empty]
      | cons next rest =>
          have backPrefix : frontierRows true state.back = (next :: rest).reverse ++ (first :: before) := by
            simpa [List.reverse_cons, List.append_assoc] using backRows
          obtain ⟨last, back, lastMember⟩ := frontier_member_prefix true state.back (next :: rest).reverse
            (first :: before) (by simp) backPrefix
          have member : last ∈ next :: rest := by simpa only [List.mem_reverse] using lastMember
          have less : first.key < last.key := (List.pairwise_cons.mp sorted).1 last member
          have notCrossed := List.lt_asymm less
          have different := key_lt_ne _ _ less
          simp only [rangeStep, front.1, back, if_neg notCrossed, if_neg different, Bool.false_eq_true,
            if_false, List.head?_cons, List.tail_cons, true_and]
          change RangeRep ⟨loadCursor false state.front.pending, state.back⟩ (next :: rest)
          rw [RangeRep, if_neg (by simp)]
          refine ⟨(List.pairwise_cons.mp sorted).2, after, first :: before, ?_, backPrefix⟩
          simpa only [load_cursor_rows] using front.2

theorem range_back_refines (state : RangeState) (rows : List Entry) (rep : RangeRep state rows) :
    (rangeStep true state).1 = rows.reverse.head? ∧
      RangeRep (rangeStep true state).2 rows.reverse.tail.reverse := by
  cases reverseEq : rows.reverse with
  | nil =>
      have emptyRows : rows = [] := List.reverse_eq_nil_iff.mp reverseEq
      subst rows
      simpa [RangeRep] using empty_range_step true state rep
  | cons last tail =>
      have rowsEq : rows = (last :: tail).reverse := by
        simpa using congrArg List.reverse reverseEq
      have nonempty : rows ≠ [] := by rw [rowsEq]; simp
      obtain ⟨sorted, after, before, frontRows, backRows⟩ := (by simpa [RangeRep, nonempty] using rep : LiveRange state rows)
      have back := frontier_cons true state.back last (tail ++ before) (by simpa [reverseEq] using backRows)
      cases tail with
      | nil =>
          have front := frontier_cons false state.front last after (by simpa [rowsEq] using frontRows)
          simp [rangeStep, front.1, back.1, List.lt_irrefl, RangeRep, clear_range_empty]
      | cons next rest =>
          have frontPrefix : frontierRows false state.front = (next :: rest).reverse ++ (last :: after) := by
            simpa [rowsEq, List.append_assoc] using frontRows
          obtain ⟨first, front, firstMember⟩ := frontier_member_prefix false state.front (next :: rest).reverse
            (last :: after) (by simp) frontPrefix
          have splitSorted : SortedEntries ((next :: rest).reverse ++ [last]) := by simpa [rowsEq] using sorted
          have pieces := List.pairwise_append.mp splitSorted
          have less : first.key < last.key := pieces.2.2 first firstMember last (by simp)
          have notCrossed := List.lt_asymm less
          have different := key_lt_ne _ _ less
          simp only [rangeStep, front, back.1, if_neg notCrossed, if_neg different, if_true,
            List.head?_cons, List.tail_cons, true_and]
          change RangeRep ⟨state.front, loadCursor true state.back.pending⟩ (next :: rest).reverse
          rw [RangeRep, if_neg (by simp)]
          refine ⟨pieces.1, last :: after, before, frontPrefix, ?_⟩
          simpa only [load_cursor_rows, List.reverse_reverse] using back.2

def consumeRows (reverse : Bool) (rows : List Entry) : Option Entry × List Entry :=
  if reverse then (rows.reverse.head?, rows.reverse.tail.reverse) else (rows.head?, rows.tail)

theorem range_step_refines (reverse : Bool) (state : RangeState) (rows : List Entry) (rep : RangeRep state rows) :
    (rangeStep reverse state).1 = (consumeRows reverse rows).1 ∧
      RangeRep (rangeStep reverse state).2 (consumeRows reverse rows).2 := by
  cases reverse with
  | false => exact range_front_refines state rows rep
  | true => exact range_back_refines state rows rep

def rangeRun (state : RangeState) : List Bool → List (Option Entry) × RangeState
  | [] => ([], state)
  | reverse :: tail =>
      let step := rangeStep reverse state
      let result := rangeRun step.2 tail
      (step.1 :: result.1, result.2)

def rangeSpecRun (rows : List Entry) : List Bool → List (Option Entry) × List Entry
  | [] => ([], rows)
  | reverse :: tail =>
      let step := consumeRows reverse rows
      let result := rangeSpecRun step.2 tail
      (step.1 :: result.1, result.2)

theorem range_history_refines (state : RangeState) (rows : List Entry) (directions : List Bool)
    (rep : RangeRep state rows) :
    (rangeRun state directions).1 = (rangeSpecRun rows directions).1 ∧
      RangeRep (rangeRun state directions).2 (rangeSpecRun rows directions).2 := by
  induction directions generalizing state rows with
  | nil => exact ⟨rfl, rep⟩
  | cons reverse tail ih =>
      have step := range_step_refines reverse state rows rep
      have rest := ih (rangeStep reverse state).2 (consumeRows reverse rows).2 step.2
      constructor
      · simp only [rangeRun, rangeSpecRun, step.1, rest.1]
      · exact rest.2

end Kv9.Radix
