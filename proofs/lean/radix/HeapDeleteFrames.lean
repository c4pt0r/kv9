import HeapDeleteRestore

set_option autoImplicit false

namespace Kv9.Radix

def deleteFrameRoots (frames : List OwnedDeleteFrame) : List NodeId := frames.map OwnedDeleteFrame.parent

-- Physical frames have Vec order (bottom first). The abstract deletion loop
-- uses top-first frames, hence pushing appends physically and prepends here.
inductive OwnedDeleteFramesRep (heap : NodeHeap) : List OwnedDeleteFrame → List DeleteFrame → Prop where
  | nil : OwnedDeleteFramesRep heap [] []
  | push (frames : List OwnedDeleteFrame) (values : List DeleteFrame) (frame : OwnedDeleteFrame) (value : DeleteFrame)
      (rest : OwnedDeleteFramesRep heap frames values) (last : OwnedDeleteFrameRep heap frame value) :
      OwnedDeleteFramesRep heap (frames ++ [frame]) (value :: values)

theorem owned_delete_frame_preserved (heap next : NodeHeap) (frame : OwnedDeleteFrame) (value : DeleteFrame)
    (rep : OwnedDeleteFrameRep heap frame value)
    (keep : ∀ tree, NodeRep heap frame.parent tree → NodeRep next frame.parent tree) :
    OwnedDeleteFrameRep next frame value := ⟨rep.index, rep.byte, keep _ rep.parent⟩

theorem owned_delete_frames_preserved (heap next : NodeHeap) (frames : List OwnedDeleteFrame) (values : List DeleteFrame)
    (rep : OwnedDeleteFramesRep heap frames values)
    (keep : ∀ address tree, address ∈ deleteFrameRoots frames → NodeRep heap address tree → NodeRep next address tree) :
    OwnedDeleteFramesRep next frames values := by
  induction rep with
  | nil => exact .nil
  | push frames values frame value rest last ih =>
      exact .push frames values frame value
        (ih (fun address tree member original => keep address tree (by simp [deleteFrameRoots, List.map_append] at member ⊢; exact Or.inl member) original))
        (owned_delete_frame_preserved heap next frame value last (fun tree original => keep frame.parent tree
          (by simp [deleteFrameRoots]) original))

theorem delete_frame_pop_owned (heap : NodeHeap) (frames : List OwnedDeleteFrame) (frame : OwnedDeleteFrame)
    (root : Option NodeId) (others : List NodeId)
    (owned : HeapOwned heap (root.toList ++ deleteFrameRoots (frames ++ [frame]) ++ others)) :
    HeapOwned heap (root.toList ++ frame.parent :: (deleteFrameRoots frames ++ others)) := by
  have moved := List.Perm.append_left root.toList
    (List.Perm.append_right others (List.perm_append_singleton frame.parent (deleteFrameRoots frames)))
  apply heap_owned_permute heap _ _ _ owned
  simpa only [deleteFrameRoots, List.map_append, List.map_cons, List.map_nil, List.append_assoc, List.cons_append] using moved

-- No proof-only tree or fuel is present in the executable pop/restore loop.
def unwindOwnedDelete (heap : NodeHeap) (others : List NodeId) (frames : List OwnedDeleteFrame) (root : Option NodeId) :
    Option (NodeHeap × Option NodeId) :=
  match _popped : frames.getLast? with
  | none => some (heap, root)
  | some frame => do
      let (next, replacement) ← restoreOwnedDelete heap frame (deleteFrameRoots frames.dropLast ++ others) root
      unwindOwnedDelete next others frames.dropLast replacement
termination_by frames.length
decreasing_by
  obtain ⟨rest, same⟩ := List.getLast?_eq_some_iff.mp _popped
  simp [same]

theorem unwind_owned_delete_refines (heap : NodeHeap) (others : List NodeId) (frames : List OwnedDeleteFrame)
    (values : List DeleteFrame) (root : Option NodeId) (replacement : Option Tree)
    (owned : HeapOwned heap (root.toList ++ deleteFrameRoots frames ++ others))
    (framesRep : OwnedDeleteFramesRep heap frames values) (rep : RootRep heap root replacement) (safe : DeleteFramesSafe values) :
    ∃ next address, unwindOwnedDelete heap others frames root = some (next, address) ∧
      OwnedResult heap others (unwindDelete values replacement) next address := by
  cases framesRep with
  | nil => exact ⟨heap, root, by rw [unwindOwnedDelete]; rfl,
      ⟨by simpa [deleteFrameRoots] using owned, rep, fun _ _ _ original => original⟩⟩
  | push frames values frame value rest last =>
      have popOwned := delete_frame_pop_owned heap frames frame root others owned
      obtain ⟨restored, replacementRoot, restoredResult, restoredCorrect⟩ := restore_owned_delete_refines heap frame value
        (deleteFrameRoots frames ++ others) root replacement popOwned last rep safe.1
      have remainingRep := owned_delete_frames_preserved heap restored frames values rest
        (fun saved tree member original => restoredCorrect.saved saved tree (List.mem_append_left others member) original)
      have remainingOwned : HeapOwned restored (replacementRoot.toList ++ deleteFrameRoots frames ++ others) := by
        simpa only [List.append_assoc] using restoredCorrect.owned
      obtain ⟨next, address, completed, result⟩ := unwind_owned_delete_refines restored others frames values replacementRoot
        (restoreDelete value replacement) remainingOwned remainingRep restoredCorrect.represented safe.2
      refine ⟨next, address, ?_, ⟨result.owned, result.represented, ?_⟩⟩
      · rw [unwindOwnedDelete]
        split
        · rename_i popped
          simp only [List.getLast?_concat] at popped
          contradiction
        · rename_i actual popped
          have same : frame = actual := by simpa only [List.getLast?_concat, Option.some.injEq] using popped
          subst actual
          simpa only [List.dropLast_concat, restoredResult, bind, Option.bind_some] using completed
      · intro saved tree member original
        exact result.saved saved tree member (restoredCorrect.saved saved tree (List.mem_append_right _ member) original)
termination_by frames.length
decreasing_by
  simp_all only [List.length_append, List.length_cons, List.length_nil]
  omega

end Kv9.Radix
