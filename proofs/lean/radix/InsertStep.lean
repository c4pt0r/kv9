import InsertSplits

set_option autoImplicit false

namespace Kv9.Radix

inductive InsertAction where
  | replaceLeaf
  | splitLeaf (sharedLength : Nat)
  | splitBranch (sharedLength : Nat)
  | terminal
  | addEdge (index : Nat)
  | descend (index nextDepth : Nat)

-- Selection reads the focused node without mutating it. The optional result
-- records a failing suffix-slice bound; valid path invariants exclude it.
def selectInsert (depth : Nat) (fresh : Entry) : Tree → Option InsertAction
  | .leaf old =>
      if old.key = fresh.key then some .replaceLeaf
      else if depth ≤ old.key.length ∧ depth ≤ fresh.key.length then
        some (.splitLeaf (common (old.key.drop depth) (fresh.key.drop depth)).length)
      else none
  | .branch pfx _ children =>
      if depth ≤ fresh.key.length then
        let sharedLength := (common pfx (fresh.key.drop depth)).length
        if sharedLength < pfx.length then some (.splitBranch sharedLength)
        else match fresh.key[depth + sharedLength]? with
          | none => some .terminal
          | some byte => match edgeSearch byte children with
            | .hit index => some (.descend index (depth + sharedLength + 1))
            | .miss index => some (.addEdge index)
      else none

-- This is a ghost description of a nested mutable slot, not an extra parent
-- Vec allocated by Rust insertion. Heap/Arc ownership is a later obligation.
structure InsertFrame where
  pfx : Key
  terminal : Option Entry
  children : Forest
  index : Nat

def plugInsert (frame : InsertFrame) (child : Tree) : Tree :=
  .branch frame.pfx frame.terminal (edgeSet frame.index child frame.children)

inductive InsertStep where
  | failed
  | done (replacement : Tree) (inserted : Bool)
  | down (frame : InsertFrame) (child : Tree) (nextDepth : Nat)

def executeInsert (depth : Nat) (fresh : Entry) (tree : Tree) : InsertAction → InsertStep
  | .replaceLeaf => match tree with
    | .leaf _ => .done (.leaf fresh) false
    | _ => .failed
  | .splitLeaf sharedLength => match tree with
    | .leaf old => match splitLeafAt depth sharedLength old fresh with
      | none => .failed
      | some result => .done result true
    | _ => .failed
  | .splitBranch sharedLength => match tree with
    | .branch pfx terminal children => match splitBranchAt depth sharedLength pfx terminal children fresh with
      | none => .failed
      | some result => .done result true
    | _ => .failed
  | .terminal => match tree with
    | .branch pfx terminal children => .done (.branch pfx (some fresh) children) terminal.isNone
    | _ => .failed
  | .addEdge index => match tree with
    | .branch pfx terminal children => match fresh.key[depth + pfx.length]? with
      | none => .failed
      | some byte =>
          if index ≤ edgeCount children then .done (.branch pfx terminal (edgeInsert index byte (.leaf fresh) children)) true
          else .failed
    | _ => .failed
  | .descend index nextDepth => match tree with
    | .branch pfx terminal children => match edgeGet index children with
      | none => .failed
      | some (_, child) => .down ⟨pfx, terminal, children, index⟩ child nextDepth
    | _ => .failed

def stepInsert (depth : Nat) (fresh : Entry) (tree : Tree) : InsertStep :=
  match selectInsert depth fresh tree with
  | none => .failed
  | some action => executeInsert depth fresh tree action

def InsertStepCorrect (path : Key) (fresh : Entry) (tree : Tree) : InsertStep → Prop
  | .failed => False
  | .done result inserted => (result, inserted) = insertTree path fresh tree
  | .down frame child nextDepth =>
      ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath fresh.key ∧
        nextDepth = childPath.length ∧ frame.index < edgeCount frame.children ∧
        tree = .branch frame.pfx frame.terminal frame.children ∧
        (∃ byte, edgeGet frame.index frame.children = some (byte, child)) ∧
        insertTree path fresh tree =
          (plugInsert frame (insertTree childPath fresh child).1, (insertTree childPath fresh child).2)

theorem execute_insert_down_smaller (depth : Nat) (fresh : Entry) (tree : Tree) (action : InsertAction)
    (frame : InsertFrame) (child : Tree) (nextDepth : Nat)
    (down : executeInsert depth fresh tree action = .down frame child nextDepth) : treeWork child < treeWork tree := by
  cases action with
  | replaceLeaf => cases tree <;> simp [executeInsert] at down
  | terminal => cases tree <;> simp [executeInsert] at down
  | splitLeaf sharedLength =>
      cases tree with
      | leaf old => cases result : splitLeafAt depth sharedLength old fresh <;> simp [executeInsert, result] at down
      | branch _ _ _ => simp [executeInsert] at down
  | splitBranch sharedLength =>
      cases tree with
      | leaf _ => simp [executeInsert] at down
      | branch pfx terminal children =>
          cases result : splitBranchAt depth sharedLength pfx terminal children fresh <;> simp [executeInsert, result] at down
  | addEdge index =>
      cases tree with
      | leaf _ => simp [executeInsert] at down
      | branch pfx terminal children =>
          cases access : fresh.key[depth + pfx.length]? with
          | none => simp [executeInsert, access] at down
          | some byte => by_cases bound : index ≤ edgeCount children <;> simp [executeInsert, access, bound] at down
  | descend index next =>
      cases tree with
      | leaf _ => simp [executeInsert] at down
      | branch pfx terminal children =>
          cases access : edgeGet index children with
          | none => simp [executeInsert, access] at down
          | some pair =>
              obtain ⟨byte, node⟩ := pair
              simp only [executeInsert, access, InsertStep.down.injEq] at down
              have smaller := edge_get_work children index byte node access
              rw [down.2.1] at smaller
              simp only [treeWork]
              omega

theorem step_insert_down_smaller (depth : Nat) (fresh : Entry) (tree : Tree)
    (frame : InsertFrame) (child : Tree) (nextDepth : Nat)
    (down : stepInsert depth fresh tree = .down frame child nextDepth) : treeWork child < treeWork tree := by
  unfold stepInsert at down
  cases selected : selectInsert depth fresh tree with
  | none => simp [selected] at down
  | some action =>
      exact execute_insert_down_smaller depth fresh tree action frame child nextDepth (by simpa [selected] using down)

theorem step_insert_refines (path : Key) (fresh : Entry) (tree : Tree)
    (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
    InsertStepCorrect path fresh tree (stepInsert path.length fresh tree) := by
  have depthBound := prefix_length_bound path fresh.key freshValid
  cases tree with
  | leaf old =>
      by_cases same : old.key = fresh.key
      · simp [stepInsert, selectInsert, same, executeInsert, InsertStepCorrect, insertTree]
      · have oldBound := prefix_length_bound path old.key valid
        have split := split_leaf_at_refines path old fresh valid freshValid same
        simp [stepInsert, selectInsert, same, oldBound, depthBound, executeInsert, split, InsertStepCorrect, insertTree]
  | branch pfx terminal children =>
      by_cases proper : (common pfx (fresh.key.drop path.length)).length < pfx.length
      · have split := split_branch_at_refines path pfx terminal children fresh proper
        simp [stepInsert, selectInsert, depthBound, proper, executeInsert, split, InsertStepCorrect, insertTree]
      · have full := (matching_prefix_bounds fresh.key pfx path.length depthBound proper).1
        have key := matching_key_decomposition path pfx fresh.key freshValid proper
        cases restEq : (fresh.key.drop path.length).drop pfx.length with
        | nil =>
            have access : fresh.key[path.length + pfx.length]? = none := by
              rw [absolute_cut_access, restEq]
              rfl
            simp [stepInsert, selectInsert, depthBound, full, access, executeInsert, InsertStepCorrect, insertTree, restEq]
        | cons byte rest =>
            have access : fresh.key[path.length + pfx.length]? = some byte := by
              rw [absolute_cut_access, restEq]
              rfl
            have childPrefix : HasPrefix ((path ++ pfx) ++ [byte]) fresh.key := by
              refine ⟨rest, ?_⟩
              simpa only [restEq, List.append_assoc, List.singleton_append] using key
            cases search : edgeSearch byte children with
            | miss index =>
                have safe := edge_search_miss_bound byte children index search
                have result := edge_insert_miss (path ++ pfx) fresh children byte index search
                simp [stepInsert, selectInsert, depthBound, full, access, search, executeInsert,
                  safe, InsertStepCorrect, insertTree, restEq, result]
            | hit index =>
                obtain ⟨child, found⟩ := edge_search_hit_get byte children index search
                have childValid := edge_get_valid (path ++ pfx) children index byte child valid.2 found
                have childOrdered := edge_get_ordered children index byte child ordered found
                have safe := edge_get_bound index children byte child found
                have result := edge_insert_hit (path ++ pfx) fresh children index byte child ordered found
                simp only [stepInsert, selectInsert, if_pos depthBound, full, Nat.lt_irrefl, if_false,
                  access, search, executeInsert, found, InsertStepCorrect]
                refine ⟨(path ++ pfx) ++ [byte], childValid, childOrdered, childPrefix, ?_, safe, trivial, ⟨byte, rfl⟩, ?_⟩
                · simp [Nat.add_assoc]
                · simp only [insertTree, if_neg proper, restEq, result, plugInsert]

end Kv9.Radix
