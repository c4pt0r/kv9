import HeapDeleteFrames

set_option autoImplicit false

namespace Kv9.Radix

def selectStoredDelete (query : Key) (offset : Nat) : StoredNode → Option Nat
  | .leaf _ => none
  | .branch _ _ edges => do
      let byte ← query[offset]?
      match storedEdgeSearch byte edges with
      | .miss _ => none
      | .hit index => some index

theorem select_stored_delete_bounds (query : Key) (offset index : Nat) (node : StoredNode)
    (selected : selectStoredDelete query offset node = some index) : offset < query.length := by
  cases node with
  | leaf entry => simp [selectStoredDelete] at selected
  | branch pfx terminal edges =>
      cases access : query[offset]? with
      | none => simp [selectStoredDelete, access] at selected
      | some byte => exact (List.getElem?_eq_some_iff.mp access).1

-- The source makes the parent unique before indexing the query and searching
-- its edges. The selected edge is moved, and the resulting parent is saved.
def descendOwnedDelete (heap : NodeHeap) (focus : NodeId) (others : List NodeId) (query : Key) (offset : Nat) :
    Option (NodeHeap × OwnedDeleteFrame × NodeId) := do
  let (copied, parent) ← makeRootUnique heap focus others
  let node ← heapRead copied parent
  let index ← selectStoredDelete query offset node
  let (detached, byte, child) ← detachEdge copied parent index
  some (detached, ⟨parent, index, byte⟩, child)

theorem descend_owned_delete_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (query pfx : Key) (offset index : Nat) (terminal : Option Entry) (children : Forest) (byte : UInt8) (tree : Tree)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus (.branch pfx terminal children))
    (access : query[offset]? = some byte) (search : edgeSearch byte children = .hit index)
    (found : edgeGet index children = some (byte, tree)) :
    ∃ next frame child, descendOwnedDelete heap focus others query offset = some (next, frame, child) ∧
      HeapOwned next (child :: frame.parent :: others) ∧
      OwnedDeleteFrameRep next frame ⟨pfx, terminal, edgeRemove index children, index, byte⟩ ∧ NodeRep next child tree ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨copied, parent, copiedResult, copiedOwned, copiedRep, _, separate, preserve⟩ :=
    make_owned_unique_refines heap focus others (.branch pfx terminal children) owned rep
  obtain ⟨edges, read, descendants⟩ := represented_branch_read copied parent pfx terminal children copiedRep
  obtain ⟨child, actual, childRep⟩ := edges_rep_get_address copied edges children index byte tree descendants found
  obtain ⟨detached, nextOwned, nextParent, nextChild, _, keep⟩ :=
    detach_edge_refines copied (parent :: others) parent child pfx terminal edges children index byte tree copiedOwned copiedRep read actual childRep
  let next := heapWrite copied parent (some (.branch pfx terminal (storedRemove edges index)))
  refine ⟨next, ⟨parent, index, byte⟩, child, ?_, nextOwned, ⟨rfl, rfl, nextParent⟩, nextChild, ?_⟩
  · simp [descendOwnedDelete, copiedResult, read, selectStoredDelete, access,
      stored_edge_search_refines copied edges children byte descendants, search, detached, next]
  · intro saved value member original
    exact keep saved value (separate saved member) (preserve saved value member original)

theorem descend_owned_delete_bounds (heap next : NodeHeap) (focus child : NodeId) (others : List NodeId)
    (query : Key) (offset : Nat) (frame : OwnedDeleteFrame)
    (completed : descendOwnedDelete heap focus others query offset = some (next, frame, child)) : offset < query.length := by
  cases unique : makeRootUnique heap focus others with
  | none => simp [descendOwnedDelete, unique] at completed
  | some copied =>
      rcases copied with ⟨copied, parent⟩
      cases read : heapRead copied parent with
      | none => simp [descendOwnedDelete, unique, read] at completed
      | some node =>
          cases selected : selectStoredDelete query offset node with
          | none => simp [descendOwnedDelete, unique, read, selected] at completed
          | some index => exact select_stored_delete_bounds query offset index node selected

def clearOwnedTerminal (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId) := do
  let (copied, address) ← makeRootUnique heap focus others
  match heapRead copied address with
  | some (.branch pfx _ edges) => some (heapWrite copied address (some (.branch pfx none edges)), address)
  | _ => none

theorem clear_owned_terminal_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest)
    (owned : HeapOwned heap (focus :: others)) (rep : NodeRep heap focus (.branch pfx terminal children)) :
    ∃ next address, clearOwnedTerminal heap focus others = some (next, address) ∧
      HeapOwned next (address :: others) ∧ NodeRep next address (.branch pfx none children) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨copied, address, completed, copiedOwned, copiedRep, one, _, preserve⟩ :=
    make_owned_unique_refines heap focus others (.branch pfx terminal children) owned rep
  obtain ⟨edges, read, _⟩ := represented_branch_read copied address pfx terminal children copiedRep
  have focused := write_branch_payload_refines copied address pfx pfx terminal none edges children copiedRep read
  obtain ⟨nextOwned, _, keep⟩ := owned_write_same_children copied address others (.branch pfx terminal edges)
    (.branch pfx none edges) (.branch pfx none children) copiedOwned one read rfl focused
  exact ⟨heapWrite copied address (some (.branch pfx none edges)), address, by simp [clearOwnedTerminal, completed, read],
    nextOwned, focused, fun saved value member original => keep saved value member (preserve saved value member original)⟩

end Kv9.Radix
