import HeapPlace

set_option autoImplicit false

namespace Kv9.Radix

theorem branch_read_rep (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (modelled : HeapModelled heap)
    (read : heapRead heap parent = some (.branch pfx terminal edges)) :
    ∃ children, NodeRep heap parent (.branch pfx terminal children) := by
  obtain ⟨tree, rep⟩ := modelled parent (.branch pfx terminal edges) read
  cases rep with
  | leaf _ entry original => simp only [read, Option.some.injEq] at original; contradiction
  | branch _ oldPfx oldTerminal stored children original descendants =>
      have same : pfx = oldPfx ∧ terminal = oldTerminal ∧ edges = stored := by
        simpa only [read, Option.some.injEq, StoredNode.branch.injEq] using original
      rcases same with ⟨rfl, rfl, rfl⟩
      exact ⟨children, .branch parent _ _ _ children original descendants⟩

theorem leaf_reach_self (heap : NodeHeap) (focus : NodeId) (entry : Entry)
    (read : heapRead heap focus = some (.leaf entry)) (address : NodeId) (reachable : HeapReach heap focus address) :
    address = focus := by
  induction reachable with
  | root => rfl
  | child parent address node byte _ parentRead edge ih =>
      rw [ih, read] at parentRead
      have same : node = .leaf entry := (Option.some.inj parentRead).symm
      simp only [same, storedChildren, List.not_mem_nil] at edge

theorem heap_reach_alloc_forward (heap : NodeHeap) (root address : NodeId) (node : StoredNode)
    (reachable : HeapReach heap root address) : HeapReach (heapAlloc heap node) root address := by
  induction reachable with
  | root => exact .root
  | child parent address stored byte _ read edge ih =>
      exact .child parent address stored byte ih (heap_alloc_preserves_read heap parent stored node read) edge

theorem writable_place_alloc_leaf (state : MutHeap) (others : List NodeId) (entry : Entry)
    (owned : HeapOwned state.heap (state.root :: others)) (writable : WritablePlace state others) :
    WritablePlace { state with heap := heapAlloc state.heap (.leaf entry) } others := by
  cases state with
  | mk heap root place =>
      cases place with
      | root => trivial
      | edge parent index =>
          obtain ⟨one, separate, reachable, pfx, terminal, edges, byte, child, read, access⟩ := writable
          refine ⟨?_, ?_, heap_reach_alloc_forward heap root parent (.leaf entry) reachable,
            pfx, terminal, edges, byte, child, heap_alloc_preserves_read heap parent _ (.leaf entry) read, access⟩
          · simpa only [strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using one
          · intro saved member reaches
            obtain ⟨tree, rep⟩ := owned.rootsRepresented saved (by simp [member])
            exact separate saved member (heap_reach_alloc_reflects heap saved tree (.leaf entry) rep parent reaches)

theorem assign_place_leaf_refines (state : MutHeap) (others : List NodeId) (fresh : NodeId) (entry : Entry)
    (owned : HeapOwned state.heap (fresh :: state.root :: others)) (writable : WritablePlace state others)
    (rep : NodeRep state.heap fresh (.leaf entry)) :
    ∃ next, assignPlace state others fresh = some next ∧ HeapOwned next.heap (next.root :: others) ∧
      placeTarget next = some fresh ∧ NodeRep next.heap fresh (.leaf entry) ∧
      (∀ saved value, saved ∈ others → NodeRep state.heap saved value → NodeRep next.heap saved value) := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          obtain ⟨next, result, nextOwned, focused, preserve⟩ := assign_root_refines heap root fresh others (.leaf entry) owned rep
          exact ⟨⟨next, fresh, .root⟩, by simp [assignPlace, result], nextOwned, rfl, focused, preserve⟩
      | edge parent index =>
          obtain ⟨_, separate, reachable, pfx, terminal, edges, byte, old, read, access⟩ := writable
          obtain ⟨children, parentRep⟩ := branch_read_rep heap parent pfx terminal edges owned.modelled read
          have leafRead : heapRead heap fresh = some (.leaf entry) := by cases rep; assumption
          have different : parent ≠ fresh := by
            intro same
            rw [same, leafRead] at read
            simp at read
          have noReturn : ¬ HeapReach heap fresh parent := fun path => different (leaf_reach_self heap fresh entry leafRead parent path)
          obtain ⟨next, result, nextOwned, _, parentRead, focused, preserve⟩ :=
            assign_edge_refines heap (root :: others) parent old fresh root pfx terminal edges children index byte (.leaf entry)
              owned parentRep read access rep noReturn (by simp) reachable
          refine ⟨⟨next, root, .edge parent index⟩, ?_, nextOwned, ?_, focused, ?_⟩
          · simp [assignPlace, result]
          · obtain ⟨bound, _⟩ := List.getElem?_eq_some_iff.mp access
            simp only [placeTarget, arcSlotRead, parentRead, Option.bind_some, storedChildren,
              List.getElem?_set_self bound, Option.map_some]
          · intro saved value member original
            exact preserve saved value (by simp [member]) (separate saved member) original

theorem leaf_write_keeps_place_target (state : MutHeap) (others : List NodeId) (focus : NodeId) (old fresh : Entry)
    (writable : WritablePlace state others) (target : placeTarget state = some focus)
    (read : heapRead state.heap focus = some (.leaf old)) :
    placeTarget { state with heap := heapWrite state.heap focus (some (.leaf fresh)) } = some focus := by
  cases state with
  | mk heap root place =>
      cases place with
      | root => exact target
      | edge parent index =>
          obtain ⟨_, _, _, pfx, terminal, edges, byte, child, parentRead, access⟩ := writable
          have different : parent ≠ focus := by
            intro same
            rw [same, read] at parentRead
            simp at parentRead
          simpa only [placeTarget, arcSlotRead, heap_write_elsewhere heap focus parent (some (.leaf fresh)) different] using target

-- Matches ReplaceLeaf's get_mut fast path. The caller's selected action
-- supplies old.key = fresh.key; shared replacement allocates only the new leaf.
def replaceLeafPlace (state : MutHeap) (others : List NodeId) (fresh : Entry) : Option MutHeap :=
  (placeTarget state).bind (fun focus => match heapRead state.heap focus with
    | some (.leaf _) =>
        if strongCount state.heap (state.root :: others) focus = 1 then
          some { state with heap := heapWrite state.heap focus (some (.leaf fresh)) }
        else assignPlace { state with heap := heapAlloc state.heap (.leaf fresh) } others state.heap.length
    | _ => none)

theorem replace_leaf_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId) (old fresh : Entry)
    (owned : HeapOwned state.heap (state.root :: others)) (writable : WritablePlace state others)
    (target : placeTarget state = some focus) (read : heapRead state.heap focus = some (.leaf old)) :
    ∃ next address, replaceLeafPlace state others fresh = some next ∧ HeapOwned next.heap (next.root :: others) ∧
      placeTarget next = some address ∧ NodeRep next.heap address (.leaf fresh) ∧
      (∀ saved value, saved ∈ others → NodeRep state.heap saved value → NodeRep next.heap saved value) := by
  by_cases unique : strongCount state.heap (state.root :: others) focus = 1
  · have changed := write_leaf_payload_refines state.heap focus old fresh read
    have nextOwned := heap_write_same_targets_owned state.heap (state.root :: others) focus (.leaf old) (.leaf fresh)
      (.leaf fresh) owned read rfl changed
    have separate := writable_unique_separates state others focus writable target unique
    refine ⟨{ state with heap := heapWrite state.heap focus (some (.leaf fresh)) }, focus, ?_, nextOwned,
      leaf_write_keeps_place_target state others focus old fresh writable target read, changed, ?_⟩
    · simp [replaceLeafPlace, target, read, unique]
    · intro saved value member rep
      exact node_rep_write_frame state.heap saved focus value _ rep (separate saved member)
  · have allocated := heap_alloc_leaf_owned state.heap (state.root :: others) fresh owned
    have allocatedWritable := writable_place_alloc_leaf state others fresh owned writable
    have newRep : NodeRep (heapAlloc state.heap (.leaf fresh)) state.heap.length (.leaf fresh) :=
      .leaf state.heap.length fresh (heap_alloc_fresh state.heap (.leaf fresh))
    obtain ⟨next, result, nextOwned, newTarget, focused, preserve⟩ :=
      assign_place_leaf_refines { state with heap := heapAlloc state.heap (.leaf fresh) } others state.heap.length fresh
        allocated allocatedWritable newRep
    refine ⟨next, state.heap.length, ?_, nextOwned, newTarget, focused, ?_⟩
    · simpa only [replaceLeafPlace, target, Option.bind_some, read, if_neg unique] using result
    · intro saved value member rep
      exact preserve saved value member (node_rep_alloc state.heap saved value (.leaf fresh) rep)

end Kv9.Radix
