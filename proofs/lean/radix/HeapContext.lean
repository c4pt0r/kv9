import HeapPlaceLeaf

set_option autoImplicit false

namespace Kv9.Radix

-- Ghost contexts describe the actual nested mutable place. They are not a
-- runtime parent Vec. Selected children are holes; all sibling values remain
-- represented at their concrete identities and indices.
inductive EdgeHoleRep (heap : NodeHeap) (hole : NodeId) : List StoredEdge → Nat → Forest → Prop where
  | here (byte : UInt8) (tail : List StoredEdge) (old : Tree) (rest : Forest)
      (remaining : EdgesRep heap tail rest) :
      EdgeHoleRep heap hole ((byte, hole) :: tail) 0 (.cons byte old rest)
  | there (byte : UInt8) (address : NodeId) (tail : List StoredEdge) (child : Tree) (rest : Forest) (index : Nat)
      (node : NodeRep heap address child) (remaining : EdgeHoleRep heap hole tail index rest) :
      EdgeHoleRep heap hole ((byte, address) :: tail) (index + 1) (.cons byte child rest)

theorem edge_hole_read (heap : NodeHeap) (hole : NodeId) (edges : List StoredEdge) (index : Nat) (children : Forest)
    (rep : EdgeHoleRep heap hole edges index children) : ∃ byte, edges[index]? = some (byte, hole) := by
  induction rep with
  | here byte => exact ⟨byte, rfl⟩
  | there byte address tail child rest index node remaining ih => exact ih

theorem edge_hole_plug (heap : NodeHeap) (hole : NodeId) (edges : List StoredEdge) (index : Nat) (children : Forest)
    (rep : EdgeHoleRep heap hole edges index children) (tree : Tree) (focused : NodeRep heap hole tree) :
    EdgesRep heap edges (edgeSet index tree children) := by
  induction rep with
  | here byte tail old rest remaining => exact .cons byte hole tail tree rest focused remaining
  | there byte address tail child rest index node remaining ih =>
      exact .cons byte address tail child (edgeSet index tree rest) node ih

structure HeapFrame where
  parent : NodeId
  edges : List StoredEdge
  value : InsertFrame

inductive HeapContext (heap : NodeHeap) : NodeId → NodeId → List HeapFrame → Prop where
  | root (hole : NodeId) : HeapContext heap hole hole []
  | up (hole root : NodeId) (frame : HeapFrame) (tail : List HeapFrame)
      (read : heapRead heap frame.parent = some (.branch frame.value.pfx frame.value.terminal frame.edges))
      (siblings : EdgeHoleRep heap hole frame.edges frame.value.index frame.value.children)
      (outer : HeapContext heap frame.parent root tail) : HeapContext heap hole root (frame :: tail)

theorem heap_context_plug (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) (tree : Tree) (focused : NodeRep heap hole tree) :
    NodeRep heap root (plugInsertFrames (frames.map HeapFrame.value) tree) := by
  induction context generalizing tree with
  | root => exact focused
  | up hole root frame tail read siblings outer ih =>
      have parent : NodeRep heap frame.parent (plugInsert frame.value tree) :=
        .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
          (edge_hole_plug heap hole frame.edges frame.value.index frame.value.children siblings tree focused)
      exact ih _ parent

theorem heap_context_reach (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) : HeapReach heap root hole := by
  induction context with
  | root => exact .root
  | up hole root frame tail read siblings outer ih =>
      obtain ⟨byte, access⟩ := edge_hole_read heap hole frame.edges frame.value.index frame.value.children siblings
      exact .child frame.parent hole (.branch frame.value.pfx frame.value.terminal frame.edges) byte ih read (List.mem_of_getElem? access)

def EdgeHoleSafe (heap : NodeHeap) (changed : NodeId) : List StoredEdge → Nat → Prop
  | [], _ => False
  | _ :: tail, 0 => ∀ edge ∈ tail, ¬ HeapReach heap edge.2 changed
  | edge :: tail, index + 1 => ¬ HeapReach heap edge.2 changed ∧ EdgeHoleSafe heap changed tail index

def ContextSafe (heap : NodeHeap) (changed : NodeId) : List HeapFrame → Prop
  | [] => True
  | frame :: tail => frame.parent ≠ changed ∧ EdgeHoleSafe heap changed frame.edges frame.value.index ∧ ContextSafe heap changed tail

theorem edge_hole_write_frame (heap : NodeHeap) (hole changed : NodeId) (node : Option StoredNode)
    (edges : List StoredEdge) (index : Nat) (children : Forest)
    (rep : EdgeHoleRep heap hole edges index children) (safe : EdgeHoleSafe heap changed edges index) :
    EdgeHoleRep (heapWrite heap changed node) hole edges index children := by
  induction rep with
  | here byte tail old rest remaining =>
      exact .here byte tail old rest (edges_rep_write_frame heap tail rest changed node remaining safe)
  | there byte address tail child rest index focused remaining ih =>
      exact .there byte address tail child rest index (node_rep_write_frame heap address changed child node focused safe.1) (ih safe.2)

theorem heap_context_write_frame (heap : NodeHeap) (hole root changed : NodeId) (frames : List HeapFrame)
    (node : Option StoredNode) (context : HeapContext heap hole root frames) (safe : ContextSafe heap changed frames) :
    HeapContext (heapWrite heap changed node) hole root frames := by
  induction context with
  | root => exact .root _
  | up hole root frame tail read siblings outer ih =>
      apply HeapContext.up hole root frame tail
      · rw [heap_write_elsewhere heap changed frame.parent node safe.1]
        exact read
      · exact edge_hole_write_frame heap hole changed node frame.edges frame.value.index frame.value.children siblings safe.2.1
      · exact ih safe.2.2

theorem heap_context_write_value (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (node : Option StoredNode) (tree : Tree) (context : HeapContext heap hole root frames)
    (safe : ContextSafe heap hole frames) (focused : NodeRep (heapWrite heap hole node) hole tree) :
    NodeRep (heapWrite heap hole node) root (plugInsertFrames (frames.map HeapFrame.value) tree) :=
  heap_context_plug (heapWrite heap hole node) hole root frames
    (heap_context_write_frame heap hole root hole frames node context safe) tree focused

-- Local root-value composition for the unique ReplaceLeaf branch. Establishing
-- and transporting these context invariants through all descents/COW/splits is
-- an obligation of the complete insertion loop, not an assumed finished result.
theorem replace_unique_leaf_root_value (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (old fresh : Entry) (frames : List HeapFrame) (target : placeTarget state = some focus)
    (read : heapRead state.heap focus = some (.leaf old))
    (unique : strongCount state.heap (state.root :: others) focus = 1)
    (context : HeapContext state.heap focus state.root frames) (safe : ContextSafe state.heap focus frames) :
    replaceLeafPlace state others fresh = some { state with heap := heapWrite state.heap focus (some (.leaf fresh)) } ∧
      NodeRep (heapWrite state.heap focus (some (.leaf fresh))) state.root
        (plugInsertFrames (frames.map HeapFrame.value) (.leaf fresh)) := by
  refine ⟨by simp [replaceLeafPlace, target, read, unique], ?_⟩
  exact heap_context_write_value state.heap focus state.root frames (some (.leaf fresh)) (.leaf fresh) context safe
    (write_leaf_payload_refines state.heap focus old fresh read)

end Kv9.Radix
