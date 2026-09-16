import HeapAllocation

set_option autoImplicit false

namespace Kv9.Radix

-- The working map root has one owned token. A place merely identifies the
-- existing root or child Arc slot borrowed by insert_mut; it contributes none.
inductive MutPlace where
  | root
  | edge (parent : NodeId) (index : Nat)
deriving DecidableEq, Repr

structure MutHeap where
  heap : NodeHeap
  root : NodeId
  place : MutPlace

def placeTarget (state : MutHeap) : Option NodeId :=
  match state.place with
  | .root => some state.root
  | .edge parent index => arcSlotRead state.heap [state.root] (.edge parent index)

def makePlaceUnique (state : MutHeap) (others : List NodeId) : Option (MutHeap × NodeId) :=
  match state.place with
  | .root => (makeRootUnique state.heap state.root others).map (fun copied =>
      (⟨copied.1, copied.2, .root⟩, copied.2))
  | .edge parent index => (makeChildUnique state.heap (state.root :: others) parent index).map (fun copied =>
      (⟨copied.1, state.root, .edge parent index⟩, copied.2))

def assignPlace (state : MutHeap) (others : List NodeId) (fresh : NodeId) : Option MutHeap :=
  match state.place with
  | .root => (assignRoot state.heap state.root fresh others).map (fun next => ⟨next, fresh, .root⟩)
  | .edge parent index => (assignEdge state.heap (state.root :: others) parent index fresh).map
      (fun next => ⟨next, state.root, .edge parent index⟩)

-- A parent Arc being count-one alone is insufficient if a shared ancestor can
-- reach it. The source obtains the nested borrow only after COW on its path.
def WritablePlace (state : MutHeap) (others : List NodeId) : Prop :=
  match state.place with
  | .root => True
  | .edge parent index => strongCount state.heap (state.root :: others) parent = 1 ∧
      HeapSeparated state.heap others parent ∧ HeapReach state.heap state.root parent ∧
      ∃ pfx terminal edges byte child, heapRead state.heap parent = some (.branch pfx terminal edges) ∧
        edges[index]? = some (byte, child)

theorem place_target_rep (state : MutHeap) (others : List NodeId) (owned : HeapOwned state.heap (state.root :: others))
    (writable : WritablePlace state others) : ∃ focus tree, placeTarget state = some focus ∧ NodeRep state.heap focus tree := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          obtain ⟨tree, rep⟩ := owned.rootsRepresented root (by simp)
          exact ⟨root, tree, rfl, rep⟩
      | edge parent index =>
          obtain ⟨_, _, _, pfx, terminal, edges, byte, child, read, access⟩ := writable
          obtain ⟨parentTree, parentRep⟩ := owned.modelled parent (.branch pfx terminal edges) read
          obtain ⟨childTree, childRep, _⟩ := node_rep_child_work heap parent parentTree (.branch pfx terminal edges)
            byte child parentRep read (List.mem_of_getElem? access)
          refine ⟨child, childTree, ?_, childRep⟩
          simp only [placeTarget, arcSlotRead, read, Option.bind_some, storedChildren, access, Option.map_some]

theorem writable_unique_separates (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (writable : WritablePlace state others) (target : placeTarget state = some focus)
    (unique : strongCount state.heap (state.root :: others) focus = 1) : HeapSeparated state.heap others focus := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          have same : root = focus := Option.some.inj target
          subst focus
          exact unique_root_separates heap root others unique
      | edge parent index =>
          exact unique_child_separates heap (root :: others) others parent focus index
            (fun value member => by simp [member]) writable.2.1 target unique

end Kv9.Radix
