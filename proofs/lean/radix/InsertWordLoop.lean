import InsertWordStep

set_option autoImplicit false

namespace Kv9.Radix

theorem execute_word_insert_down_smaller (depth : USize) (fresh : Entry) (tree : Tree) (action : InsertAction)
    (frame : InsertFrame) (child : Tree) (nextDepth : Nat)
    (down : executeWordInsert depth fresh tree action = .down frame child nextDepth) : treeWork child < treeWork tree := by
  cases action with
  | replaceLeaf => cases tree <;> simp [executeWordInsert] at down
  | terminal => cases tree <;> simp [executeWordInsert] at down
  | splitLeaf sharedLength =>
      cases tree with
      | leaf old => cases result : splitLeafWordAt depth sharedLength old fresh <;> simp [executeWordInsert, result] at down
      | branch _ _ _ => simp [executeWordInsert] at down
  | splitBranch sharedLength =>
      cases tree with
      | leaf _ => simp [executeWordInsert] at down
      | branch pfx terminal children =>
          cases result : splitBranchWordAt depth sharedLength pfx terminal children fresh <;> simp [executeWordInsert, result] at down
  | addEdge index =>
      cases tree with
      | leaf _ => simp [executeWordInsert] at down
      | branch pfx terminal children =>
          cases access : fresh.key[(depth + USize.ofNat pfx.length).toNat]? with
          | none => simp only [executeWordInsert, access] at down; cases down
          | some byte =>
              simp only [executeWordInsert, access] at down
              split at down <;> cases down
  | descend index next => exact execute_insert_down_smaller depth.toNat fresh tree (.descend index next) frame child nextDepth down

theorem step_word_insert_down_smaller (depth : USize) (fresh : Entry) (tree : Tree)
    (frame : InsertFrame) (child : Tree) (nextDepth : Nat)
    (down : stepWordInsert depth fresh tree = .down frame child nextDepth) : treeWork child < treeWork tree := by
  unfold stepWordInsert at down
  cases selected : selectWordInsert depth fresh tree with
  | none => simp [selected] at down
  | some action =>
      exact execute_word_insert_down_smaller depth fresh tree action frame child nextDepth (by simpa [selected] using down)

def insertWordLoop (fresh : Entry) (tree : Tree) (depth : USize) (frames : List InsertFrame) : Option (Tree × Bool) :=
  match _transition : stepWordInsert depth fresh tree with
  | .failed => none
  | .done replacement inserted => (plugInsertChecked frames replacement).map (fun root => (root, inserted))
  | .down frame child nextDepth => insertWordLoop fresh child (USize.ofNat nextDepth) (frame :: frames)
termination_by treeWork tree
decreasing_by
  exact step_word_insert_down_smaller depth fresh tree frame child nextDepth _transition

theorem insert_word_loop_refines (path : Key) (fresh : Entry) (tree : Tree) (depth : USize) (frames : List InsertFrame)
    (aligned : depth.toNat = path.length) (valid : Valid path tree) (ordered : OrderedTree tree)
    (freshValid : HasPrefix path fresh.key) (safe : InsertFramesSafe frames)
    (buffers : RoutingBuffersFit tree) (fits : fresh.key.length < USize.size) :
    insertWordLoop fresh tree depth frames =
      some (plugInsertFrames frames (insertTree path fresh tree).1, (insertTree path fresh tree).2) := by
  have stepEq := step_word_insert_refines path depth fresh tree aligned valid freshValid buffers fits
  rw [aligned] at stepEq
  have correct := step_insert_refines path fresh tree valid ordered freshValid
  rw [← stepEq] at correct
  rw [insertWordLoop]
  split
  · rename_i transition
    simp only [transition, InsertStepCorrect] at correct
  · rename_i replacement inserted transition
    have result : (replacement, inserted) = insertTree path fresh tree := by
      simpa only [transition, InsertStepCorrect] using correct
    rw [insert_context_checked frames replacement safe]
    simp only [Option.map_some, ← result]
  · rename_i frame child nextDepth transition
    have smaller := step_word_insert_down_smaller depth fresh tree frame child nextDepth transition
    obtain ⟨childPath, childValid, childOrdered, childPrefix, depthEq, indexSafe, parentEq, access, result⟩ :=
      (by simpa only [transition, InsertStepCorrect] using correct :
        ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath fresh.key ∧
          nextDepth = childPath.length ∧ frame.index < edgeCount frame.children ∧
          tree = .branch frame.pfx frame.terminal frame.children ∧
          (∃ byte, edgeGet frame.index frame.children = some (byte, child)) ∧
          insertTree path fresh tree =
            (plugInsert frame (insertTree childPath fresh child).1, (insertTree childPath fresh child).2))
    obtain ⟨byte, found⟩ := access
    have childBuffers : RoutingBuffersFit child := by
      rw [parentEq] at buffers
      exact edge_get_buffers_fit frame.children frame.index byte child buffers.2 found
    have nextFits : nextDepth < USize.size := by
      have bound := prefix_length_bound childPath fresh.key childPrefix
      omega
    have nextAligned : (USize.ofNat nextDepth).toNat = childPath.length :=
      (USize.toNat_ofNat_of_lt nextFits).trans depthEq
    rw [insert_word_loop_refines childPath fresh child (USize.ofNat nextDepth) (frame :: frames)
      nextAligned childValid childOrdered childPrefix ⟨indexSafe, safe⟩ childBuffers fits]
    simp only [plugInsertFrames, result]
termination_by treeWork tree
decreasing_by
  assumption

def putWordLoop (fresh : Entry) : Option Tree → Option (Tree × Bool)
  | none => some (.leaf fresh, true)
  | some tree => insertWordLoop fresh tree 0 []

theorem put_word_loop_refines (fresh : Entry) (root : Option Tree) (good : Good root)
    (buffers : ∀ tree, root = some tree → RoutingBuffersFit tree) (fits : fresh.key.length < USize.size) :
    putWordLoop fresh root = some (putRoot fresh root) := by
  cases root with
  | none => rfl
  | some tree =>
      exact insert_word_loop_refines [] fresh tree 0 [] rfl good.1 (good.2 tree rfl).2
        ⟨fresh.key, rfl⟩ trivial (buffers tree rfl) fits

def putWordState (state : WordCounted) (fresh : Entry) : Option (WordCounted × Bool) :=
  match state.root with
  | none => some (⟨some (.leaf fresh), 1⟩, true)
  | some tree => (insertWordLoop fresh tree 0 []).bind
      (fun result =>
        if state.size.toNat + result.2.toUSize.toNat < USize.size then
          some (⟨some result.1, state.size + result.2.toUSize⟩, result.2)
        else none)

theorem put_word_count_add_fits (state : WordCounted) (fresh : Entry) (good : WordGood state)
    (resident : (entries (putRoot fresh state.root).1).length < USize.size) :
    state.size.toNat + (putRoot fresh state.root).2.toUSize.toNat < USize.size := by
  have count := put_root_cardinality fresh state.root good.1
  have bit : (putRoot fresh state.root).2.toUSize.toNat = (if (putRoot fresh state.root).2 then 1 else 0) := by
    cases (putRoot fresh state.root).2 <;> simp [Bool.toUSize]
  rw [good.2, bit, ← count]
  exact resident

theorem put_word_state_refines (state : WordCounted) (fresh : Entry) (good : WordGood state)
    (buffers : ∀ tree, state.root = some tree → RoutingBuffersFit tree) (fits : fresh.key.length < USize.size)
    (resident : (entries (putRoot fresh state.root).1).length < USize.size) :
    putWordState state fresh = some (wordPut state fresh) := by
  cases rootEq : state.root with
  | none => simp [putWordState, wordPut, rootEq]
  | some tree =>
      have rootGood : Good (some tree) := by simpa only [rootEq] using good.1
      have loop := insert_word_loop_refines [] fresh tree 0 [] rfl rootGood.1 (rootGood.2 tree rfl).2
        ⟨fresh.key, rfl⟩ trivial (buffers tree rootEq) fits
      have sumFits : state.size.toNat + (insertTree [] fresh tree).2.toUSize.toNat < USize.size := by
        simpa only [rootEq, putRoot] using put_word_count_add_fits state fresh good resident
      simp only [putWordState, rootEq, loop, plugInsertFrames, Option.bind_some, if_pos sumFits, wordPut]

theorem put_word_state_good (state result : WordCounted) (fresh : Entry) (inserted : Bool)
    (good : WordGood state) (buffers : ∀ tree, state.root = some tree → RoutingBuffersFit tree)
    (fits : fresh.key.length < USize.size) (resident : (entries (putRoot fresh state.root).1).length < USize.size)
    (completed : putWordState state fresh = some (result, inserted)) : WordGood result := by
  rw [put_word_state_refines state fresh good buffers fits resident] at completed
  have same := congrArg Prod.fst (Option.some.inj completed)
  simpa only [same] using word_put_good state fresh good resident

theorem put_word_state_no_failure (state : WordCounted) (fresh : Entry) (good : WordGood state)
    (buffers : ∀ tree, state.root = some tree → RoutingBuffersFit tree) (fits : fresh.key.length < USize.size)
    (resident : (entries (putRoot fresh state.root).1).length < USize.size) :
    putWordState state fresh ≠ none := by
  rw [put_word_state_refines state fresh good buffers fits resident]
  simp

end Kv9.Radix
