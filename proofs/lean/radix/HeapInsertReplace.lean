import HeapInsertTerminal

set_option autoImplicit false

namespace Kv9.Radix

theorem assign_edge_context_root (heap next : NodeHeap) (roots : List NodeId) (old fresh root : NodeId)
    (frame : HeapFrame) (tail : List HeapFrame) (tree replacement : Tree)
    (owned : HeapOwned heap (fresh :: roots)) (member : root ∈ roots)
    (context : HeapContext heap old root (frame :: tail)) (focused : NodeRep heap old tree)
    (privateParents : ContextPrivate heap roots (frame :: tail)) (newRep : NodeRep heap fresh replacement)
    (noReturn : ¬ HeapReach heap fresh frame.parent)
    (completed : assignEdge heap roots frame.parent frame.value.index fresh = some next) :
    NodeRep next root (plugInsertFrames ((frame :: tail).map HeapFrame.value) replacement) := by
  cases context with
  | up _ _ _ _ read siblings outer =>
      obtain ⟨byte, access⟩ := edge_hole_read heap old frame.edges frame.value.index frame.value.children siblings
      have parentRep : NodeRep heap frame.parent (plugInsert frame.value tree) :=
        .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
          (edge_hole_plug heap old frame.edges frame.value.index frame.value.children siblings tree focused)
      have safe := heap_context_private_safe heap roots frame.parent root tail outer (plugInsert frame.value tree)
        parentRep (privateParents frame (by simp)) (fun item kept => privateParents item (by simp [kept]))
      let changed := heapWrite heap frame.parent (some (.branch frame.value.pfx frame.value.terminal
        (repointedFrame frame byte fresh).edges))
      have context : HeapContext heap old root (frame :: tail) := .up old root frame tail read siblings outer
      have newContext := heap_context_repoint heap old fresh root frame tail tree byte context focused access safe
      have newFocus : NodeRep changed fresh replacement := node_rep_write_frame heap fresh frame.parent replacement _ newRep noReturn
      have newRoot := heap_context_plug changed fresh root (repointedFrame frame byte fresh :: tail) newContext replacement newFocus
      have transferred := edge_token_transfer_owned heap roots frame.parent old fresh frame.value.pfx frame.value.terminal
        frame.edges (edgeSet frame.value.index tree frame.value.children) frame.value.index byte replacement
        owned parentRep read access newRep noReturn
      have workOwned : HeapOwned changed (roots ++ [old]) :=
        heap_owned_permute changed (old :: roots) (roots ++ [old]) (List.perm_append_singleton old roots).symm transferred
      obtain ⟨actual, dropped, _, preserve⟩ := drop_loop_owned changed roots [old] workOwned
      have dropCompleted : dropLoop changed roots [old] = some next := by
        simpa only [assignEdge, swapEdgeToken, read, access, Option.map_some, Option.bind_some, changed, repointedFrame] using completed
      have same := Option.some.inj (dropped.symm.trans dropCompleted)
      subst actual
      exact preserve root _ member newRoot

theorem assign_place_leaf_result (state : MutHeap) (others : List NodeId) (old fresh : NodeId)
    (oldTree : Tree) (entry : Entry) (frames : List HeapFrame)
    (owned : HeapOwned state.heap (fresh :: state.root :: others)) (writable : WritablePlace state others)
    (context : HeapContext state.heap old state.root frames) (focused : NodeRep state.heap old oldTree)
    (privateParents : ContextPrivate state.heap (state.root :: others) frames)
    (aligned : PlaceAligned state.place frames) (newRep : NodeRep state.heap fresh (.leaf entry)) :
    ∃ next, assignPlace state others fresh = some next ∧ MutationResult state others frames (.leaf entry) next := by
  cases state with
  | mk heap root place =>
      cases place with
      | root =>
          cases frames with
          | cons _ _ => exact False.elim aligned
          | nil =>
              obtain ⟨next, completed, nextOwned, newRoot, preserve⟩ := assign_root_refines heap root fresh others (.leaf entry) owned newRep
              exact ⟨⟨next, fresh, .root⟩, by simp [assignPlace, completed], ⟨nextOwned, newRoot, preserve⟩⟩
      | edge parent index =>
          cases frames with
          | nil => exact False.elim aligned
          | cons frame tail =>
              obtain ⟨parentEq, indexEq⟩ := aligned
              have details : ∃ byte, frame.edges[frame.value.index]? = some (byte, old) ∧
                  heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges) ∧
                  NodeRep heap frame.parent (plugInsert frame.value oldTree) := by
                cases context with
                | up _ _ _ _ read siblings _ =>
                    obtain ⟨byte, access⟩ := edge_hole_read heap old frame.edges frame.value.index frame.value.children siblings
                    exact ⟨byte, access, read, .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
                      (edge_hole_plug heap old frame.edges frame.value.index frame.value.children siblings oldTree focused)⟩
              obtain ⟨byte, access, read, parentRep⟩ := details
              have leafRead : heapRead heap fresh = some (.leaf entry) := by cases newRep; assumption
              have different : frame.parent ≠ fresh := by
                intro same
                rw [same, leafRead] at read
                simp at read
              have noReturn : ¬ HeapReach heap fresh frame.parent := fun path => different (leaf_reach_self heap fresh entry leafRead _ path)
              have reachable : HeapReach heap root frame.parent := by simpa only [parentEq] using writable.2.2.1
              obtain ⟨next, completed, nextOwned, _, _, _, preserve⟩ := assign_edge_refines heap (root :: others)
                frame.parent old fresh root frame.value.pfx frame.value.terminal frame.edges
                (edgeSet frame.value.index oldTree frame.value.children) frame.value.index byte (.leaf entry)
                owned parentRep read access newRep noReturn (by simp) reachable
              refine ⟨⟨next, root, .edge parent index⟩, ?_, ⟨nextOwned, ?_, ?_⟩⟩
              · simp only [assignPlace, ← parentEq, ← indexEq, completed, Option.map_some]
              · exact assign_edge_context_root heap next (root :: others) old fresh root frame tail oldTree (.leaf entry)
                  owned (by simp) context focused privateParents newRep noReturn completed
              · intro saved value member original
                exact preserve saved value (by simp [member]) (by simpa only [parentEq] using writable.2.1 saved member) original

theorem replace_leaf_place_result (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame)
    (inv : MutationInvariant state others focus (.leaf old) frames) :
    ∃ next, replaceLeafPlace state others fresh = some next ∧ MutationResult state others frames (.leaf fresh) next := by
  have target := mutation_invariant_target state others focus (.leaf old) frames inv
  have writable := mutation_invariant_writable state others focus (.leaf old) frames inv
  have read : heapRead state.heap focus = some (.leaf old) := by cases inv.focused; assumption
  by_cases one : strongCount state.heap (state.root :: others) focus = 1
  · have focused := write_leaf_payload_refines state.heap focus old fresh read
    obtain ⟨nextInv, _, preserve⟩ := mutation_same_children_write state others focus (.leaf old) (.leaf fresh) frames
      (.leaf old) (.leaf fresh) inv one read rfl focused
    exact ⟨{ state with heap := heapWrite state.heap focus (some (.leaf fresh)) },
      by simp [replaceLeafPlace, target, read, one], ⟨nextInv.owned,
        heap_context_plug _ focus state.root frames nextInv.context (.leaf fresh) nextInv.focused, preserve⟩⟩
  · have allocated := heap_alloc_leaf_owned state.heap (state.root :: others) fresh inv.owned
    have allocatedWritable := writable_place_alloc_leaf state others fresh inv.owned writable
    have newRep : NodeRep (heapAlloc state.heap (.leaf fresh)) state.heap.length (.leaf fresh) :=
      .leaf state.heap.length fresh (heap_alloc_fresh state.heap (.leaf fresh))
    have privateParents : ContextPrivate (heapAlloc state.heap (.leaf fresh)) (state.root :: others) frames := by
      intro frame member
      simpa only [strong_count_alloc, storedChildren, List.map_nil, List.count_nil, Nat.add_zero] using inv.privateParents frame member
    obtain ⟨next, completed, result⟩ := assign_place_leaf_result { state with heap := heapAlloc state.heap (.leaf fresh) }
      others focus state.heap.length (.leaf old) fresh frames allocated allocatedWritable
      (heap_context_alloc state.heap focus state.root frames (.leaf fresh) inv.context)
      (node_rep_alloc state.heap focus (.leaf old) (.leaf fresh) inv.focused) privateParents inv.aligned newRep
    refine ⟨next, ?_, ⟨result.owned, result.rootValue, ?_⟩⟩
    · simpa only [replaceLeafPlace, target, Option.bind_some, read, if_neg one] using completed
    · intro saved value member original
      exact result.saved saved value member (node_rep_alloc state.heap saved value (.leaf fresh) original)

theorem replace_leaf_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame) (depth : Nat)
    (inv : MutationInvariant state others focus (.leaf old) frames) :
    executeInsert depth fresh (.leaf old) .replaceLeaf = .done (.leaf fresh) false ∧
      ∃ next, replaceLeafPlace state others fresh = some next ∧ MutationResult state others frames (.leaf fresh) next :=
  ⟨rfl, replace_leaf_place_result state others focus old fresh frames inv⟩

end Kv9.Radix
