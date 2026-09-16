import HeapContextSafety

set_option autoImplicit false

namespace Kv9.Radix

theorem heap_reach_transitive (heap : NodeHeap) (a b c : NodeId)
    (first : HeapReach heap a b) (second : HeapReach heap b c) : HeapReach heap a c := by
  induction second with
  | root => exact first
  | child parent address node byte _ read edge ih => exact .child parent address node byte ih read edge

theorem heap_context_ancestor_no_return (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame)
    (context : HeapContext heap hole root frames) (tree : Tree) (focused : NodeRep heap hole tree)
    (frame : HeapFrame) (member : frame ∈ frames) : ¬ HeapReach heap hole frame.parent := by
  induction context generalizing tree with
  | root => simp at member
  | up hole root head tail read siblings outer ih =>
      obtain ⟨byte, access⟩ := edge_hole_read heap hole head.edges head.value.index head.value.children siblings
      have parentRep : NodeRep heap head.parent (plugInsert head.value tree) :=
        .branch head.parent head.value.pfx head.value.terminal head.edges _ read
          (edge_hole_plug heap hole head.edges head.value.index head.value.children siblings tree focused)
      rcases List.mem_cons.mp member with same | later
      · subst frame
        exact represented_edge_no_return heap head.parent (plugInsert head.value tree)
          (.branch head.value.pfx head.value.terminal head.edges) byte hole parentRep read (List.mem_of_getElem? access)
      · intro back
        have down : HeapReach heap head.parent hole := .child head.parent hole _ byte .root read (List.mem_of_getElem? access)
        exact ih (plugInsert head.value tree) parentRep later (heap_reach_transitive heap head.parent hole frame.parent down back)

theorem edge_hole_from_edges (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (hole : NodeId) (rep : EdgesRep heap edges children)
    (access : edges[index]? = some (byte, hole)) : EdgeHoleRep heap hole edges index children := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => simp at access
      | cons storedByte address tail child rest node remaining =>
          have same : storedByte = byte ∧ address = hole := by simpa using access
          rcases same with ⟨rfl, rfl⟩
          exact .here _ tail child rest remaining
  | succ index ih =>
      cases rep with
      | nil => simp at access
      | cons storedByte address tail child rest node remaining =>
          exact .there storedByte address tail child rest index node (ih tail rest remaining access)

theorem edge_hole_alloc (heap : NodeHeap) (hole : NodeId) (edges : List StoredEdge) (index : Nat)
    (children : Forest) (node : StoredNode) (rep : EdgeHoleRep heap hole edges index children) :
    EdgeHoleRep (heapAlloc heap node) hole edges index children := by
  induction rep with
  | here byte tail old rest remaining => exact .here byte tail old rest (edges_rep_alloc heap tail rest node remaining)
  | there byte address tail child rest index focused remaining ih =>
      exact .there byte address tail child rest index (node_rep_alloc heap address child node focused) ih

theorem heap_context_alloc (heap : NodeHeap) (hole root : NodeId) (frames : List HeapFrame) (node : StoredNode)
    (context : HeapContext heap hole root frames) : HeapContext (heapAlloc heap node) hole root frames := by
  induction context with
  | root => exact .root _
  | up hole root frame tail read siblings outer ih =>
      exact .up hole root frame tail (heap_alloc_preserves_read heap frame.parent _ node read)
        (edge_hole_alloc heap hole frame.edges frame.value.index frame.value.children node siblings) ih

theorem edge_hole_repoint (heap : NodeHeap) (old fresh : NodeId) (edges : List StoredEdge) (index : Nat)
    (children : Forest) (byte : UInt8) (rep : EdgeHoleRep heap old edges index children)
    (access : edges[index]? = some (byte, old)) :
    EdgeHoleRep heap fresh (edges.set index (byte, fresh)) index children := by
  induction rep with
  | here storedByte tail previous rest remaining =>
      have same : storedByte = byte := (Prod.mk.inj (Option.some.inj access)).1
      subst storedByte
      exact .here byte tail previous rest remaining
  | there storedByte address tail child rest index focused remaining ih =>
      exact .there storedByte address (tail.set index (byte, fresh)) child rest index focused (ih access)

theorem edge_hole_parent_safe (heap : NodeHeap) (parent : NodeId) (pfx : Key) (terminal : Option Entry)
    (edges : List StoredEdge) (children : Forest) (index : Nat) (byte : UInt8) (hole : NodeId)
    (rep : NodeRep heap parent (.branch pfx terminal children))
    (read : heapRead heap parent = some (.branch pfx terminal edges)) (access : edges[index]? = some (byte, hole)) :
    EdgeHoleSafe heap parent edges index := by
  apply edge_hole_safe_of_indices heap parent edges index (byte, hole) access
  intro other edge otherRead _different
  exact represented_edge_no_return heap parent (.branch pfx terminal children) (.branch pfx terminal edges)
    edge.1 edge.2 rep read (List.mem_of_getElem? otherRead)

def repointedFrame (frame : HeapFrame) (byte : UInt8) (fresh : NodeId) : HeapFrame :=
  { frame with edges := frame.edges.set frame.value.index (byte, fresh) }

theorem heap_context_repoint (heap : NodeHeap) (old fresh root : NodeId) (frame : HeapFrame) (tail : List HeapFrame)
    (tree : Tree) (byte : UInt8) (context : HeapContext heap old root (frame :: tail))
    (focused : NodeRep heap old tree) (access : frame.edges[frame.value.index]? = some (byte, old))
    (safe : ContextSafe heap frame.parent tail) :
    let next := heapWrite heap frame.parent (some (.branch frame.value.pfx frame.value.terminal (repointedFrame frame byte fresh).edges))
    HeapContext next fresh root (repointedFrame frame byte fresh :: tail) := by
  cases context with
  | up _ _ _ _ read siblings outer =>
      have parentRep : NodeRep heap frame.parent (plugInsert frame.value tree) :=
        .branch frame.parent frame.value.pfx frame.value.terminal frame.edges _ read
          (edge_hole_plug heap old frame.edges frame.value.index frame.value.children siblings tree focused)
      have innerSafe := edge_hole_parent_safe heap frame.parent frame.value.pfx frame.value.terminal frame.edges
        (edgeSet frame.value.index tree frame.value.children) frame.value.index byte old parentRep read access
      apply HeapContext.up fresh root (repointedFrame frame byte fresh) tail
      · exact heap_write_here heap frame.parent _ (heap_read_bound heap frame.parent _ read)
      · exact edge_hole_repoint _ old fresh frame.edges frame.value.index frame.value.children byte
          (edge_hole_write_frame heap old frame.parent _ frame.edges frame.value.index frame.value.children siblings innerSafe) access
      · exact heap_context_write_frame heap frame.parent root frame.parent tail _ outer safe

theorem repointed_frames_values (frame : HeapFrame) (tail : List HeapFrame) (byte : UInt8) (fresh : NodeId) :
    ((repointedFrame frame byte fresh) :: tail).map HeapFrame.value = (frame :: tail).map HeapFrame.value := rfl

end Kv9.Radix
