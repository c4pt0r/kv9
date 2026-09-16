import HeapDeleteFinish

set_option autoImplicit false

namespace Kv9.Radix

-- The graph loop contains only the actual current node, detached parent Vec,
-- query depth, and owning roots. Each descent consumes a query byte; no fuel,
-- abstract Tree, or extra prefix check is inserted into the executable path.
def deleteOwnedLoop (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (query : Key) (depth : Nat) (frames : List OwnedDeleteFrame) : Option (NodeHeap × Option NodeId) :=
  match heapRead heap focus with
  | none => none
  | some (.leaf _) => finishOwnedLeafDelete heap focus others frames
  | some (.branch pfx _ _) =>
      let offset := depth + pfx.length
      if offset = query.length then finishOwnedTerminalDelete heap focus others frames
      else match _descended : descendOwnedDelete heap focus (deleteFrameRoots frames ++ others) query offset with
        | none => none
        | some (next, frame, child) => deleteOwnedLoop next child others query (offset + 1) (frames ++ [frame])
termination_by query.length - depth
decreasing_by
  have within := descend_owned_delete_bounds heap next focus child (deleteFrameRoots frames ++ others) query
    (depth + pfx.length) frame _descended
  omega

theorem delete_owned_loop_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (query : Key) (depth : Nat) (frames : List OwnedDeleteFrame) (values : List DeleteFrame) (tree : Tree)
    (owned : HeapOwned heap (focus :: (deleteFrameRoots frames ++ others)))
    (framesRep : OwnedDeleteFramesRep heap frames values) (rep : NodeRep heap focus tree)
    (ordered : OrderedTree tree) (bounded : depth ≤ query.length) (safe : DeleteFramesSafe values)
    (present : treeLookup query tree (query.drop depth) ≠ none) :
    ∃ next root, deleteOwnedLoop heap focus others query depth frames = some (next, root) ∧
      OwnedResult heap others (unwindDelete values (deleteTree query tree (query.drop depth))) next root := by
  cases tree with
  | leaf entry =>
      have read : heapRead heap focus = some (.leaf entry) := by cases rep; assumption
      obtain ⟨next, root, completed, result⟩ := finish_owned_leaf_delete_refines heap focus others frames values owned framesRep safe
      exact ⟨next, root, by rw [deleteOwnedLoop, read]; exact completed, result⟩
  | branch pfx terminal children =>
      obtain ⟨edges, read, _⟩ := represented_branch_read heap focus pfx terminal children rep
      cases matched : strip pfx (query.drop depth) with
      | none => simp [treeLookup, matched] at present
      | some suffix =>
          obtain ⟨within, remaining, atEnd⟩ := deletion_offset query pfx suffix depth bounded matched
          cases suffix with
          | nil =>
              have done := atEnd.mpr rfl
              obtain ⟨next, root, completed, result⟩ := finish_owned_terminal_delete_refines heap focus others frames values
                pfx terminal children owned framesRep rep safe
              refine ⟨next, root, ?_, ?_⟩
              · rw [deleteOwnedLoop, read]
                simpa only [if_pos done] using completed
              · simpa only [deleteTree, matched] using result
          | cons byte rest =>
              have notDone : depth + pfx.length ≠ query.length := by
                intro equal
                have impossible := atEnd.mp equal
                cases impossible
              have access : query[depth + pfx.length]? = some byte := by rw [← drop_head_access, remaining]; rfl
              have nextRemaining : query.drop (depth + pfx.length + 1) = rest := by rw [List.drop_add_one_eq_tail_drop, remaining]; rfl
              have nextBound := (byte_descent_bounds query depth pfx.length byte access).1
              have smaller := (byte_descent_bounds query depth pfx.length byte access).2.2
              have forestPresent : forestLookup query children byte rest ≠ none := by simpa only [treeLookup, matched] using present
              obtain ⟨index, childTree, search, childAccess, childPresent⟩ := forest_present_search query rest children byte ordered forestPresent
              have childOrdered := edge_get_ordered children index byte childTree ordered childAccess
              let value : DeleteFrame := ⟨pfx, terminal, edgeRemove index children, index, byte⟩
              have nextSafe : DeleteFramesSafe (value :: values) := ⟨edge_restore_index_safe index children byte childTree childAccess, safe⟩
              have nextPresent : treeLookup query childTree (query.drop (depth + pfx.length + 1)) ≠ none := by
                simpa only [nextRemaining] using childPresent
              obtain ⟨advanced, frame, child, descended, advancedOwned, frameRep, childRep, keep⟩ :=
                descend_owned_delete_refines heap focus (deleteFrameRoots frames ++ others) query pfx (depth + pfx.length) index
                  terminal children byte childTree owned rep access search childAccess
              have keptFrames := owned_delete_frames_preserved heap advanced frames values framesRep
                (fun address old member original => keep address old (List.mem_append_left _ member) original)
              have pushed : OwnedDeleteFramesRep advanced (frames ++ [frame]) (value :: values) :=
                .push frames values frame value keptFrames frameRep
              have tokens : HeapOwned advanced (child :: (deleteFrameRoots (frames ++ [frame]) ++ others)) := by
                apply heap_owned_permute advanced _ _ _ advancedOwned
                simpa only [deleteFrameRoots, List.map_append, List.map_cons, List.map_nil, List.cons_append,
                  List.nil_append, List.append_assoc] using
                  (List.Perm.cons child (List.Perm.append_right others (List.perm_append_singleton frame.parent (deleteFrameRoots frames)).symm))
              obtain ⟨next, root, completed, result⟩ := delete_owned_loop_refines advanced child others query (depth + pfx.length + 1)
                (frames ++ [frame]) (value :: values) childTree tokens pushed childRep childOrdered nextBound nextSafe nextPresent
              refine ⟨next, root, ?_, ⟨result.owned, ?_, ?_⟩⟩
              · rw [deleteOwnedLoop, read]
                simp only [if_neg notDone]
                split
                · rename_i absent
                  rw [descended] at absent
                  contradiction
                · rename_i actual actualFrame actualChild transition
                  have same := Option.some.inj (transition.symm.trans descended)
                  cases same
                  exact completed
              · have equal : unwindDelete (value :: values) (deleteTree query childTree (query.drop (depth + pfx.length + 1))) =
                    unwindDelete values (deleteTree query (.branch pfx terminal children) (query.drop depth)) := by
                  simp only [nextRemaining, unwindDelete, value, restore_delete_refines pfx terminal children index byte childTree _ childAccess]
                  rw [deleteTree, matched]
                  dsimp only
                  rw [edge_delete_hit query rest children index byte childTree ordered childAccess]
                simpa only [equal] using result.represented
              · intro saved old member original
                exact result.saved saved old member (keep saved old (List.mem_append_right _ member) original)
termination_by query.length - depth
decreasing_by assumption

end Kv9.Radix
