import HeapInsertDescent

set_option autoImplicit false

namespace Kv9.Radix

-- A completed insertion arm must establish the whole working root's value,
-- exact live ownership inventory, and every saved snapshot's old value.
structure MutationResult (before : MutHeap) (others : List NodeId) (frames : List HeapFrame)
    (replacement : Tree) (after : MutHeap) : Prop where
  owned : HeapOwned after.heap (after.root :: others)
  rootValue : NodeRep after.heap after.root (plugInsertFrames (frames.map HeapFrame.value) replacement)
  saved : ∀ root tree, root ∈ others → NodeRep before.heap root tree → NodeRep after.heap root tree

theorem mutation_same_children_write (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree replacement : Tree) (frames : List HeapFrame) (old node : StoredNode)
    (inv : MutationInvariant state others focus tree frames)
    (one : strongCount state.heap (state.root :: others) focus = 1)
    (read : heapRead state.heap focus = some old) (targets : storedChildren node = storedChildren old)
    (focused : NodeRep (heapWrite state.heap focus (some node)) focus replacement) :
    MutationInvariant { state with heap := heapWrite state.heap focus (some node) } others focus replacement frames ∧
      strongCount (heapWrite state.heap focus (some node)) (state.root :: others) focus = 1 ∧
      (∀ root value, root ∈ others → NodeRep state.heap root value →
        NodeRep (heapWrite state.heap focus (some node)) root value) := by
  have separate := writable_unique_separates state others focus (mutation_invariant_writable state others focus tree frames inv)
    (mutation_invariant_target state others focus tree frames inv) one
  have safe := heap_context_private_safe state.heap (state.root :: others) focus state.root frames inv.context tree inv.focused one inv.privateParents
  have counts := heap_write_same_targets_count state.heap (state.root :: others) focus old node
  refine ⟨⟨heap_write_same_targets_owned state.heap (state.root :: others) focus old node replacement inv.owned read targets focused,
    heap_context_write_frame state.heap focus state.root focus frames (some node) inv.context safe, ?_, ?_, inv.aligned, focused⟩,
    (counts focus read targets).trans one, ?_⟩
  · intro frame member
    rw [counts frame.parent read targets]
    exact inv.privateParents frame member
  · intro frame member root saved reachable
    exact inv.savedParents frame member root saved
      (heap_reach_write_reflects state.heap root focus (some node) (separate root saved) frame.parent reachable)
  · intro root value member original
    exact node_rep_write_frame state.heap root focus value (some node) original (separate root member)

def terminalPlace (state : MutHeap) (others : List NodeId) (fresh : Entry) : Option (MutHeap × Bool) := do
  let (next, focus) ← makePlaceUnique state others
  match heapRead next.heap focus with
  | some (.branch pfx terminal edges) =>
      some ({ next with heap := heapWrite next.heap focus (some (.branch pfx (some fresh) edges)) }, terminal.isNone)
  | _ => none

theorem terminal_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame) (fresh : Entry)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames) :
    ∃ next, terminalPlace state others fresh = some (next, terminal.isNone) ∧
      MutationResult state others frames (.branch pfx (some fresh) children) next := by
  obtain ⟨unique, address, uniqueFrames, completed, uniqueInv, one, _, values, preserve⟩ :=
    make_place_unique_invariant state others focus (.branch pfx terminal children) frames inv
  have stored : ∃ edges, heapRead unique.heap address = some (.branch pfx terminal edges) := by
    cases uniqueInv.focused with
    | branch _ _ _ edges _ read _ => exact ⟨edges, read⟩
  obtain ⟨edges, read⟩ := stored
  have focused := write_branch_payload_refines unique.heap address pfx pfx terminal (some fresh) edges children uniqueInv.focused read
  obtain ⟨nextInv, _, keep⟩ := mutation_same_children_write unique others address (.branch pfx terminal children)
    (.branch pfx (some fresh) children) uniqueFrames (.branch pfx terminal edges) (.branch pfx (some fresh) edges)
    uniqueInv one read rfl focused
  refine ⟨{ unique with heap := heapWrite unique.heap address (some (.branch pfx (some fresh) edges)) }, ?_,
    ⟨nextInv.owned, ?_, fun root old member original => keep root old member (preserve root old member original)⟩⟩
  · simp [terminalPlace, completed, read]
  · have rootValue := heap_context_plug _ address unique.root uniqueFrames nextInv.context (.branch pfx (some fresh) children) nextInv.focused
    simpa only [values] using rootValue

theorem terminal_place_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame) (depth : Nat) (fresh : Entry)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames) :
    executeInsert depth fresh (.branch pfx terminal children) .terminal = .done (.branch pfx (some fresh) children) terminal.isNone ∧
      ∃ next, terminalPlace state others fresh = some (next, terminal.isNone) ∧
        MutationResult state others frames (.branch pfx (some fresh) children) next :=
  ⟨rfl, terminal_place_refines state others focus pfx terminal children frames fresh inv⟩

end Kv9.Radix
