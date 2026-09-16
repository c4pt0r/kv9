import HeapOwnedMutation

set_option autoImplicit false

namespace Kv9.Radix

structure OwnedResult (before : NodeHeap) (others : List NodeId) (replacement : Option Tree)
    (after : NodeHeap) (root : Option NodeId) : Prop where
  owned : HeapOwned after (root.toList ++ others)
  represented : RootRep after root replacement
  saved : ∀ address tree, address ∈ others → NodeRep before address tree → NodeRep after address tree

def releaseOwned (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option NodeHeap :=
  dropLoop heap others [focus]

theorem release_owned_result (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (owned : HeapOwned heap (focus :: others)) :
    ∃ next, releaseOwned heap focus others = some next ∧ OwnedResult heap others none next none := by
  have workOwned := heap_owned_permute heap (focus :: others) (others ++ [focus]) (List.perm_append_singleton focus others).symm owned
  obtain ⟨next, completed, nextOwned, preserve⟩ := drop_loop_owned heap others [focus] workOwned
  exact ⟨next, completed, ⟨nextOwned, trivial, preserve⟩⟩

def takeOwnedTerminal (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId × Entry) := do
  let (copied, address) ← makeRootUnique heap focus others
  match heapRead copied address with
  | some (.branch pfx (some entry) edges) =>
      some (heapWrite copied address (some (.branch pfx none edges)), address, entry)
  | _ => none

theorem take_owned_terminal_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (entry : Entry) (children : Forest) (owned : HeapOwned heap (focus :: others))
    (rep : NodeRep heap focus (.branch pfx (some entry) children)) :
    ∃ next address, takeOwnedTerminal heap focus others = some (next, address, entry) ∧ HeapOwned next (address :: others) ∧
      NodeRep next address (.branch pfx none children) ∧ strongCount next (address :: others) address = 1 ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨copied, address, completed, copiedOwned, copiedRep, one, _, preserve⟩ :=
    make_owned_unique_refines heap focus others (.branch pfx (some entry) children) owned rep
  have stored : ∃ edges, heapRead copied address = some (.branch pfx (some entry) edges) := by
    cases copiedRep with
    | branch _ _ _ edges _ read _ => exact ⟨edges, read⟩
  obtain ⟨edges, read⟩ := stored
  have focused := write_branch_payload_refines copied address pfx pfx (some entry) none edges children copiedRep read
  obtain ⟨nextOwned, nextOne, keep⟩ := owned_write_same_children copied address others (.branch pfx (some entry) edges)
    (.branch pfx none edges) (.branch pfx none children) copiedOwned one read rfl focused
  exact ⟨heapWrite copied address (some (.branch pfx none edges)), address, by simp [takeOwnedTerminal, completed, read],
    nextOwned, focused, nextOne, fun saved value member original => keep saved value member (preserve saved value member original)⟩

theorem empty_branch_read (heap : NodeHeap) (focus : NodeId) (pfx : Key) (terminal : Option Entry)
    (rep : NodeRep heap focus (.branch pfx terminal .nil)) : heapRead heap focus = some (.branch pfx terminal []) := by
  cases rep with
  | branch _ _ _ _ _ read descendants => cases descendants; exact read

-- The second make_mut is retained in the executable model. After taking the
-- terminal, its owning node is still count-one, so this call does not allocate.
def terminalOwnedLeaf (heap : NodeHeap) (focus : NodeId) (others : List NodeId) : Option (NodeHeap × NodeId) := do
  let (taken, address, entry) ← takeOwnedTerminal heap focus others
  let (unique, actual) ← makeRootUnique taken address others
  some (heapWrite unique actual (some (.leaf entry)), actual)

theorem terminal_owned_leaf_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (pfx : Key) (entry : Entry) (owned : HeapOwned heap (focus :: others))
    (rep : NodeRep heap focus (.branch pfx (some entry) .nil)) :
    ∃ next address, terminalOwnedLeaf heap focus others = some (next, address) ∧
      OwnedResult heap others (some (.leaf entry)) next (some address) := by
  obtain ⟨taken, address, completed, takenOwned, takenRep, one, preserve⟩ :=
    take_owned_terminal_refines heap focus others pfx entry .nil owned rep
  have read := empty_branch_read taken address pfx none takenRep
  have same := make_owned_unique_identity taken address others (.branch pfx none []) read one
  have focused : NodeRep (heapWrite taken address (some (.leaf entry))) address (.leaf entry) :=
    .leaf address entry (heap_write_here taken address _ (heap_read_bound taken address _ read))
  obtain ⟨nextOwned, _, keep⟩ := owned_write_same_children taken address others (.branch pfx none []) (.leaf entry)
    (.leaf entry) takenOwned one read rfl focused
  exact ⟨heapWrite taken address (some (.leaf entry)), address, by simp [terminalOwnedLeaf, completed, same],
    ⟨nextOwned, focused, fun saved value member original => keep saved value member (preserve saved value member original)⟩⟩

end Kv9.Radix
