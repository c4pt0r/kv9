import HeapNormalizeSingle

set_option autoImplicit false

namespace Kv9.Radix

def normalizeOwned (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × Option NodeId) :=
  match heapRead heap focus with
  | none => none
  | some (.leaf _) => some (heap, some focus)
  | some (.branch _ terminal edges) => match terminal, edges with
    | none, [] => (releaseOwned heap focus others).map (fun next => (next, none))
    | some _, [] => (terminalOwnedLeaf heap focus others).map (fun result => (result.1, some result.2))
    | none, [_] => (singletonOwnedChild heap focus others).map (fun result => (result.1, some result.2))
    | _, _ => some (heap, some focus)

theorem owned_result_identity (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus tree) :
    OwnedResult heap others (some tree) heap (some focus) :=
  ⟨owned, rep, fun _ _ _ original => original⟩

theorem normalize_owned_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus tree) :
    ∃ next root, normalizeOwned heap focus others = some (next, root) ∧ OwnedResult heap others (normalize tree) next root := by
  cases tree with
  | leaf entry =>
      have read : heapRead heap focus = some (.leaf entry) := by cases rep; assumption
      exact ⟨heap, some focus, by simp [normalizeOwned, read], owned_result_identity heap focus others (.leaf entry) owned rep⟩
  | branch pfx terminal children =>
      obtain ⟨edges, read, descendants⟩ := represented_branch_read heap focus pfx terminal children rep
      cases children with
      | nil =>
          cases descendants
          cases terminal with
          | none =>
              obtain ⟨next, completed, result⟩ := release_owned_result heap focus others owned
              exact ⟨next, none, by simp [normalizeOwned, read, completed], result⟩
          | some entry =>
              obtain ⟨next, address, completed, result⟩ := terminal_owned_leaf_refines heap focus others pfx entry owned rep
              exact ⟨next, some address, by simp [normalizeOwned, read, completed], result⟩
      | cons byte child tail =>
          cases descendants with
          | cons _ childAddress childEdges _ _ childRep tailRep =>
              cases terminal with
              | some entry =>
                  exact ⟨heap, some focus, by simp [normalizeOwned, read],
                    owned_result_identity heap focus others (.branch pfx (some entry) (.cons byte child tail)) owned rep⟩
              | none =>
                  cases tail with
                  | nil =>
                      cases tailRep
                      obtain ⟨next, address, completed, result⟩ := singleton_owned_child_refines heap focus others pfx byte child owned rep
                      exact ⟨next, some address, by simp [normalizeOwned, read, completed], result⟩
                  | cons nextByte nextChild rest =>
                      cases tailRep with
                      | cons _ nextAddress restEdges _ _ nextRep restRep =>
                          exact ⟨heap, some focus, by simp [normalizeOwned, read],
                            owned_result_identity heap focus others (.branch pfx none (.cons byte child (.cons nextByte nextChild rest))) owned rep⟩

theorem normalize_owned_completed (heap next : NodeHeap) (focus : NodeId) (root : Option NodeId)
    (others : List NodeId) (tree : Tree) (owned : HeapOwned heap (focus :: others))
    (rep : NodeRep heap focus tree) (completed : normalizeOwned heap focus others = some (next, root)) :
    OwnedResult heap others (normalize tree) next root := by
  obtain ⟨actual, address, result, correct⟩ := normalize_owned_refines heap focus others tree owned rep
  have same := Option.some.inj (result.symm.trans completed)
  cases same
  exact correct

end Kv9.Radix
