import EdgeMutation

set_option autoImplicit false

namespace Kv9.Radix

-- A popped Rust frame contains a parent with its selected edge removed.
-- The top-first list is the reverse of the Rust Vec of saved frames.
structure DeleteFrame where
  pfx : Key
  terminal : Option Entry
  detached : Forest
  index : Nat
  byte : UInt8

def restoreDelete (frame : DeleteFrame) (replacement : Option Tree) : Option Tree :=
  normalize (.branch frame.pfx frame.terminal
    (match replacement with
      | none => frame.detached
      | some child => edgeInsert frame.index frame.byte child frame.detached))

def unwindDelete : List DeleteFrame → Option Tree → Option Tree
  | [], replacement => replacement
  | frame :: tail, replacement => unwindDelete tail (restoreDelete frame replacement)

def DeleteFramesSafe : List DeleteFrame → Prop
  | [] => True
  | frame :: tail => frame.index ≤ edgeCount frame.detached ∧ DeleteFramesSafe tail

def unwindDeleteChecked : List DeleteFrame → Option Tree → Option (Option Tree)
  | [], replacement => some replacement
  | frame :: tail, replacement =>
      if replacement.isSome && decide (frame.index > edgeCount frame.detached) then none
      else unwindDeleteChecked tail (restoreDelete frame replacement)

theorem unwind_delete_checked (frames : List DeleteFrame) (replacement : Option Tree)
    (safe : DeleteFramesSafe frames) :
    unwindDeleteChecked frames replacement = some (unwindDelete frames replacement) := by
  induction frames generalizing replacement with
  | nil => rfl
  | cons frame tail ih =>
      have bounded : ¬ frame.index > edgeCount frame.detached := by have bound := safe.1; omega
      simpa only [unwindDeleteChecked, bounded, decide_false, Bool.and_false, Bool.false_eq_true,
        if_false, unwindDelete] using ih (restoreDelete frame replacement) safe.2

theorem unwind_delete_append (a b : List DeleteFrame) (replacement : Option Tree) :
    unwindDelete (a ++ b) replacement = unwindDelete b (unwindDelete a replacement) := by
  induction a generalizing replacement with
  | nil => rfl
  | cons frame tail ih => exact ih (restoreDelete frame replacement)

theorem restore_delete_refines (pfx : Key) (terminal : Option Entry) (forest : Forest)
    (index : Nat) (byte : UInt8) (child : Tree) (replacement : Option Tree)
    (found : edgeGet index forest = some (byte, child)) :
    restoreDelete ⟨pfx, terminal, edgeRemove index forest, index, byte⟩ replacement =
      normalize (.branch pfx terminal (edgeReplace index replacement forest)) := by
  cases replacement with
  | none => rfl
  | some result =>
      exact congrArg (fun edges => normalize (.branch pfx terminal edges))
        (edge_remove_restore index forest byte child result found)

-- The outer none denotes a failed index/search assertion. A successful
-- deletion may return some none when the last entry was removed.
-- No fuel truncation and no extra prefix comparison is added to Rust's loop.
def deleteLoop (query : Key) (tree : Tree) (depth : Nat) (frames : List DeleteFrame) : Option (Option Tree) :=
  match tree with
  | .leaf _ => unwindDeleteChecked frames none
  | .branch pfx terminal children =>
      let endOffset := depth + pfx.length
      if endOffset = query.length then
        unwindDeleteChecked frames (normalize (.branch pfx none children))
      else
        match query[endOffset]? with
        | none => none
        | some byte =>
            match edgeSearch byte children with
            | .miss _ => none
            | .hit index =>
                match access : edgeGet index children with
                | none => none
                | some (storedByte, child) =>
                    deleteLoop query child (endOffset + 1)
                      (⟨pfx, terminal, edgeRemove index children, index, storedByte⟩ :: frames)
termination_by treeWork tree
decreasing_by
  have smaller := edge_get_work children index storedByte child access
  simp only [treeWork]
  omega

theorem drop_head_access (query : Key) (offset : Nat) :
    (query.drop offset).head? = query[offset]? := by
  induction offset generalizing query with
  | zero => cases query <;> rfl
  | succ n ih => cases query <;> simp [ih]

theorem deletion_offset (query pfx suffix : Key) (depth : Nat)
    (bounded : depth ≤ query.length) (matched : strip pfx (query.drop depth) = some suffix) :
    depth + pfx.length ≤ query.length ∧ query.drop (depth + pfx.length) = suffix ∧
      (depth + pfx.length = query.length ↔ suffix = []) := by
  have split := (strip_some_iff _ _ _).mp matched
  have lengths := congrArg List.length split
  simp only [List.length_drop, List.length_append] at lengths
  have rest : query.drop (depth + pfx.length) = suffix := by
    rw [← List.drop_drop, split]
    simp
  refine ⟨by omega, rest, ?_⟩
  constructor
  · intro endEq
    have empty : suffix.length = 0 := by omega
    simpa using empty
  · intro empty
    simp only [empty, List.length_nil] at lengths
    omega

theorem delete_loop_refines (query : Key) (tree : Tree) (depth : Nat) (frames : List DeleteFrame)
    (ordered : OrderedTree tree) (bounded : depth ≤ query.length) (safe : DeleteFramesSafe frames)
    (present : treeLookup query tree (query.drop depth) ≠ none) :
    deleteLoop query tree depth frames = some (unwindDelete frames (deleteTree query tree (query.drop depth))) := by
  cases treeEq : tree with
  | leaf entry => rw [deleteLoop, deleteTree, unwind_delete_checked frames none safe]
  | branch pfx terminal children =>
      rw [treeEq] at ordered present
      cases matched : strip pfx (query.drop depth) with
      | none => simp [treeLookup, matched] at present
      | some suffix =>
          obtain ⟨within, remaining, atEnd⟩ := deletion_offset query pfx suffix depth bounded matched
          cases suffix with
          | nil =>
              have done := atEnd.mpr rfl
              simp only [deleteLoop, if_pos done, deleteTree, matched, unwind_delete_checked frames _ safe]
          | cons byte rest =>
              have notDone : depth + pfx.length ≠ query.length := by
                intro equal
                have impossible := atEnd.mp equal
                cases impossible
              have access : query[depth + pfx.length]? = some byte := by
                rw [← drop_head_access, remaining]
                rfl
              have nextRemaining : query.drop (depth + pfx.length + 1) = rest := by
                rw [List.drop_add_one_eq_tail_drop, remaining]
                rfl
              have nextBound : depth + pfx.length + 1 ≤ query.length :=
                (byte_descent_bounds query depth pfx.length byte access).1
              have forestPresent : forestLookup query children byte rest ≠ none := by
                simpa only [treeLookup, matched] using present
              obtain ⟨index, child, search, childAccess, childPresent⟩ :=
                forest_present_search query rest children byte ordered forestPresent
              have childOrdered := edge_get_ordered children index byte child ordered childAccess
              have nextSafe : DeleteFramesSafe (⟨pfx, terminal, edgeRemove index children, index, byte⟩ :: frames) :=
                ⟨edge_restore_index_safe index children byte child childAccess, safe⟩
              have nextPresent : treeLookup query child (query.drop (depth + pfx.length + 1)) ≠ none := by
                simpa only [nextRemaining] using childPresent
              rw [deleteLoop]
              simp only [if_neg notDone, access, search]
              split
              · rename_i absentAccess
                rw [childAccess] at absentAccess
                cases absentAccess
              · rename_i storedByte next accessEq
                have pair := Option.some.inj (accessEq.symm.trans childAccess)
                cases pair
                rw [delete_loop_refines query child (depth + pfx.length + 1)
                  (⟨pfx, terminal, edgeRemove index children, index, byte⟩ :: frames) childOrdered nextBound nextSafe nextPresent]
                simp only [nextRemaining, unwindDelete, restore_delete_refines pfx terminal children index byte child _ childAccess]
                rw [deleteTree, matched]
                dsimp only
                rw [edge_delete_hit query rest children index byte child ordered childAccess]
termination_by treeWork tree
decreasing_by
  have smaller := edge_get_work children index byte child childAccess
  simp only [treeEq, treeWork]
  omega

def eraseLoop (query : Key) (root : Option Tree) : Option (Option Tree) :=
  if optionalLookup query root = none then some root
  else match root with
    | none => none
    | some tree => deleteLoop query tree 0 []

theorem erase_loop_refines (query : Key) (root : Option Tree)
    (ordered : ∀ tree, root = some tree → OrderedTree tree) :
    eraseLoop query root = some (eraseRoot query root) := by
  cases root with
  | none => rfl
  | some tree =>
      by_cases absent : treeLookup query tree query = none
      · simp [eraseLoop, optionalLookup, eraseRoot, absent]
      · have result := delete_loop_refines query tree 0 [] (ordered tree rfl) (Nat.zero_le _) trivial
          (by simpa using absent)
        simpa [eraseLoop, optionalLookup, eraseRoot, absent, unwindDelete] using result

theorem erase_loop_no_assertion_failure (query : Key) (root : Option Tree) (good : Good root) :
    eraseLoop query root ≠ none := by
  rw [erase_loop_refines query root (fun tree eq => (good.2 tree eq).2)]
  simp

theorem erase_loop_entries (query : Key) (root : Option Tree) (good : Good root) :
    (eraseLoop query root).map optionalEntries = some (dropKey query (optionalEntries root)) := by
  rw [erase_loop_refines query root (fun tree eq => (good.2 tree eq).2)]
  simp only [Option.map_some, erase_root_entries query root good.1]

end Kv9.Radix
