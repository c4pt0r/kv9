import SeekWordLoop

set_option autoImplicit false

namespace Kv9.Radix

structure VectorFrontier where
  current : Option Entry
  pending : List CursorTask

def vectorFrontierModel (cursor : VectorFrontier) : Frontier := ⟨cursor.current, cursor.pending.reverse⟩

def loadVector (reverse : Bool) (pending : List CursorTask) : VectorFrontier :=
  match nextVector reverse pending with
  | none => ⟨none, []⟩
  | some (entry, rest) => ⟨some entry, rest⟩

theorem load_vector_refines (reverse : Bool) (pending : List CursorTask) :
    vectorFrontierModel (loadVector reverse pending) = loadCursor reverse pending.reverse := by
  have relation := next_vector_refines reverse pending
  cases next : nextVector reverse pending with
  | none =>
      have stopped : nextTask reverse pending.reverse = none := by simpa only [next, Option.map_none] using relation.symm
      simp only [loadVector, next, vectorFrontierModel, loadCursor, stopped, List.reverse_nil]
  | some pair =>
      obtain ⟨entry, rest⟩ := pair
      have yielded : nextTask reverse pending.reverse = some (entry, rest.reverse) := by
        simpa only [next, Option.map_some] using relation.symm
      simp only [loadVector, next, vectorFrontierModel, loadCursor, yielded]

structure VectorRangeState where
  front : VectorFrontier
  back : VectorFrontier

def vectorRangeModel (state : VectorRangeState) : RangeState :=
  ⟨vectorFrontierModel state.front, vectorFrontierModel state.back⟩

def clearVectorRange (state : VectorRangeState) : VectorRangeState :=
  ⟨⟨none, state.front.pending⟩, ⟨none, state.back.pending⟩⟩

def vectorRangeStep (reverse : Bool) (state : VectorRangeState) : Option Entry × VectorRangeState :=
  match state.front.current, state.back.current with
  | some first, some last =>
      if last.key < first.key then (none, clearVectorRange state)
      else if first.key = last.key then
        (some (if reverse then last else first), clearVectorRange state)
      else if reverse then
        (some last, ⟨state.front, loadVector true state.back.pending⟩)
      else (some first, ⟨loadVector false state.front.pending, state.back⟩)
  | _, _ => (none, state)

theorem vector_range_step_refines (reverse : Bool) (state : VectorRangeState) :
    ((vectorRangeStep reverse state).1, vectorRangeModel (vectorRangeStep reverse state).2) =
      rangeStep reverse (vectorRangeModel state) := by
  cases front : state.front.current with
  | none => simp [vectorRangeStep, rangeStep, vectorRangeModel, vectorFrontierModel, front]
  | some first =>
      cases back : state.back.current with
      | none => simp [vectorRangeStep, rangeStep, vectorRangeModel, vectorFrontierModel, front, back]
      | some last =>
          by_cases crossed : last.key < first.key
          · simp [vectorRangeStep, rangeStep, vectorRangeModel, vectorFrontierModel, front, back, crossed,
              clearVectorRange, clearRange]
          · by_cases equal : first.key = last.key
            · simp only [vectorRangeStep, rangeStep, vectorRangeModel, vectorFrontierModel, front, back, if_neg crossed, if_pos equal,
                clearVectorRange, clearRange]
            · cases reverse <;>
                simp only [vectorRangeStep, rangeStep, vectorRangeModel, vectorFrontierModel, front, back,
                  if_neg crossed, if_neg equal, Bool.false_eq_true, if_false, if_true]
              · simpa only [vectorFrontierModel, back] using congrArg (fun cursor => (some first, RangeState.mk cursor (vectorFrontierModel state.back)))
                  (load_vector_refines false state.front.pending)
              · simpa only [vectorFrontierModel, front] using congrArg (fun cursor => (some last, RangeState.mk (vectorFrontierModel state.front) cursor))
                  (load_vector_refines true state.back.pending)

def vectorRangeRun (state : VectorRangeState) : List Bool → List (Option Entry) × VectorRangeState
  | [] => ([], state)
  | reverse :: tail =>
      let step := vectorRangeStep reverse state
      let result := vectorRangeRun step.2 tail
      (step.1 :: result.1, result.2)

theorem vector_range_run_refines (state : VectorRangeState) (directions : List Bool) :
    ((vectorRangeRun state directions).1, vectorRangeModel (vectorRangeRun state directions).2) =
      rangeRun (vectorRangeModel state) directions := by
  induction directions generalizing state with
  | nil => rfl
  | cons reverse tail ih =>
      have step := vector_range_step_refines reverse state
      have rest := ih (vectorRangeStep reverse state).2
      have output := congrArg Prod.fst step
      have model := congrArg Prod.snd step
      have resultOutput := congrArg Prod.fst rest
      have resultModel := congrArg Prod.snd rest
      simp only at output model resultOutput resultModel
      simp only [vectorRangeRun, rangeRun, output, resultOutput, resultModel, model]

def initVectorRange (root : Option Tree) (lower upper : Option (Key × Bool)) : Option VectorRangeState :=
  (seekWordVector root lower false).bind (fun front =>
    (seekWordVector root upper true).map (fun back => ⟨loadVector false front, loadVector true back⟩))

theorem init_vector_range_refines (root : Option Tree) (lower upper : Option (Key × Bool))
    (good : Good root) (lowerFits : SeekBoundFits lower) (upperFits : SeekBoundFits upper) :
    (initVectorRange root lower upper).map vectorRangeModel = some (initRange root lower upper) := by
  simp only [initVectorRange, seek_word_vector_refines root lower false good lowerFits,
    seek_word_vector_refines root upper true good upperFits, Option.bind_some, Option.map_some,
    vectorRangeModel, load_vector_refines, List.reverse_reverse, initRange]

theorem vector_root_range_history (root : Option Tree) (lower upper : Option (Key × Bool))
    (state : VectorRangeState) (directions : List Bool) (good : Good root)
    (lowerFits : SeekBoundFits lower) (upperFits : SeekBoundFits upper)
    (opened : initVectorRange root lower upper = some state) :
    (vectorRangeRun state directions).1 = (rangeSpecRun (selectedRows root lower upper) directions).1 := by
  have initialized := init_vector_range_refines root lower upper good lowerFits upperFits
  rw [opened] at initialized
  have model : vectorRangeModel state = initRange root lower upper := Option.some.inj initialized
  have result := congrArg Prod.fst (vector_range_run_refines state directions)
  simpa only [model, root_range_history root lower upper directions good] using result

theorem vector_root_range_no_duplicates (root : Option Tree) (lower upper : Option (Key × Bool))
    (state : VectorRangeState) (directions : List Bool) (good : Good root)
    (lowerFits : SeekBoundFits lower) (upperFits : SeekBoundFits upper)
    (opened : initVectorRange root lower upper = some state) :
    UniqueKeys ((vectorRangeRun state directions).1.filterMap id) := by
  rw [vector_root_range_history root lower upper state directions good lowerFits upperFits opened]
  exact spec_range_no_duplicate_keys _ directions (sorted_unique_keys _ (selected_rows_sorted root lower upper good))

def openVectorRange (root : Option Tree) (lower upper : Option (Key × Bool)) : Option VectorRangeState :=
  if ValidRangeBounds lower upper then initVectorRange root lower upper else none

theorem open_vector_range_refines (root : Option Tree) (lower upper : Option (Key × Bool))
    (good : Good root) (lowerFits : SeekBoundFits lower) (upperFits : SeekBoundFits upper) :
    (openVectorRange root lower upper).map vectorRangeModel = openRange root lower upper := by
  by_cases valid : ValidRangeBounds lower upper
  · simp only [openVectorRange, openRange, if_pos valid, init_vector_range_refines root lower upper good lowerFits upperFits]
  · simp only [openVectorRange, openRange, if_neg valid, Option.map_none]

end Kv9.Radix
