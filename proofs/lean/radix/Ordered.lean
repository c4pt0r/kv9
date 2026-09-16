import SplitBranch
import Delete

set_option autoImplicit false

namespace Kv9.Radix

def Above (lower : UInt8) : Forest → Prop
  | .nil => True
  | .cons byte _ tail => lower.toNat < byte.toNat ∧ Above lower tail

mutual
  def OrderedTree : Tree → Prop
    | .leaf _ => True
    | .branch _ _ children => OrderedForest children
  def OrderedForest : Forest → Prop
    | .nil => True
    | .cons byte child tail => OrderedTree child ∧ OrderedForest tail ∧ Above byte tail
end

theorem above_missing (lower : UInt8) (forest : Forest) (above : Above lower forest) :
    MissingEdge lower forest := by
  cases forest with
  | nil => trivial
  | cons byte child tail =>
      refine ⟨?_, above_missing lower tail above.2⟩
      intro same
      have strict := above.1
      simp [same] at strict

theorem above_trans (a b : UInt8) (forest : Forest) (less : a.toNat < b.toNat) (above : Above b forest) :
    Above a forest := by
  cases forest with
  | nil => trivial
  | cons byte child tail =>
      exact ⟨Nat.lt_trans less above.1, above_trans a b tail less above.2⟩

theorem two_edges_ordered (a : UInt8) (left : Tree) (b : UInt8) (right : Tree)
    (leftOrdered : OrderedTree left) (rightOrdered : OrderedTree right) (different : a ≠ b) :
    OrderedForest (twoEdges a left b right) := by
  by_cases less : a < b
  · simp [twoEdges, less, OrderedForest, Above, leftOrdered, rightOrdered, UInt8.lt_iff_toNat_lt.mp less]
  · have reverse : b.toNat < a.toNat := by
      have differentNat : a.toNat ≠ b.toNat := fun same => different (UInt8.toNat_inj.mp same)
      have notLess : ¬ a.toNat < b.toNat := less
      omega
    simp [twoEdges, less, OrderedForest, Above, leftOrdered, rightOrdered, reverse]

theorem leaf_split_cases_ordered (pfx : Key) (old fresh : Entry) (left right : Key)
    (separated : Separated left right) : OrderedTree (leafSplitCases pfx old fresh left right) := by
  cases left with
  | nil => cases right <;> trivial
  | cons a left =>
      cases right with
      | nil => trivial
      | cons b right => exact two_edges_ordered a (.leaf old) b (.leaf fresh) trivial trivial separated

theorem split_leaf_ordered (path : Key) (old fresh : Entry) : OrderedTree (splitLeaf path old fresh) :=
  leaf_split_cases_ordered _ _ _ _ _ (common_separates _ _)

theorem branch_split_cases_ordered (shared : Key) (oldByte : UInt8) (oldRest : Key)
    (terminal : Option Entry) (children : Forest) (fresh : Entry) (newRest : Key)
    (childOrdered : OrderedTree (.branch oldRest terminal children))
    (separated : Separated (oldByte :: oldRest) newRest) :
    OrderedTree (branchSplitCases shared oldByte oldRest terminal children fresh newRest) := by
  cases newRest with
  | nil => exact ⟨childOrdered, trivial, trivial⟩
  | cons byte rest => exact two_edges_ordered _ _ _ _ childOrdered trivial separated

theorem split_branch_ordered (path pfx : Key) (terminal : Option Entry) (children : Forest) (fresh : Entry)
    (ordered : OrderedTree (.branch pfx terminal children)) :
    OrderedTree (splitBranch path pfx terminal children fresh) := by
  dsimp only [splitBranch]
  generalize oldTailEq : pfx.drop (common pfx (fresh.key.drop path.length)).length = oldTail
  cases oldTail with
  | nil => exact ordered
  | cons byte rest =>
      exact branch_split_cases_ordered _ _ _ _ _ _ _ ordered (by
        simpa [oldTailEq] using common_separates pfx (fresh.key.drop path.length))

theorem normalize_ordered (tree : Tree) (ordered : OrderedTree tree) :
    ∀ result, normalize tree = some result → OrderedTree result := by
  cases tree with
  | leaf entry =>
      intro result eq
      simp only [normalize, Option.some.injEq] at eq
      subst result
      trivial
  | branch pfx terminal children =>
      cases children with
      | nil =>
          cases terminal with
          | none => simp [normalize]
          | some entry =>
              intro result eq
              simp only [normalize, Option.some.injEq] at eq
              subst result
              trivial
      | cons byte child tail =>
          cases terminal with
          | some entry =>
              intro result eq
              simp only [normalize, Option.some.injEq] at eq
              subst result
              exact ordered
          | none =>
              cases tail with
              | cons nextByte nextChild nextTail =>
                  intro result eq
                  simp only [normalize, Option.some.injEq] at eq
                  subst result
                  exact ordered
              | nil =>
                  cases child with
                  | leaf entry =>
                      intro result eq
                      simp only [normalize, Option.some.injEq] at eq
                      subst result
                      trivial
                  | branch cp ct cf =>
                      intro result eq
                      simp only [normalize, Option.some.injEq] at eq
                      subst result
                      exact ordered.1

theorem delete_forest_above (query : Key) (forest : Forest) (target lower : UInt8) (suffix : Key)
    (above : Above lower forest) : Above lower (deleteForest query forest target suffix) := by
  cases forest with
  | nil => trivial
  | cons byte child tail =>
      by_cases same : target = byte
      · rw [deleteForest, if_pos same]
        cases deleteTree query child suffix with
        | none => exact above.2
        | some result => exact above
      · rw [deleteForest, if_neg same]
        exact ⟨above.1, delete_forest_above query tail target lower suffix above.2⟩

mutual
  theorem delete_tree_ordered (tree : Tree) (query remaining : Key) (ordered : OrderedTree tree) :
      ∀ result, deleteTree query tree remaining = some result → OrderedTree result := by
    cases tree with
    | leaf entry => simp [deleteTree]
    | branch pfx terminal children =>
        cases h : strip pfx remaining with
        | none =>
            intro result eq
            simp only [deleteTree, h, Option.some.injEq] at eq
            subst result
            exact ordered
        | some suffix =>
            cases suffix with
            | nil =>
                rw [deleteTree, h]
                exact normalize_ordered _ ordered
            | cons byte rest =>
                rw [deleteTree, h]
                exact normalize_ordered _ (delete_forest_ordered children query byte rest ordered)
  theorem delete_forest_ordered (forest : Forest) (query : Key) (target : UInt8) (suffix : Key)
      (ordered : OrderedForest forest) : OrderedForest (deleteForest query forest target suffix) := by
    cases forest with
    | nil => trivial
    | cons byte child tail =>
        by_cases same : target = byte
        · rw [deleteForest, if_pos same]
          cases h : deleteTree query child suffix with
          | none => exact ordered.2.1
          | some result => exact ⟨delete_tree_ordered child query suffix ordered.1 result h, ordered.2⟩
        · rw [deleteForest, if_neg same]
          exact ⟨ordered.1, delete_forest_ordered tail query target suffix ordered.2.1,
            delete_forest_above query tail target byte suffix ordered.2.2⟩
end

end Kv9.Radix
