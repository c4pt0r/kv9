import HeapInsertStep

set_option autoImplicit false

namespace Kv9.Radix

-- This evaluator follows the physical place transitions. It carries neither
-- a ghost Tree nor a parent-frame vector at runtime. Every actual descent
-- consumes at least one byte of the fresh key, so no fuel/check is inserted.
def insertPlaceLoop (state : MutHeap) (others : List NodeId) (depth : Nat) (fresh : Entry) : Option (MutHeap × Bool) :=
  match _transition : stepPlaceInsert state others depth fresh with
  | .failed => none
  | .done next inserted => some (next, inserted)
  | .down next nextDepth => insertPlaceLoop next others nextDepth fresh
termination_by fresh.key.length - depth
decreasing_by exact (step_place_down_bounds state next others depth nextDepth fresh _transition).2.2

theorem insert_place_loop_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (path : Key) (fresh : Entry) (tree : Tree) (frames : List HeapFrame)
    (inv : MutationInvariant state others focus tree frames)
    (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
    ∃ next, insertPlaceLoop state others path.length fresh = some (next, (insertTree path fresh tree).2) ∧
      MutationResult state others frames (insertTree path fresh tree).1 next := by
  have correct := step_insert_refines path fresh tree valid ordered freshValid
  have simulation := step_place_insert_refines state others focus tree frames path.length fresh inv
  cases transition : stepInsert path.length fresh tree with
  | failed => simp only [transition, InsertStepCorrect] at correct
  | done replacement inserted =>
      have same : (replacement, inserted) = insertTree path fresh tree := by
        simpa only [transition, InsertStepCorrect] using correct
      obtain ⟨next, actual, result⟩ := (by simpa only [transition, PlaceStepRep] using simulation :
        ∃ next, stepPlaceInsert state others path.length fresh = .done next inserted ∧ MutationResult state others frames replacement next)
      refine ⟨next, ?_, ?_⟩
      · rw [insertPlaceLoop, actual]
        simp only [← same]
      · simpa only [← same] using result
  | down frame child nextDepth =>
      obtain ⟨childPath, childValid, childOrdered, childPrefix, depthEq, _indexSafe, _parentEq, _access, result⟩ :=
        (by simpa only [transition, InsertStepCorrect] using correct :
          ∃ childPath, Valid childPath child ∧ OrderedTree child ∧ HasPrefix childPath fresh.key ∧
            nextDepth = childPath.length ∧ frame.index < edgeCount frame.children ∧
            tree = .branch frame.pfx frame.terminal frame.children ∧
            (∃ byte, edgeGet frame.index frame.children = some (byte, child)) ∧
            insertTree path fresh tree =
              (plugInsert frame (insertTree childPath fresh child).1, (insertTree childPath fresh child).2))
      obtain ⟨advanced, address, nextFrames, actual, nextInv, values, preserve⟩ :=
        (by simpa only [transition, PlaceStepRep] using simulation :
          ∃ next address nextFrames, stepPlaceInsert state others path.length fresh = .down next nextDepth ∧
            MutationInvariant next others address child nextFrames ∧
            nextFrames.map HeapFrame.value = frame :: frames.map HeapFrame.value ∧
            (∀ saved value, saved ∈ others → NodeRep state.heap saved value → NodeRep next.heap saved value))
      have smaller : fresh.key.length - childPath.length < fresh.key.length - path.length := by
        simpa only [depthEq] using (step_place_down_bounds state advanced others path.length nextDepth fresh actual).2.2
      obtain ⟨next, completed, final⟩ := insert_place_loop_refines advanced others address childPath fresh child nextFrames
        nextInv childValid childOrdered childPrefix
      refine ⟨next, ?_, ⟨final.owned, ?_, ?_⟩⟩
      · rw [insertPlaceLoop, actual, depthEq]
        simpa only [result] using completed
      · simpa only [values, plugInsertFrames, result] using final.rootValue
      · intro saved value member original
        exact final.saved saved value member (preserve saved value member original)
termination_by fresh.key.length - path.length
decreasing_by assumption

theorem insert_place_loop_no_failure (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (path : Key) (fresh : Entry) (tree : Tree) (frames : List HeapFrame)
    (inv : MutationInvariant state others focus tree frames)
    (valid : Valid path tree) (ordered : OrderedTree tree) (freshValid : HasPrefix path fresh.key) :
    insertPlaceLoop state others path.length fresh ≠ none := by
  obtain ⟨next, completed, _⟩ := insert_place_loop_refines state others focus path fresh tree frames inv valid ordered freshValid
  rw [completed]
  simp

def RootRep (heap : NodeHeap) : Option NodeId → Option Tree → Prop
  | none, none => True
  | some address, some tree => NodeRep heap address tree
  | _, _ => False

-- The root option is the actual map state; saved roots own independent Arc
-- tokens. Cardinality storage and native word arithmetic are separate bridges.
def putHeap (heap : NodeHeap) (root : Option NodeId) (others : List NodeId) (fresh : Entry) : Option (NodeHeap × NodeId × Bool) :=
  match root with
  | none => some (heapAlloc heap (.leaf fresh), heap.length, true)
  | some address => (insertPlaceLoop ⟨heap, address, .root⟩ others 0 fresh).map
      (fun (next, inserted) => (next.heap, next.root, inserted))

theorem put_heap_refines (heap : NodeHeap) (root : Option NodeId) (others : List NodeId)
    (tree : Option Tree) (fresh : Entry) (owned : HeapOwned heap (root.toList ++ others))
    (rep : RootRep heap root tree) (good : Good tree) :
    ∃ next address, putHeap heap root others fresh = some (next, address, (putRoot fresh tree).2) ∧
      HeapOwned next (address :: others) ∧ NodeRep next address (putRoot fresh tree).1 ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  cases root with
  | none =>
      cases tree with
      | some => exact False.elim rep
      | none =>
          exact ⟨heapAlloc heap (.leaf fresh), heap.length, rfl, heap_alloc_leaf_owned heap others fresh owned,
            .leaf heap.length fresh (heap_alloc_fresh heap (.leaf fresh)),
            fun saved value _ original => node_rep_alloc heap saved value (.leaf fresh) original⟩
  | some address =>
      cases tree with
      | none => exact False.elim rep
      | some tree =>
          have inv := mutation_invariant_initial heap address others tree owned rep
          obtain ⟨next, completed, result⟩ := insert_place_loop_refines ⟨heap, address, .root⟩ others address [] fresh tree [] inv
            good.1 (good.2 tree rfl).2 ⟨fresh.key, rfl⟩
          change insertPlaceLoop ⟨heap, address, .root⟩ others 0 fresh = some (next, (insertTree [] fresh tree).2) at completed
          exact ⟨next.heap, next.root, by simp only [putHeap, completed, Option.map_some, putRoot],
            result.owned, result.rootValue, result.saved⟩

end Kv9.Radix
