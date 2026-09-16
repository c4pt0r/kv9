import HeapContextCow

set_option autoImplicit false

namespace Kv9.Radix

def PlaceAligned : MutPlace → List HeapFrame → Prop
  | .root, [] => True
  | .edge parent index, frame :: _ => frame.parent = parent ∧ frame.value.index = index
  | _, _ => False

def ContextSaved (heap : NodeHeap) (saved : List NodeId) (frames : List HeapFrame) : Prop :=
  ∀ frame ∈ frames, HeapSeparated heap saved frame.parent

-- The context is ghost state for the nested mutable borrow. It owns no Arc
-- tokens and records every ancestor's isolation from the saved snapshots.
structure MutationInvariant (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) : Prop where
  owned : HeapOwned state.heap (state.root :: others)
  context : HeapContext state.heap focus state.root frames
  privateParents : ContextPrivate state.heap (state.root :: others) frames
  savedParents : ContextSaved state.heap others frames
  aligned : PlaceAligned state.place frames
  focused : NodeRep state.heap focus tree

theorem mutation_invariant_target (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (inv : MutationInvariant state others focus tree frames) :
    placeTarget state = some focus := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          cases frames with
          | nil => cases inv.context; rfl
          | cons frame tail => exact False.elim inv.aligned
      | edge parent index =>
          cases frames with
          | nil => exact False.elim inv.aligned
          | cons frame tail =>
              obtain ⟨parentEq, indexEq⟩ := inv.aligned
              cases inv.context with
              | up _ _ _ _ read siblings _ =>
                  obtain ⟨byte, access⟩ := edge_hole_read heap focus frame.edges frame.value.index frame.value.children siblings
                  simp only [placeTarget, arcSlotRead, ← parentEq, read, Option.bind_some,
                    storedChildren, ← indexEq, access, Option.map_some]

theorem mutation_invariant_writable (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (inv : MutationInvariant state others focus tree frames) :
    WritablePlace state others := by
  cases state with
  | mk heap root place =>
      cases place with
      | root => trivial
      | edge parent index =>
          cases frames with
          | nil => exact False.elim inv.aligned
          | cons frame tail =>
              obtain ⟨parentEq, indexEq⟩ := inv.aligned
              cases inv.context with
              | up _ _ _ _ read siblings outer =>
                  obtain ⟨byte, access⟩ := edge_hole_read heap focus frame.edges frame.value.index frame.value.children siblings
                  change strongCount heap (root :: others) parent = 1 ∧ _
                  rw [← parentEq, ← indexEq]
                  exact ⟨inv.privateParents frame (by simp), inv.savedParents frame (by simp),
                    heap_context_reach heap frame.parent root tail outer,
                    frame.value.pfx, frame.value.terminal, frame.edges, byte, focus, read, access⟩

theorem mutation_invariant_initial (heap : NodeHeap) (root : NodeId) (others : List NodeId) (tree : Tree)
    (owned : HeapOwned heap (root :: others)) (focused : NodeRep heap root tree) :
    MutationInvariant ⟨heap, root, .root⟩ others root tree [] :=
  ⟨owned, .root root, by simp [ContextPrivate], by simp [ContextSaved], trivial, focused⟩

theorem allocated_write_keeps_separation (heap : NodeHeap) (saved : List NodeId) (parent target : NodeId)
    (node : StoredNode) (replacement : Option StoredNode) (represented : RootsModelled heap saved)
    (parentSeparate : HeapSeparated heap saved parent) (targetSeparate : HeapSeparated heap saved target) :
    HeapSeparated (heapWrite (heapAlloc heap node) parent replacement) saved target := by
  intro root member reachable
  obtain ⟨tree, rep⟩ := represented root member
  have separate : ¬ HeapReach (heapAlloc heap node) root parent := fun path =>
    parentSeparate root member (heap_reach_alloc_reflects heap root tree node rep parent path)
  exact targetSeparate root member (heap_reach_alloc_reflects heap root tree node rep target
    (heap_reach_write_reflects (heapAlloc heap node) root parent replacement separate target reachable))

theorem make_child_unique_keeps_separation (heap next : NodeHeap) (roots saved : List NodeId)
    (parent child address target : NodeId) (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge)
    (index : Nat) (byte : UInt8) (tree : Tree) (parentOne : strongCount heap roots parent = 1)
    (parentRead : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, child))
    (focused : NodeRep heap child tree) (represented : RootsModelled heap saved)
    (parentSeparate : HeapSeparated heap saved parent) (targetSeparate : HeapSeparated heap saved target)
    (completed : makeChildUnique heap roots parent index = some (next, address)) : HeapSeparated next saved target := by
  obtain ⟨node, read⟩ := node_rep_has_cell heap child tree focused
  by_cases one : strongCount heap roots child = 1
  · have same : (heap, child) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, read, one] using completed
    cases same
    exact targetSeparate
  · have same : (cloneChildAt heap parent pfx terminal edges index byte node, heap.length) = (next, address) := by
      simpa [makeChildUnique, parentOne, parentRead, access, read, one] using completed
    cases same
    exact allocated_write_keeps_separation heap saved parent target node _ represented parentSeparate targetSeparate

theorem make_place_unique_invariant (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (tree : Tree) (frames : List HeapFrame) (inv : MutationInvariant state others focus tree frames) :
    ∃ next address nextFrames, makePlaceUnique state others = some (next, address) ∧
      MutationInvariant next others address tree nextFrames ∧
      strongCount next.heap (next.root :: others) address = 1 ∧
      HeapSeparated next.heap others address ∧ nextFrames.map HeapFrame.value = frames.map HeapFrame.value ∧
      (∀ root old, root ∈ others → NodeRep state.heap root old → NodeRep next.heap root old) := by
  cases state with
  | mk heap root place =>
      have represented : RootsModelled heap others := fun saved member => inv.owned.rootsRepresented saved (by simp [member])
      cases place with
      | root =>
          cases frames with
          | cons frame tail => exact False.elim inv.aligned
          | nil =>
              have same : root = focus := Option.some.inj (mutation_invariant_target _ others focus tree [] inv)
              subst focus
              obtain ⟨next, address, completed, focused, separate, preserve⟩ :=
                make_root_unique_refines heap root others tree inv.focused represented
              exact ⟨⟨next, address, .root⟩, address, [], by simp [makePlaceUnique, completed],
                mutation_invariant_initial next address others tree (make_root_unique_owned heap next root address others inv.owned completed) focused,
                make_root_unique_count heap next root address others tree (heap_modelled_closed heap inv.owned.modelled)
                  inv.focused represented completed, separate, rfl, fun saved old _ previous => preserve saved old previous⟩
      | edge parent index =>
          cases frames with
          | nil => exact False.elim inv.aligned
          | cons frame tail =>
              obtain ⟨parentEq, indexEq⟩ := inv.aligned
              obtain ⟨next, address, newFrame, completed, context, focused, privateParents, newParent, newValue⟩ :=
                make_child_unique_context heap (root :: others) focus root frame tail tree inv.context inv.focused inv.privateParents
              have parentOne := inv.privateParents frame (by simp)
              have parentSeparate := inv.savedParents frame (by simp)
              have details : ∃ byte, frame.edges[frame.value.index]? = some (byte, focus) ∧
                  heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges) ∧
                  NodeRep heap frame.parent (plugInsert frame.value tree) := by
                cases inv.context with
                | up _ _ _ _ read siblings _ =>
                    obtain ⟨byte, access⟩ := edge_hole_read heap focus frame.edges frame.value.index frame.value.children siblings
                    exact ⟨byte, access, read, .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
                      (edge_hole_plug heap focus frame.edges frame.value.index frame.value.children siblings tree inv.focused)⟩
              obtain ⟨byte, access, parentRead, parentRep⟩ := details
              have owned := make_child_unique_owned heap next (root :: others) frame.parent focus address frame.value.pfx frame.value.terminal
                frame.edges (edgeSet frame.value.index tree frame.value.children) frame.value.index byte inv.owned parentOne parentRep parentRead access completed
              have one := make_child_unique_count heap next (root :: others) frame.parent focus address frame.value.pfx frame.value.terminal
                frame.edges (edgeSet frame.value.index tree frame.value.children) frame.value.index byte parentOne
                (heap_modelled_closed heap inv.owned.modelled) inv.owned.rootsRepresented parentRep parentRead access completed
              obtain ⟨actualHeap, actualAddress, _, actual, _, _, _, separate, preserve⟩ :=
                make_child_unique_refines heap (root :: others) others frame.parent focus frame.value.pfx frame.value.terminal
                  frame.edges (edgeSet frame.value.index tree frame.value.children) frame.value.index byte parentOne
                  (fun saved member => by simp [member]) represented parentRep parentRead access parentSeparate
              have same := Option.some.inj (actual.symm.trans completed)
              cases same
              have savedParents : ContextSaved next others (newFrame :: tail) := by
                intro item member
                have original : HeapSeparated heap others item.parent := by
                  rcases List.mem_cons.mp member with atHead | later
                  · subst item
                    rw [newParent]
                    exact parentSeparate
                  · exact inv.savedParents item (by simp [later])
                exact make_child_unique_keeps_separation heap next (root :: others) others frame.parent focus address item.parent
                  frame.value.pfx frame.value.terminal frame.edges frame.value.index byte tree parentOne parentRead access
                  inv.focused represented parentSeparate original completed
              refine ⟨⟨next, root, .edge parent index⟩, address, newFrame :: tail, ?_,
                ⟨owned, context, privateParents, savedParents, ?_, focused⟩, one, separate, ?_, preserve⟩
              · simp only [makePlaceUnique, ← parentEq, ← indexEq, completed, Option.map_some]
              · exact ⟨newParent.trans parentEq, by rw [newValue]; exact indexEq⟩
              · simp only [List.map_cons, newValue]

end Kv9.Radix
