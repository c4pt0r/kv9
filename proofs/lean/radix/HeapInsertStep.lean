import HeapInsertSelect

set_option autoImplicit false

namespace Kv9.Radix

inductive PlaceInsertStep where
  | failed
  | done (next : MutHeap) (inserted : Bool)
  | down (next : MutHeap) (depth : Nat)

def executePlaceInsert (state : MutHeap) (others : List NodeId) (depth : Nat) (fresh : Entry) : InsertAction → PlaceInsertStep
  | .replaceLeaf => match replaceLeafPlace state others fresh with
      | none => .failed
      | some next => .done next false
  | .splitLeaf sharedLength => match splitLeafPlace state others depth sharedLength fresh with
      | none => .failed
      | some next => .done next true
  | .splitBranch sharedLength => match splitBranchPlace state others depth sharedLength fresh with
      | none => .failed
      | some next => .done next true
  | .terminal => match terminalPlace state others fresh with
      | none => .failed
      | some (next, inserted) => .done next inserted
  | .addEdge index => match addEdgePlace state others depth index fresh with
      | none => .failed
      | some (next, inserted) => .done next inserted
  | .descend index nextDepth => match descendPlace state others index with
      | none => .failed
      | some next => .down next nextDepth

def stepPlaceInsert (state : MutHeap) (others : List NodeId) (depth : Nat) (fresh : Entry) : PlaceInsertStep :=
  match selectPlaceInsert state depth fresh with
  | none => .failed
  | some action => executePlaceInsert state others depth fresh action

-- Failure is not asserted equivalent on invalid source preconditions. Valid
-- abstract insertion never fails, and each successful transition is simulated.
def PlaceStepRep (before : MutHeap) (others : List NodeId) (frames : List HeapFrame) : InsertStep → PlaceInsertStep → Prop
  | .failed, _ => True
  | .done replacement inserted, actual =>
      ∃ next, actual = .done next inserted ∧ MutationResult before others frames replacement next
  | .down frame child nextDepth, actual =>
      ∃ next address nextFrames, actual = .down next nextDepth ∧ MutationInvariant next others address child nextFrames ∧
        nextFrames.map HeapFrame.value = frame :: frames.map HeapFrame.value ∧
        (∀ saved value, saved ∈ others → NodeRep before.heap saved value → NodeRep next.heap saved value)

theorem execute_place_insert_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (depth : Nat) (fresh : Entry) (action : InsertAction)
    (inv : MutationInvariant state others focus tree frames) :
    PlaceStepRep state others frames (executeInsert depth fresh tree action) (executePlaceInsert state others depth fresh action) := by
  cases action with
  | replaceLeaf =>
      cases tree with
      | branch => trivial
      | leaf old =>
          obtain ⟨next, completed, result⟩ := replace_leaf_place_result state others focus old fresh frames inv
          exact ⟨next, by simp [executePlaceInsert, completed], result⟩
  | splitLeaf sharedLength =>
      cases tree with
      | branch => trivial
      | leaf old =>
          cases result : splitLeafAt depth sharedLength old fresh with
          | none => simp only [executeInsert, result, PlaceStepRep]
          | some replacement =>
              obtain ⟨next, completed, correct⟩ := split_leaf_place_refines state others focus old fresh frames depth sharedLength replacement inv result
              simp only [executeInsert, result, PlaceStepRep]
              exact ⟨next, by simp [executePlaceInsert, completed], correct⟩
  | splitBranch sharedLength =>
      cases tree with
      | leaf => trivial
      | branch pfx terminal children =>
          cases result : splitBranchAt depth sharedLength pfx terminal children fresh with
          | none => simp only [executeInsert, result, PlaceStepRep]
          | some replacement =>
              obtain ⟨next, completed, correct⟩ := split_branch_place_refines state others focus pfx terminal children frames depth sharedLength fresh replacement inv result
              simp only [executeInsert, result, PlaceStepRep]
              exact ⟨next, by simp [executePlaceInsert, completed], correct⟩
  | terminal =>
      cases tree with
      | leaf => trivial
      | branch pfx terminal children =>
          obtain ⟨next, completed, result⟩ := terminal_place_refines state others focus pfx terminal children frames fresh inv
          exact ⟨next, by simp [executePlaceInsert, completed], result⟩
  | addEdge index =>
      cases tree with
      | leaf => trivial
      | branch pfx terminal children =>
          cases access : fresh.key[depth + pfx.length]? with
          | none => simp only [executeInsert, access, PlaceStepRep]
          | some byte =>
              by_cases bounded : index ≤ edgeCount children
              · obtain ⟨next, completed, result⟩ := add_edge_place_refines state others focus pfx terminal children frames depth index byte fresh inv access bounded
                simp only [executeInsert, access, if_pos bounded, PlaceStepRep]
                exact ⟨next, by simp [executePlaceInsert, completed], result⟩
              · simp only [executeInsert, access, if_neg bounded, PlaceStepRep]
  | descend index nextDepth =>
      cases tree with
      | leaf => trivial
      | branch pfx terminal children =>
          cases access : edgeGet index children with
          | none => simp only [executeInsert, access, PlaceStepRep]
          | some pair =>
              obtain ⟨byte, child⟩ := pair
              obtain ⟨next, address, nextFrames, completed, nextInv, values, preserve⟩ :=
                descend_place_refines state others focus pfx terminal children frames index byte child inv access
              simp only [executeInsert, access, PlaceStepRep]
              exact ⟨next, address, nextFrames, by simp [executePlaceInsert, completed], nextInv, values, preserve⟩

theorem step_place_insert_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (depth : Nat) (fresh : Entry)
    (inv : MutationInvariant state others focus tree frames) :
    PlaceStepRep state others frames (stepInsert depth fresh tree) (stepPlaceInsert state others depth fresh) := by
  rw [stepPlaceInsert, select_place_insert_refines state others focus tree frames depth fresh inv, stepInsert]
  cases selected : selectInsert depth fresh tree with
  | none => trivial
  | some action => exact execute_place_insert_refines state others focus tree frames depth fresh action inv

theorem execute_place_down_action (state next : MutHeap) (others : List NodeId) (depth nextDepth : Nat)
    (fresh : Entry) (action : InsertAction)
    (step : executePlaceInsert state others depth fresh action = .down next nextDepth) :
    ∃ index, action = .descend index nextDepth := by
  cases action with
  | replaceLeaf => cases result : replaceLeafPlace state others fresh <;> simp [executePlaceInsert, result] at step
  | splitLeaf commonLength => cases result : splitLeafPlace state others depth commonLength fresh <;> simp [executePlaceInsert, result] at step
  | splitBranch commonLength => cases result : splitBranchPlace state others depth commonLength fresh <;> simp [executePlaceInsert, result] at step
  | terminal =>
      cases result : terminalPlace state others fresh with
      | none => simp [executePlaceInsert, result] at step
      | some pair => obtain ⟨state, inserted⟩ := pair; simp [executePlaceInsert, result] at step
  | addEdge index =>
      cases result : addEdgePlace state others depth index fresh with
      | none => simp [executePlaceInsert, result] at step
      | some pair => obtain ⟨state, inserted⟩ := pair; simp [executePlaceInsert, result] at step
  | descend index targetDepth =>
      cases result : descendPlace state others index with
      | none => simp [executePlaceInsert, result] at step
      | some advanced =>
          have same : advanced = next ∧ targetDepth = nextDepth := by simpa only [executePlaceInsert, result, PlaceInsertStep.down.injEq] using step
          exact ⟨index, by rw [same.2]⟩

theorem step_place_down_bounds (state next : MutHeap) (others : List NodeId) (depth nextDepth : Nat) (fresh : Entry)
    (step : stepPlaceInsert state others depth fresh = .down next nextDepth) :
    nextDepth ≤ fresh.key.length ∧ depth < nextDepth ∧ fresh.key.length - nextDepth < fresh.key.length - depth := by
  cases selected : selectPlaceInsert state depth fresh with
  | none => simp [stepPlaceInsert, selected] at step
  | some action =>
      have actual : executePlaceInsert state others depth fresh action = .down next nextDepth := by simpa only [stepPlaceInsert, selected] using step
      obtain ⟨index, rfl⟩ := execute_place_down_action state next others depth nextDepth fresh action actual
      exact select_place_descend_bounds state depth fresh index nextDepth selected

end Kv9.Radix
