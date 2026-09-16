import HeapGraph

set_option autoImplicit false

namespace Kv9.Radix

-- External slots include live/saved map roots, deletion frames/current and
-- owned destructor worklist references. Each slot contributes one strong ref.
inductive ArcSlot where
  | root (index : Nat)
  | edge (parent : NodeId) (index : Nat)
deriving DecidableEq, Repr

def arcSlotRead (heap : NodeHeap) (roots : List NodeId) : ArcSlot → Option NodeId
  | .root index => roots[index]?
  | .edge parent index => (heapRead heap parent).bind (fun node =>
      ((storedChildren node)[index]?).map Prod.snd)

def nodeHandles (parent : NodeId) (node : StoredNode) : List (ArcSlot × NodeId) :=
  (storedChildren node).zipIdx.map (fun pair => (.edge parent pair.2, pair.1.2))

def arcHandles (heap : NodeHeap) (roots : List NodeId) : List (ArcSlot × NodeId) :=
  roots.zipIdx.map (fun pair => (.root pair.2, pair.1)) ++
    heap.zipIdx.flatMap (fun pair => match pair.1 with
      | none => []
      | some node => nodeHandles pair.2 node)

-- Counts come from concrete reference locations, including shared child edges;
-- they are not an assumed assertion that unique references isolate snapshots.
def strongCount (heap : NodeHeap) (roots : List NodeId) (address : NodeId) : Nat :=
  ((arcHandles heap roots).filter (fun pair => pair.2 == address)).length

theorem heap_read_cell (heap : NodeHeap) (address : NodeId) (node : StoredNode)
    (read : heapRead heap address = some node) : heap[address]? = some (some node) := by
  cases access : heap[address]? with
  | none => simp [heapRead, access] at read
  | some cell =>
      cases cell with
      | none => simp [heapRead, access] at read
      | some stored =>
          have same : stored = node := by simpa only [heapRead, access, Option.join_some, Option.some.injEq] using read
          simp [same]

theorem arc_slot_in_handles (heap : NodeHeap) (roots : List NodeId) (slot : ArcSlot) (address : NodeId)
    (read : arcSlotRead heap roots slot = some address) : (slot, address) ∈ arcHandles heap roots := by
  cases slot with
  | root index =>
      apply List.mem_append.mpr (Or.inl _)
      exact List.mem_map.mpr ⟨(address, index), List.mk_mem_zipIdx_iff_getElem?.mpr read, rfl⟩
  | edge parent index =>
      cases parentRead : heapRead heap parent with
      | none => simp [arcSlotRead, parentRead] at read
      | some node =>
          cases edgeRead : (storedChildren node)[index]? with
          | none => simp [arcSlotRead, parentRead, edgeRead] at read
          | some pair =>
              obtain ⟨byte, child⟩ := pair
              have same : child = address := by simpa only [arcSlotRead, parentRead, Option.bind_some, edgeRead,
                Option.map_some, Option.some.injEq] using read
              subst child
              apply List.mem_append.mpr (Or.inr _)
              apply List.mem_flatMap.mpr
              refine ⟨(some node, parent), List.mk_mem_zipIdx_iff_getElem?.mpr (heap_read_cell heap parent node parentRead), ?_⟩
              exact List.mem_map.mpr ⟨((byte, address), index), List.mk_mem_zipIdx_iff_getElem?.mpr edgeRead, rfl⟩

theorem strong_one_unique_slot (heap : NodeHeap) (roots : List NodeId) (address : NodeId) (a b : ArcSlot)
    (one : strongCount heap roots address = 1)
    (first : arcSlotRead heap roots a = some address) (second : arcSlotRead heap roots b = some address) : a = b := by
  obtain ⟨only, singleton⟩ := List.length_eq_one_iff.mp one
  have left : (a, address) ∈ (arcHandles heap roots).filter (fun pair => pair.2 == address) :=
    List.mem_filter.mpr ⟨arc_slot_in_handles heap roots a address first, by simp⟩
  have right : (b, address) ∈ (arcHandles heap roots).filter (fun pair => pair.2 == address) :=
    List.mem_filter.mpr ⟨arc_slot_in_handles heap roots b address second, by simp⟩
  rw [singleton] at left right
  have same : (a, address) = (b, address) := (List.mem_singleton.mp left).trans (List.mem_singleton.mp right).symm
  exact congrArg Prod.fst same

def HeapSeparated (heap : NodeHeap) (saved : List NodeId) (address : NodeId) : Prop :=
  ∀ root ∈ saved, ¬ HeapReach heap root address

theorem unique_root_separates (heap : NodeHeap) (focus : NodeId) (saved : List NodeId)
    (one : strongCount heap (focus :: saved) focus = 1) : HeapSeparated heap saved focus := by
  intro root member reachable
  cases reachable with
  | root =>
      obtain ⟨index, access⟩ := List.mem_iff_getElem?.mp member
      have same := strong_one_unique_slot heap (focus :: saved) focus (.root 0) (.root (index + 1)) one rfl
        (by simpa only [arcSlotRead, List.getElem?_cons_succ] using access)
      have impossible := ArcSlot.root.inj same
      omega
  | child parent _ node byte _ read edge =>
      obtain ⟨index, access⟩ := List.mem_iff_getElem?.mp edge
      have owner : arcSlotRead heap (focus :: saved) (.edge parent index) = some focus := by
        simp only [arcSlotRead, read, Option.bind_some, access, Option.map_some]
      have same := strong_one_unique_slot heap (focus :: saved) focus (.root 0) (.edge parent index) one rfl owner
      cases same

theorem unique_child_separates (heap : NodeHeap) (roots saved : List NodeId) (parent child : NodeId) (index : Nat)
    (included : ∀ root ∈ saved, root ∈ roots) (parentSeparate : HeapSeparated heap saved parent)
    (read : arcSlotRead heap roots (.edge parent index) = some child)
    (one : strongCount heap roots child = 1) : HeapSeparated heap saved child := by
  intro root member reachable
  cases reachable with
  | root =>
      obtain ⟨rootIndex, access⟩ := List.mem_iff_getElem?.mp (included child member)
      have same := strong_one_unique_slot heap roots child (.edge parent index) (.root rootIndex) one read access
      cases same
  | child otherParent _ node byte previous parentRead edge =>
      obtain ⟨otherIndex, access⟩ := List.mem_iff_getElem?.mp edge
      have owner : arcSlotRead heap roots (.edge otherParent otherIndex) = some child := by
        simp only [arcSlotRead, parentRead, Option.bind_some, access, Option.map_some]
      have same := strong_one_unique_slot heap roots child (.edge parent index) (.edge otherParent otherIndex) one read owner
      have parentEq := (ArcSlot.edge.inj same).1
      rw [← parentEq] at previous
      exact parentSeparate root member previous

theorem separated_write_preserves_saved (heap : NodeHeap) (saved : List NodeId) (focus : NodeId)
    (replacement : Option StoredNode) (separate : HeapSeparated heap saved focus)
    (root : NodeId) (tree : Tree) (member : root ∈ saved) (rep : NodeRep heap root tree) :
    NodeRep (heapWrite heap focus replacement) root tree :=
  node_rep_write_frame heap root focus tree replacement rep (separate root member)

end Kv9.Radix
