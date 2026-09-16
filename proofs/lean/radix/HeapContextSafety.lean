import HeapContext

set_option autoImplicit false

namespace Kv9.Radix

theorem unique_edge_dominates (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId) (index : Nat)
    (other : ArcSlot) (root : NodeId) (one : strongCount heap roots child = 1)
    (selected : arcSlotRead heap roots (.edge parent index) = some child)
    (incoming : arcSlotRead heap roots other = some root) (different : other ≠ .edge parent index)
    (separate : ¬ HeapReach heap root parent) : ¬ HeapReach heap root child := by
  intro reachable
  cases reachable with
  | root =>
      exact different (strong_one_unique_slot heap roots child other (.edge parent index) one incoming selected)
  | child lastParent _ node byte previous read edge =>
      obtain ⟨lastIndex, access⟩ := List.mem_iff_getElem?.mp edge
      have lastSlot : arcSlotRead heap roots (.edge lastParent lastIndex) = some child := by
        simp only [arcSlotRead, read, Option.bind_some, access, Option.map_some]
      have same := strong_one_unique_slot heap roots child (.edge lastParent lastIndex) (.edge parent index) one lastSlot selected
      have sameParent := (ArcSlot.edge.inj same).1
      exact separate (sameParent ▸ previous)

theorem edge_hole_safe_of_indices (heap : NodeHeap) (changed : NodeId) (edges : List StoredEdge)
    (index : Nat) (selected : StoredEdge) (access : edges[index]? = some selected)
    (others : ∀ other edge, edges[other]? = some edge → other ≠ index → ¬ HeapReach heap edge.2 changed) :
    EdgeHoleSafe heap changed edges index := by
  induction index generalizing edges with
  | zero =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          intro edge member
          obtain ⟨other, read⟩ := List.mem_iff_getElem?.mp member
          exact others (other + 1) edge read (by omega)
  | succ index ih =>
      cases edges with
      | nil => simp at access
      | cons head tail =>
          refine ⟨others 0 head rfl (by omega), ih tail access ?_⟩
          intro other edge read different
          exact others (other + 1) edge read (by omega)

theorem unique_child_siblings_safe (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId)
    (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (one : strongCount heap roots child = 1)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, child)) :
    EdgeHoleSafe heap child edges index := by
  have selected : arcSlotRead heap roots (.edge parent index) = some child := by
    simp only [arcSlotRead, read, Option.bind_some, storedChildren, access, Option.map_some]
  apply edge_hole_safe_of_indices heap child edges index (byte, child) access
  intro other edge otherAccess different
  have incoming : arcSlotRead heap roots (.edge parent other) = some edge.2 := by
    simp only [arcSlotRead, read, Option.bind_some, storedChildren, otherAccess, Option.map_some]
  have slotDifferent : ArcSlot.edge parent other ≠ .edge parent index := fun same => different (ArcSlot.edge.inj same).2
  exact unique_edge_dominates heap roots parent child index (.edge parent other) edge.2 one selected incoming slotDifferent
    (represented_edge_no_return heap parent (.branch pfx terminal children) (.branch pfx terminal edges)
      edge.1 edge.2 rep read (List.mem_of_getElem? otherAccess))

theorem edge_hole_safe_change_target (heap : NodeHeap) (before after : NodeId) (edges : List StoredEdge) (index : Nat)
    (safe : EdgeHoleSafe heap before edges index)
    (transfer : ∀ edge ∈ edges, ¬ HeapReach heap edge.2 before → ¬ HeapReach heap edge.2 after) :
    EdgeHoleSafe heap after edges index := by
  induction index generalizing edges with
  | zero =>
      cases edges with
      | nil => exact False.elim safe
      | cons head tail =>
          intro edge member
          exact transfer edge (by simp [member]) (safe edge member)
  | succ index ih =>
      cases edges with
      | nil => exact False.elim safe
      | cons head tail =>
          exact ⟨transfer head (by simp) safe.1,
            ih tail safe.2 (fun edge member => transfer edge (by simp [member]))⟩

theorem heap_context_frame_read (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) (frame : HeapFrame) (member : frame ∈ frames) :
    heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges) := by
  induction context with
  | root => simp at member
  | up hole root head tail read siblings outer ih =>
      rcases List.mem_cons.mp member with same | later
      · subst frame
        exact read
      · exact ih later

theorem heap_context_ancestor_reach (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) (frame : HeapFrame) (member : frame ∈ frames) :
    HeapReach heap frame.parent hole := by
  induction context with
  | root => simp at member
  | up hole root head tail read siblings outer ih =>
      obtain ⟨byte, access⟩ := edge_hole_read heap hole head.edges head.value.index head.value.children siblings
      rcases List.mem_cons.mp member with same | later
      · subst frame
        exact .child head.parent hole _ byte .root read (List.mem_of_getElem? access)
      · exact .child head.parent hole _ byte (ih later) read (List.mem_of_getElem? access)

theorem context_safe_child (heap : NodeHeap) (roots : List NodeId) (parent child : NodeId) (index : Nat)
    (frames : List HeapFrame) (one : strongCount heap roots child = 1)
    (selected : arcSlotRead heap roots (.edge parent index) = some child) (noReturn : ¬ HeapReach heap child parent)
    (reads : ∀ frame ∈ frames, heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges))
    (ancestors : ∀ frame ∈ frames, HeapReach heap frame.parent parent) (safe : ContextSafe heap parent frames) :
    ContextSafe heap child frames := by
  induction frames with
  | nil => trivial
  | cons frame tail ih =>
      have read := reads frame (by simp)
      have different : frame.parent ≠ child := fun same => noReturn (same ▸ ancestors frame (by simp))
      refine ⟨different, ?_, ih (fun item member => reads item (by simp [member]))
        (fun item member => ancestors item (by simp [member])) safe.2.2⟩
      apply edge_hole_safe_change_target heap parent child frame.edges frame.value.index safe.2.1
      intro edge member separate
      obtain ⟨other, access⟩ := List.mem_iff_getElem?.mp member
      have incoming : arcSlotRead heap roots (.edge frame.parent other) = some edge.2 := by
        simp only [arcSlotRead, read, Option.bind_some, storedChildren, access, Option.map_some]
      have slotDifferent : ArcSlot.edge frame.parent other ≠ .edge parent index := fun same => safe.1 (ArcSlot.edge.inj same).1
      exact unique_edge_dominates heap roots parent child index (.edge frame.parent other) edge.2 one selected incoming slotDifferent separate

def ContextPrivate (heap : NodeHeap) (roots : List NodeId) (frames : List HeapFrame) : Prop :=
  ∀ frame ∈ frames, strongCount heap roots frame.parent = 1

-- Safety is derived from the actual reference inventory and private ancestor
-- chain, rather than assumed merely because the focused Arc is count-one.
theorem heap_context_private_safe (heap : NodeHeap) (roots : List NodeId) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) (tree : Tree) (focused : NodeRep heap hole tree)
    (one : strongCount heap roots hole = 1) (privateParents : ContextPrivate heap roots frames) :
    ContextSafe heap hole frames := by
  induction context generalizing tree with
  | root => trivial
  | up hole root frame tail read siblings outer ih =>
      have parentRep : NodeRep heap frame.parent (plugInsert frame.value tree) :=
        .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
          (edge_hole_plug heap hole frame.edges frame.value.index frame.value.children siblings tree focused)
      obtain ⟨byte, access⟩ := edge_hole_read heap hole frame.edges frame.value.index frame.value.children siblings
      have noReturn := represented_edge_no_return heap frame.parent (plugInsert frame.value tree)
        (.branch frame.value.pfx frame.value.terminal frame.edges) byte hole parentRep read (List.mem_of_getElem? access)
      have different : frame.parent ≠ hole := fun same => noReturn (same ▸ (HeapReach.root : HeapReach heap frame.parent frame.parent))
      have outerPrivate : ContextPrivate heap roots tail := fun item member => privateParents item (by simp [member])
      have outerSafe := ih (plugInsert frame.value tree) parentRep (privateParents frame (by simp)) outerPrivate
      have selected : arcSlotRead heap roots (.edge frame.parent frame.value.index) = some hole := by
        simp only [arcSlotRead, read, Option.bind_some, storedChildren, access, Option.map_some]
      refine ⟨different, ?_, context_safe_child heap roots frame.parent hole frame.value.index tail one selected noReturn
        (fun item member => heap_context_frame_read heap frame.parent root tail outer item member)
        (fun item member => heap_context_ancestor_reach heap frame.parent root tail outer item member) outerSafe⟩
      exact unique_child_siblings_safe heap roots frame.parent hole frame.value.pfx frame.value.terminal frame.edges
        (edgeSet frame.value.index tree frame.value.children) frame.value.index byte one parentRep read access

theorem replace_unique_leaf_private_root (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame) (target : placeTarget state = some focus)
    (read : heapRead state.heap focus = some (.leaf old)) (unique : strongCount state.heap (state.root :: others) focus = 1)
    (context : HeapContext state.heap focus state.root frames) (privateParents : ContextPrivate state.heap (state.root :: others) frames) :
    replaceLeafPlace state others fresh = some { state with heap := heapWrite state.heap focus (some (.leaf fresh)) } ∧
      NodeRep (heapWrite state.heap focus (some (.leaf fresh))) state.root
        (plugInsertFrames (frames.map HeapFrame.value) (.leaf fresh)) :=
  replace_unique_leaf_root_value state others focus old fresh frames target read unique context
    (heap_context_private_safe state.heap (state.root :: others) focus state.root frames context (.leaf old)
      (.leaf focus old read) unique privateParents)

end Kv9.Radix
