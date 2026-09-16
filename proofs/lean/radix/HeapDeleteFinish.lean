import HeapDeleteDescent

set_option autoImplicit false

namespace Kv9.Radix

def dropOwnedTemporary (heap : NodeHeap) (root : Option NodeId) (temporary : NodeId) (others : List NodeId) : Option NodeHeap :=
  dropLoop heap (root.toList ++ others) [temporary]

theorem drop_owned_temporary_refines (heap : NodeHeap) (root : Option NodeId) (temporary : NodeId) (others : List NodeId)
    (tree : Option Tree) (owned : HeapOwned heap (root.toList ++ temporary :: others)) (rep : RootRep heap root tree) :
    ∃ next, dropOwnedTemporary heap root temporary others = some next ∧ OwnedResult heap others tree next root := by
  have moved := List.Perm.append_left root.toList (List.perm_append_singleton temporary others).symm
  have tokens : HeapOwned heap ((root.toList ++ others) ++ [temporary]) :=
    heap_owned_permute heap _ _ (by simpa only [List.append_assoc] using moved) owned
  obtain ⟨next, completed, nextOwned, keep⟩ := drop_loop_owned heap (root.toList ++ others) [temporary] tokens
  refine ⟨next, completed, ⟨nextOwned, ?_, ?_⟩⟩
  · cases root with
    | none => cases tree <;> exact rep
    | some address =>
        cases tree with
        | none => cases rep
        | some value => exact keep address value (by simp) rep
  · intro saved value member original
    exact keep saved value (List.mem_append_right _ member) original

-- On the leaf arm, current is not moved out of its local variable. Rust keeps
-- that Arc through frame unwind and releases it at function exit. The caller
-- installs the returned root and updates size before this final scope drop;
-- those field/word operations are separate from the node-heap transitions.
def finishOwnedLeafDelete (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (frames : List OwnedDeleteFrame) :
    Option (NodeHeap × Option NodeId) := do
  let (unwound, root) ← unwindOwnedDelete heap (focus :: others) frames none
  let next ← dropOwnedTemporary unwound root focus others
  some (next, root)

theorem finish_owned_leaf_delete_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (frames : List OwnedDeleteFrame) (values : List DeleteFrame)
    (owned : HeapOwned heap (focus :: (deleteFrameRoots frames ++ others)))
    (framesRep : OwnedDeleteFramesRep heap frames values) (safe : DeleteFramesSafe values) :
    ∃ next root, finishOwnedLeafDelete heap focus others frames = some (next, root) ∧
      OwnedResult heap others (unwindDelete values none) next root := by
  have permutation : (focus :: (deleteFrameRoots frames ++ others)).Perm (deleteFrameRoots frames ++ focus :: others) := by
    simpa only [List.cons_append, List.append_assoc, List.nil_append] using
      (List.Perm.append_right others (List.perm_append_singleton focus (deleteFrameRoots frames)).symm)
  have unwindOwned : HeapOwned heap (none.toList ++ deleteFrameRoots frames ++ focus :: others) :=
    heap_owned_permute heap _ _ permutation owned
  obtain ⟨unwound, root, unwindResult, unwindCorrect⟩ := unwind_owned_delete_refines heap (focus :: others) frames values none none
    unwindOwned framesRep trivial safe
  obtain ⟨next, completed, result⟩ := drop_owned_temporary_refines unwound root focus others (unwindDelete values none)
    unwindCorrect.owned unwindCorrect.represented
  exact ⟨next, root, by simp [finishOwnedLeafDelete, unwindResult, completed],
    ⟨result.owned, result.represented,
      fun saved value member original => result.saved saved value member (unwindCorrect.saved saved value (by simp [member]) original)⟩⟩

def finishOwnedTerminalDelete (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (frames : List OwnedDeleteFrame) :
    Option (NodeHeap × Option NodeId) := do
  let held := deleteFrameRoots frames ++ others
  let (cleared, parent) ← clearOwnedTerminal heap focus held
  let (normalized, root) ← normalizeOwned cleared parent held
  unwindOwnedDelete normalized others frames root

theorem finish_owned_terminal_delete_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (frames : List OwnedDeleteFrame) (values : List DeleteFrame) (pfx : Key) (terminal : Option Entry) (children : Forest)
    (owned : HeapOwned heap (focus :: (deleteFrameRoots frames ++ others)))
    (framesRep : OwnedDeleteFramesRep heap frames values) (rep : NodeRep heap focus (.branch pfx terminal children))
    (safe : DeleteFramesSafe values) :
    ∃ next root, finishOwnedTerminalDelete heap focus others frames = some (next, root) ∧
      OwnedResult heap others (unwindDelete values (normalize (.branch pfx none children))) next root := by
  let held := deleteFrameRoots frames ++ others
  obtain ⟨cleared, parent, clearResult, clearedOwned, clearedRep, keepClear⟩ :=
    clear_owned_terminal_refines heap focus held pfx terminal children owned rep
  obtain ⟨normalized, root, normalizeResult, normalizedCorrect⟩ := normalize_owned_refines cleared parent held
    (.branch pfx none children) clearedOwned clearedRep
  have keep : ∀ saved value, saved ∈ held → NodeRep heap saved value → NodeRep normalized saved value :=
    fun saved value member original => normalizedCorrect.saved saved value member (keepClear saved value member original)
  have remaining := owned_delete_frames_preserved heap normalized frames values framesRep
    (fun saved value member original => keep saved value (List.mem_append_left _ member) original)
  have tokens : HeapOwned normalized (root.toList ++ deleteFrameRoots frames ++ others) := by
    simpa only [held, List.append_assoc] using normalizedCorrect.owned
  obtain ⟨next, address, completed, result⟩ := unwind_owned_delete_refines normalized others frames values root
    (normalize (.branch pfx none children)) tokens remaining normalizedCorrect.represented safe
  simp only [held] at clearResult normalizeResult
  exact ⟨next, address, by simp [finishOwnedTerminalDelete, clearResult, normalizeResult, completed],
    ⟨result.owned, result.represented,
      fun saved value member original => result.saved saved value member (keep saved value (List.mem_append_right _ member) original)⟩⟩

end Kv9.Radix
