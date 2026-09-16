import HeapNormalizeTerminal

set_option autoImplicit false

namespace Kv9.Radix

theorem represented_branch_read (heap : NodeHeap) (focus : NodeId) (pfx : Key)
    (terminal : Option Entry) (children : Forest) (rep : NodeRep heap focus (.branch pfx terminal children)) :
    ∃ edges, heapRead heap focus = some (.branch pfx terminal edges) ∧ EdgesRep heap edges children := by
  cases rep with
  | branch _ _ _ edges _ read descendants => exact ⟨edges, read, descendants⟩

-- A detached parent remains private while its separately owned child is made
-- unique. Copying the child adds references only to its descendants.
theorem make_owned_unique_observer_count (heap next : NodeHeap) (focus address observer : NodeId)
    (others : List NodeId) (node : StoredNode) (read : heapRead heap focus = some node)
    (bound : observer < heap.length) (noReturn : ¬ HeapReach heap focus observer)
    (completed : makeRootUnique heap focus others = some (next, address)) :
    strongCount next (address :: others) observer = strongCount heap (focus :: others) observer := by
  by_cases one : strongCount heap (focus :: others) focus = 1
  · have same : (heap, focus) = (next, address) := by
      simpa only [makeRootUnique, read, Option.map_some, if_pos one, Option.some.injEq] using completed
    cases same
    rfl
  · have same : (heapAlloc heap node, heap.length) = (next, address) := by
      simpa only [makeRootUnique, read, Option.map_some, if_neg one, Option.some.injEq] using completed
    cases same
    have different : focus ≠ observer := fun equal => noReturn (equal ▸ HeapReach.root)
    have freshDifferent : heap.length ≠ observer := Nat.ne_of_gt bound
    have empty := no_reach_children_count_zero heap focus observer node read noReturn
    simpa only [referenceHit, if_neg different, if_neg freshDifferent, empty, Nat.add_zero]
      using cloned_root_count_delta heap focus others node observer

def writeOwnedPrefix (heap : NodeHeap) (focus : NodeId) (pfx : Key) : Option NodeHeap :=
  match heapRead heap focus with
  | some (.branch _ terminal edges) => some (heapWrite heap focus (some (.branch pfx terminal edges)))
  | _ => none

theorem owned_prefix_write_refines (heap : NodeHeap) (focus : NodeId) (others : List NodeId)
    (oldPfx pfx : Key) (terminal : Option Entry) (children : Forest)
    (owned : HeapOwned heap (focus :: others)) (one : strongCount heap (focus :: others) focus = 1)
    (rep : NodeRep heap focus (.branch oldPfx terminal children)) :
    ∃ next, writeOwnedPrefix heap focus pfx = some next ∧ HeapOwned next (focus :: others) ∧
      NodeRep next focus (.branch pfx terminal children) ∧
      (∀ observer, strongCount next (focus :: others) observer = strongCount heap (focus :: others) observer) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  obtain ⟨edges, read, _⟩ := represented_branch_read heap focus oldPfx terminal children rep
  have focused := write_branch_payload_refines heap focus oldPfx pfx terminal terminal edges children rep read
  obtain ⟨nextOwned, _, keep⟩ := owned_write_same_children heap focus others (.branch oldPfx terminal edges)
    (.branch pfx terminal edges) (.branch pfx terminal children) owned one read rfl focused
  exact ⟨heapWrite heap focus (some (.branch pfx terminal edges)), by simp [writeOwnedPrefix, read], nextOwned, focused,
    fun observer => heap_write_same_targets_count heap (focus :: others) focus (.branch oldPfx terminal edges)
      (.branch pfx terminal edges) observer read rfl, keep⟩

-- The caller owns both the empty detached parent and its former child. These
-- writes retain the source order: child make_mut, parent mem::take, child
-- prefix moved out by append, and the joined prefix installed in the child.
-- Buffer moves here are value transitions; native allocation contracts remain
-- a separate obligation.
def mergeDetachedPrefix (heap : NodeHeap) (parent child : NodeId) (others : List NodeId) (byte : UInt8) :
    Option (NodeHeap × NodeId) := do
  let (copied, address) ← makeRootUnique heap child (parent :: others)
  match heapRead copied parent, heapRead copied address with
  | some (.branch parentPfx _ _), some (.branch childPfx _ _) =>
      let parentTaken ← writeOwnedPrefix copied parent []
      let childTaken ← writeOwnedPrefix parentTaken address []
      let joined ← writeOwnedPrefix childTaken address (parentPfx ++ byte :: childPfx)
      some (joined, address)
  | _, _ => none

theorem merge_detached_prefix_refines (heap : NodeHeap) (parent child : NodeId) (others : List NodeId)
    (parentPfx childPfx : Key) (terminal : Option Entry) (children : Forest) (byte : UInt8)
    (owned : HeapOwned heap (child :: parent :: others))
    (parentRep : NodeRep heap parent (.branch parentPfx none .nil))
    (childRep : NodeRep heap child (.branch childPfx terminal children))
    (parentOne : strongCount heap (child :: parent :: others) parent = 1) :
    ∃ next address, mergeDetachedPrefix heap parent child others byte = some (next, address) ∧
      HeapOwned next (address :: parent :: others) ∧
      NodeRep next address (.branch (parentPfx ++ byte :: childPfx) terminal children) ∧
      NodeRep next parent (.branch [] none .nil) ∧
      (∀ saved value, saved ∈ others → NodeRep heap saved value → NodeRep next saved value) := by
  have swap : (child :: parent :: others).Perm (parent :: child :: others) := List.Perm.swap _ _ _
  have originalOne : strongCount heap (parent :: child :: others) parent = 1 :=
    (strong_count_permute heap _ _ swap parent).symm.trans parentOne
  have noReturn := unique_root_separates heap parent (child :: others) originalOne child (by simp)
  have originalParentRead := empty_branch_read heap parent parentPfx none parentRep
  obtain ⟨originalEdges, originalChildRead, _⟩ := represented_branch_read heap child childPfx terminal children childRep
  obtain ⟨copied, address, copiedResult, copiedOwned, copiedRep, childOne, _, preserve⟩ :=
    make_owned_unique_refines heap child (parent :: others) (.branch childPfx terminal children) owned childRep
  have copiedParent := preserve parent (.branch parentPfx none .nil) (by simp) parentRep
  have copiedParentRead := empty_branch_read copied parent parentPfx none copiedParent
  obtain ⟨edges, copiedChildRead, _⟩ := represented_branch_read copied address childPfx terminal children copiedRep
  have parentCount := (make_owned_unique_observer_count heap copied child address parent (parent :: others)
    (.branch childPfx terminal originalEdges) originalChildRead
    (heap_read_bound heap parent _ originalParentRead) noReturn copiedResult).trans parentOne
  have copiedSwap : (address :: parent :: others).Perm (parent :: address :: others) := List.Perm.swap _ _ _
  have swappedOwned := heap_owned_permute copied _ _ copiedSwap copiedOwned
  have swappedOne := (strong_count_permute copied _ _ copiedSwap parent).symm.trans parentCount
  obtain ⟨parentTaken, parentResult, parentTakenOwned, emptyParent, parentCounts, keepParent⟩ :=
    owned_prefix_write_refines copied parent (address :: others) parentPfx [] none .nil swappedOwned swappedOne copiedParent
  have parentTakenChild := keepParent address (.branch childPfx terminal children) (by simp) copiedRep
  have parentTakenSwap := heap_owned_permute parentTaken _ _ copiedSwap.symm parentTakenOwned
  have parentTakenOne : strongCount parentTaken (address :: parent :: others) address = 1 := by
    rw [strong_count_permute parentTaken _ _ copiedSwap address, parentCounts address]
    exact (strong_count_permute copied _ _ copiedSwap address).symm.trans childOne
  obtain ⟨childTaken, childResult, childTakenOwned, emptyChild, childCounts, keepChild⟩ :=
    owned_prefix_write_refines parentTaken address (parent :: others) childPfx [] terminal children
      parentTakenSwap parentTakenOne parentTakenChild
  have childTakenOne := (childCounts address).trans parentTakenOne
  obtain ⟨joined, joinedResult, joinedOwned, joinedChild, _, keepJoined⟩ :=
    owned_prefix_write_refines childTaken address (parent :: others) [] (parentPfx ++ byte :: childPfx) terminal children
      childTakenOwned childTakenOne emptyChild
  refine ⟨joined, address, ?_, joinedOwned, joinedChild, ?_, ?_⟩
  · simp [mergeDetachedPrefix, copiedResult, copiedParentRead, copiedChildRead, parentResult, childResult, joinedResult]
  · exact keepJoined parent (.branch [] none .nil) (by simp)
      (keepChild parent (.branch [] none .nil) (by simp) emptyParent)
  · intro saved value member original
    exact keepJoined saved value (by simp [member]) (keepChild saved value (by simp [member])
      (keepParent saved value (by simp [member]) (preserve saved value (by simp [member]) original)))

end Kv9.Radix
