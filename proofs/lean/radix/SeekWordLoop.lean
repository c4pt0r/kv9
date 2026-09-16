import SeekWordStep

set_option autoImplicit false

namespace Kv9.Radix

def seekWordLoop (query : Key) (inclusive reverse : Bool) (tree : Tree) (depth : USize)
    (pending : List CursorTask) : Option (List CursorTask) :=
  match _transition : stepWordSeek depth query inclusive reverse tree with
  | .failed => none
  | .done pushes => some (pending ++ pushes)
  | .down pushes child nextDepth => seekWordLoop query inclusive reverse child (USize.ofNat nextDepth) (pending ++ pushes)
termination_by treeWork tree
decreasing_by
  exact step_word_seek_down_smaller depth query inclusive reverse tree pushes child nextDepth _transition

theorem seek_word_loop_refines (path query : Key) (inclusive reverse : Bool) (tree : Tree)
    (depth : USize) (pending : List CursorTask) (aligned : depth.toNat = path.length)
    (valid : Valid path tree) (ordered : OrderedTree tree) (queryValid : HasPrefix path query)
    (fits : query.length < USize.size) :
    seekWordLoop query inclusive reverse tree depth pending =
      some (pending ++ (seekTree path query inclusive reverse tree).reverse) := by
  have bounded : depth.toNat ≤ query.length := by
    rw [aligned]
    exact prefix_length_bound path query queryValid
  have stepEq := step_word_seek_refines depth query inclusive reverse tree ordered bounded fits
  rw [aligned] at stepEq
  have correct := step_seek_refines path query inclusive reverse tree valid ordered queryValid
  rw [← stepEq] at correct
  rw [seekWordLoop]
  split
  · rename_i transition
    simp only [transition, SeekStepCorrect] at correct
  · rename_i pushes transition
    have result : pushes.reverse = seekTree path query inclusive reverse tree := by
      simpa only [transition, SeekStepCorrect] using correct
    rw [← result, List.reverse_reverse]
  · rename_i pushes child nextDepth transition
    have smaller := step_word_seek_down_smaller depth query inclusive reverse tree pushes child nextDepth transition
    obtain ⟨childPath, childValid, childOrdered, childPrefix, depthEq, result⟩ :=
      (by simpa only [transition, SeekStepCorrect] using correct :
        ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath query ∧
          nextDepth = childPath.length ∧
          seekTree path query inclusive reverse tree =
            seekTree childPath query inclusive reverse child ++ pushes.reverse)
    have nextFits : nextDepth < USize.size := by
      have bound := prefix_length_bound childPath query childPrefix
      omega
    have nextAligned : (USize.ofNat nextDepth).toNat = childPath.length :=
      (USize.toNat_ofNat_of_lt nextFits).trans depthEq
    rw [seek_word_loop_refines childPath query inclusive reverse child (USize.ofNat nextDepth) (pending ++ pushes)
      nextAligned childValid childOrdered childPrefix fits]
    simp only [result, List.reverse_append, List.reverse_reverse, List.append_assoc]
termination_by treeWork tree
decreasing_by
  assumption

def seekWordVector (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool) : Option (List CursorTask) :=
  match root, bound with
  | none, _ => some []
  | some tree, none => some [.node tree]
  | some tree, some (query, inclusive) => seekWordLoop query inclusive reverse tree 0 []

def SeekBoundFits (bound : Option (Key × Bool)) : Prop :=
  ∀ query inclusive, bound = some (query, inclusive) → query.length < USize.size

theorem seek_word_vector_refines (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (good : Good root) (fits : SeekBoundFits bound) :
    seekWordVector root bound reverse = some (seekRoot root bound reverse).reverse := by
  cases root with
  | none => cases bound <;> rfl
  | some tree =>
      cases bound with
      | none => rfl
      | some pair =>
          obtain ⟨query, inclusive⟩ := pair
          exact seek_word_loop_refines [] query inclusive reverse tree 0 [] rfl good.1 (good.2 tree rfl).2
            ⟨query, rfl⟩ (fits query inclusive rfl)

theorem seek_word_vector_no_failure (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (good : Good root) (fits : SeekBoundFits bound) : seekWordVector root bound reverse ≠ none := by
  rw [seek_word_vector_refines root bound reverse good fits]
  simp

theorem seek_word_vector_rows (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending : List CursorTask) (good : Good root) (fits : SeekBoundFits bound)
    (completed : seekWordVector root bound reverse = some pending) :
    pendingRows reverse pending.reverse = boundRows root bound reverse := by
  rw [seek_word_vector_refines root bound reverse good fits] at completed
  have same := Option.some.inj completed
  rw [← same, List.reverse_reverse]
  exact seek_root_rows root bound reverse good

theorem seek_word_vector_next_yield (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending rest : List CursorTask) (entry : Entry) (good : Good root) (fits : SeekBoundFits bound)
    (completed : seekWordVector root bound reverse = some pending)
    (yielded : nextVector reverse pending = some (entry, rest)) :
    boundRows root bound reverse = entry :: pendingRows reverse rest.reverse := by
  rw [← seek_word_vector_rows root bound reverse pending good fits completed]
  exact next_vector_yield reverse pending rest entry yielded

theorem seek_word_vector_next_empty (root : Option Tree) (bound : Option (Key × Bool)) (reverse : Bool)
    (pending : List CursorTask) (good : Good root) (fits : SeekBoundFits bound)
    (completed : seekWordVector root bound reverse = some pending) (empty : nextVector reverse pending = none) :
    boundRows root bound reverse = [] := by
  rw [← seek_word_vector_rows root bound reverse pending good fits completed]
  exact next_vector_empty reverse pending empty

end Kv9.Radix
