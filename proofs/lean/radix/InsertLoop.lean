import InsertStep

set_option autoImplicit false

namespace Kv9.Radix

def plugInsertFrames : List InsertFrame → Tree → Tree
  | [], child => child
  | frame :: tail, child => plugInsertFrames tail (plugInsert frame child)

def InsertFramesSafe : List InsertFrame → Prop
  | [] => True
  | frame :: tail => frame.index < edgeCount frame.children ∧ InsertFramesSafe tail

def plugInsertChecked : List InsertFrame → Tree → Option Tree
  | [], child => some child
  | frame :: tail, child =>
      if frame.index < edgeCount frame.children then plugInsertChecked tail (plugInsert frame child)
      else none

theorem insert_context_checked (frames : List InsertFrame) (child : Tree) (safe : InsertFramesSafe frames) :
    plugInsertChecked frames child = some (plugInsertFrames frames child) := by
  induction frames generalizing child with
  | nil => rfl
  | cons frame tail ih =>
      rw [plugInsertChecked, if_pos safe.1]
      exact ih (plugInsert frame child) safe.2

theorem insert_context_compose (a b : List InsertFrame) (child : Tree) :
    plugInsertFrames (a ++ b) child = plugInsertFrames b (plugInsertFrames a child) := by
  induction a generalizing child with
  | nil => rfl
  | cons frame tail ih => exact ih (plugInsert frame child)

-- Only the selected child is revisited. Frames represent the value context
-- of Rust's nested &mut slot; they are not claimed to be runtime allocations.
def insertLoop (fresh : Entry) (tree : Tree) (depth : Nat) (frames : List InsertFrame) : Option (Tree × Bool) :=
  match _transition : stepInsert depth fresh tree with
  | .failed => none
  | .done replacement inserted => (plugInsertChecked frames replacement).map (fun root => (root, inserted))
  | .down frame child nextDepth => insertLoop fresh child nextDepth (frame :: frames)
termination_by treeWork tree
decreasing_by
  exact step_insert_down_smaller depth fresh tree frame child nextDepth _transition

theorem insert_loop_refines (path : Key) (fresh : Entry) (tree : Tree) (frames : List InsertFrame)
    (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key)
    (safe : InsertFramesSafe frames) :
    insertLoop fresh tree path.length frames =
      some (plugInsertFrames frames (insertTree path fresh tree).1, (insertTree path fresh tree).2) := by
  have correct := step_insert_refines path fresh tree valid ordered freshValid
  rw [insertLoop]
  split
  · rename_i transition
    simp only [transition, InsertStepCorrect] at correct
  · rename_i replacement inserted transition
    have result : (replacement, inserted) = insertTree path fresh tree := by
      simpa only [transition, InsertStepCorrect] using correct
    rw [insert_context_checked frames replacement safe]
    simp only [Option.map_some, ← result]
  · rename_i frame child nextDepth transition
    have smaller := step_insert_down_smaller path.length fresh tree frame child nextDepth transition
    obtain ⟨childPath, childValid, childOrdered, childPrefix, depthEq, indexSafe, _parentEq, _access, result⟩ :=
      (by simpa only [transition, InsertStepCorrect] using correct :
        ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath fresh.key ∧
          nextDepth = childPath.length ∧ frame.index < edgeCount frame.children ∧
          tree = .branch frame.pfx frame.terminal frame.children ∧
          (∃ byte, edgeGet frame.index frame.children = some (byte, child)) ∧
          insertTree path fresh tree =
            (plugInsert frame (insertTree childPath fresh child).1, (insertTree childPath fresh child).2))
    rw [depthEq, insert_loop_refines childPath fresh child (frame :: frames) childValid childOrdered childPrefix ⟨indexSafe, safe⟩]
    simp only [plugInsertFrames, result]
termination_by treeWork tree
decreasing_by
  assumption

def putLoop (fresh : Entry) : Option Tree → Option (Tree × Bool)
  | none => some (.leaf fresh, true)
  | some tree => insertLoop fresh tree 0 []

theorem put_loop_refines (fresh : Entry) (root : Option Tree) (good : Good root) :
    putLoop fresh root = some (putRoot fresh root) := by
  cases root with
  | none => rfl
  | some tree =>
      exact insert_loop_refines [] fresh tree [] good.1 (good.2 tree rfl).2
        ⟨fresh.key, rfl⟩ trivial

theorem put_loop_no_failure (fresh : Entry) (root : Option Tree) (good : Good root) : putLoop fresh root ≠ none := by
  rw [put_loop_refines fresh root good]
  simp

theorem put_loop_good (fresh : Entry) (root : Option Tree) (result : Tree) (inserted : Bool)
    (good : Good root) (completed : putLoop fresh root = some (result, inserted)) : Good (some result) := by
  rw [put_loop_refines fresh root good] at completed
  have pair := Option.some.inj completed
  have same := congrArg Prod.fst pair
  simpa only [same] using put_root_good fresh root good

theorem put_loop_lookup (fresh : Entry) (root : Option Tree) (query : Key) (good : Good root) :
    (putLoop fresh root).map (fun result => optionalLookup query (some result.1)) =
      some (if query = fresh.key then some fresh.value else optionalLookup query root) := by
  rw [put_loop_refines fresh root good]
  simp only [Option.map_some, optionalLookup]
  exact congrArg some (put_root_lookup fresh root query good.1 (fun tree eq => (good.2 tree eq).2))

end Kv9.Radix
