import SeekStep

set_option autoImplicit false

namespace Kv9.Radix

-- Pending tasks remain in physical Vec order throughout the source loop.
def seekLoop (query : Key) (inclusive reverse : Bool) (tree : Tree) (depth : Nat)
    (pending : List CursorTask) : Option (List CursorTask) :=
  match _transition : stepSeek depth query inclusive reverse tree with
  | .failed => none
  | .done pushes => some (pending ++ pushes)
  | .down pushes child nextDepth => seekLoop query inclusive reverse child nextDepth (pending ++ pushes)
termination_by treeWork tree
decreasing_by
  exact step_seek_down_smaller depth query inclusive reverse tree pushes child nextDepth _transition

theorem seek_loop_refines (path query : Key) (inclusive reverse : Bool) (tree : Tree)
    (pending : List CursorTask) (valid : Valid path tree) (ordered : OrderedTree tree)
    (queryValid : HasPrefix path query) :
    seekLoop query inclusive reverse tree path.length pending =
      some (pending ++ (seekTree path query inclusive reverse tree).reverse) := by
  have correct := step_seek_refines path query inclusive reverse tree valid ordered queryValid
  rw [seekLoop]
  split
  · rename_i transition
    simp only [transition, SeekStepCorrect] at correct
  · rename_i pushes transition
    have result : pushes.reverse = seekTree path query inclusive reverse tree := by
      simpa only [transition, SeekStepCorrect] using correct
    rw [← result, List.reverse_reverse]
  · rename_i pushes child nextDepth transition
    have smaller := step_seek_down_smaller path.length query inclusive reverse tree pushes child nextDepth transition
    obtain ⟨childPath, childValid, childOrdered, childPrefix, depthEq, result⟩ :=
      (by simpa only [transition, SeekStepCorrect] using correct :
        ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath query ∧
          nextDepth = childPath.length ∧
          seekTree path query inclusive reverse tree =
            seekTree childPath query inclusive reverse child ++ pushes.reverse)
    rw [depthEq, seek_loop_refines childPath query inclusive reverse child (pending ++ pushes)
      childValid childOrdered childPrefix]
    simp only [result, List.reverse_append, List.reverse_reverse, List.append_assoc]
termination_by treeWork tree
decreasing_by
  assumption

def seekVector (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) : Option (List CursorTask) :=
  match root, bound with
  | none, _ => some []
  | some tree, none => some [.node tree]
  | some tree, some (query, inclusive) => seekLoop query inclusive reverse tree 0 []

theorem seek_vector_refines (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) (good : Good root) :
    seekVector root bound reverse = some (seekRoot root bound reverse).reverse := by
  cases root with
  | none => cases bound <;> rfl
  | some tree =>
      cases bound with
      | none => rfl
      | some pair =>
          obtain ⟨query, inclusive⟩ := pair
          exact seek_loop_refines [] query inclusive reverse tree [] good.1 (good.2 tree rfl).2 ⟨query, rfl⟩

theorem seek_vector_no_failure (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) (good : Good root) :
    seekVector root bound reverse ≠ none := by
  rw [seek_vector_refines root bound reverse good]
  simp

theorem seek_vector_rows (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending : List CursorTask) (good : Good root) (completed : seekVector root bound reverse = some pending) :
    pendingRows reverse pending.reverse = boundRows root bound reverse := by
  rw [seek_vector_refines root bound reverse good] at completed
  have same := Option.some.inj completed
  rw [← same, List.reverse_reverse]
  exact seek_root_rows root bound reverse good

theorem seek_vector_next_yield (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending rest : List CursorTask) (entry : Entry) (good : Good root)
    (completed : seekVector root bound reverse = some pending)
    (yielded : nextVector reverse pending = some (entry, rest)) :
    boundRows root bound reverse = entry :: pendingRows reverse rest.reverse := by
  rw [← seek_vector_rows root bound reverse pending good completed]
  exact next_vector_yield reverse pending rest entry yielded

theorem seek_vector_next_empty (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending : List CursorTask) (good : Good root) (completed : seekVector root bound reverse = some pending)
    (empty : nextVector reverse pending = none) : boundRows root bound reverse = [] := by
  rw [← seek_vector_rows root bound reverse pending good completed]
  exact next_vector_empty reverse pending empty

end Kv9.Radix
