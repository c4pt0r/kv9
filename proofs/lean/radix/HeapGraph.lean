import VectorRange

set_option autoImplicit false

namespace Kv9.Radix

-- Allocation identities are independent of physical addresses. Reclaimed
-- cells may be absent; allocation below uses a fresh identity. Connecting
-- identities and owned Vec/Box payloads to addressable storage is separate.
abbrev NodeId := Nat
abbrev StoredEdge := UInt8 × NodeId

inductive StoredNode where
  | leaf (entry : Entry)
  | branch (pfx : Key) (terminal : Option Entry) (children : List StoredEdge)
deriving DecidableEq, Repr

abbrev NodeHeap := List (Option StoredNode)

def heapRead (heap : NodeHeap) (address : NodeId) : Option StoredNode := (heap[address]?).join

def heapAlloc (heap : NodeHeap) (node : StoredNode) : NodeHeap := heap ++ [some node]

def heapWrite (heap : NodeHeap) (address : NodeId) (node : Option StoredNode) : NodeHeap := heap.set address node

def storedChildren : StoredNode → List StoredEdge
  | .leaf _ => []
  | .branch _ _ children => children

-- Stored nodes contain child identities, never recursively embedded Trees.
-- Finite representation derivations unfold those exact references.
mutual
  inductive NodeRep (heap : NodeHeap) : NodeId → Tree → Prop where
    | leaf (address : NodeId) (entry : Entry)
        (read : heapRead heap address = some (.leaf entry)) : NodeRep heap address (.leaf entry)
    | branch (address : NodeId) (pfx : Key) (terminal : Option Entry) (edges : List StoredEdge) (children : Forest)
        (read : heapRead heap address = some (.branch pfx terminal edges))
        (descendants : EdgesRep heap edges children) : NodeRep heap address (.branch pfx terminal children)
  inductive EdgesRep (heap : NodeHeap) : List StoredEdge → Forest → Prop where
    | nil : EdgesRep heap [] .nil
    | cons (byte : UInt8) (address : NodeId) (edges : List StoredEdge) (child : Tree) (tail : Forest)
        (node : NodeRep heap address child) (rest : EdgesRep heap edges tail) :
        EdgesRep heap ((byte, address) :: edges) (.cons byte child tail)
end

inductive HeapReach (heap : NodeHeap) (root : NodeId) : NodeId → Prop where
  | root : HeapReach heap root root
  | child (parent address : NodeId) (node : StoredNode) (byte : UInt8)
      (previous : HeapReach heap root parent) (read : heapRead heap parent = some node)
      (edge : (byte, address) ∈ storedChildren node) : HeapReach heap root address

theorem heap_read_bound (heap : NodeHeap) (address : NodeId) (node : StoredNode)
    (read : heapRead heap address = some node) : address < heap.length := by
  by_cases bound : address < heap.length
  · exact bound
  · have absent : heap[address]? = none := List.getElem?_eq_none (Nat.le_of_not_gt bound)
    simp [heapRead, absent] at read

theorem heap_alloc_fresh (heap : NodeHeap) (node : StoredNode) :
    heapRead (heapAlloc heap node) heap.length = some node := by
  simp [heapRead, heapAlloc]

theorem heap_alloc_preserves_read (heap : NodeHeap) (address : NodeId) (old fresh : StoredNode)
    (read : heapRead heap address = some old) : heapRead (heapAlloc heap fresh) address = some old := by
  have bound := heap_read_bound heap address old read
  simpa only [heapRead, heapAlloc, List.getElem?_append_left bound] using read

theorem heap_write_here (heap : NodeHeap) (address : NodeId) (node : Option StoredNode)
    (bound : address < heap.length) : heapRead (heapWrite heap address node) address = node := by
  simp [heapRead, heapWrite, bound]

theorem heap_write_elsewhere (heap : NodeHeap) (address other : NodeId) (node : Option StoredNode)
    (different : other ≠ address) : heapRead (heapWrite heap address node) other = heapRead heap other := by
  simp [heapRead, heapWrite, Ne.symm different]

theorem node_rep_has_cell (heap : NodeHeap) (address : NodeId) (tree : Tree) (rep : NodeRep heap address tree) :
    ∃ node, heapRead heap address = some node := by
  cases rep with
  | leaf _ entry read => exact ⟨.leaf entry, read⟩
  | branch _ pfx terminal edges _ read _ => exact ⟨.branch pfx terminal edges, read⟩

theorem edges_rep_member (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (rep : EdgesRep heap edges children) (edge : StoredEdge) (member : edge ∈ edges) :
    ∃ tree, NodeRep heap edge.2 tree := by
  cases rep with
  | nil => simp at member
  | cons byte address tail child rest node remaining =>
      rcases List.mem_cons.mp member with same | later
      · subst edge
        exact ⟨child, node⟩
      · exact edges_rep_member heap tail rest remaining edge later

theorem node_rep_reachable (heap : NodeHeap) (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree)
    (address : NodeId) (reachable : HeapReach heap root address) : ∃ child, NodeRep heap address child := by
  induction reachable with
  | root => exact ⟨tree, rep⟩
  | child parent address node byte _ read edge ih =>
      obtain ⟨parentTree, parentRep⟩ := ih
      cases parentRep with
      | leaf _ entry readLeaf =>
          have same : node = .leaf entry := Option.some.inj (read.symm.trans readLeaf)
          simp [same, storedChildren] at edge
      | branch _ pfx terminal edges children readBranch descendants =>
          have same : node = .branch pfx terminal edges := Option.some.inj (read.symm.trans readBranch)
          exact edges_rep_member heap edges children descendants (byte, address)
            (by simpa only [same, storedChildren] using edge)

mutual
  theorem node_rep_unique (heap : NodeHeap) (address : NodeId) (a b : Tree)
      (left : NodeRep heap address a) (right : NodeRep heap address b) : a = b := by
    cases left with
    | leaf _ entry read =>
        cases right with
        | leaf _ other readOther =>
            have same : entry = other := by simpa only [read, Option.some.injEq, StoredNode.leaf.injEq] using readOther
            rw [same]
        | branch _ pfx terminal edges children readOther _ => simp [read] at readOther
    | branch _ pfx terminal edges children read descendants =>
        cases right with
        | leaf _ other readOther => simp [read] at readOther
        | branch _ pfxOther terminalOther edgesOther childrenOther readOther descendantsOther =>
            have same : pfx = pfxOther ∧ terminal = terminalOther ∧ edges = edgesOther := by
              simpa only [read, Option.some.injEq, StoredNode.branch.injEq] using readOther
            rcases same with ⟨rfl, rfl, rfl⟩
            rw [edges_rep_unique heap edges children childrenOther descendants descendantsOther]
  theorem edges_rep_unique (heap : NodeHeap) (edges : List StoredEdge) (a b : Forest)
      (left : EdgesRep heap edges a) (right : EdgesRep heap edges b) : a = b := by
    cases left with
    | nil => cases right; rfl
    | cons byte address tail child rest node remaining =>
        cases right with
        | cons _ _ _ other otherRest otherNode otherRemaining =>
            rw [node_rep_unique heap address child other node otherNode,
              edges_rep_unique heap tail rest otherRest remaining otherRemaining]
end

-- Transport requires agreement and closure on the actual reachable identity
-- set. It does not assume that an unchanged root implies an unchanged graph.
mutual
  theorem node_rep_transport (heap next : NodeHeap) (keptSet : NodeId → Prop)
      (agree : ∀ address, keptSet address → heapRead next address = heapRead heap address)
      (closed : ∀ parent node byte child, keptSet parent → heapRead heap parent = some node →
        (byte, child) ∈ storedChildren node → keptSet child)
      (address : NodeId) (tree : Tree) (rep : NodeRep heap address tree) (kept : keptSet address) :
      NodeRep next address tree := by
    cases rep with
    | leaf _ entry read => exact .leaf address entry ((agree address kept).trans read)
    | branch _ pfx terminal edges children read descendants =>
        refine .branch address pfx terminal edges children ((agree address kept).trans read) ?_
        apply edges_rep_transport heap next keptSet agree closed edges children descendants
        intro edge member
        exact closed address (.branch pfx terminal edges) edge.1 edge.2 kept read member
  theorem edges_rep_transport (heap next : NodeHeap) (keptSet : NodeId → Prop)
      (agree : ∀ address, keptSet address → heapRead next address = heapRead heap address)
      (closed : ∀ parent node byte child, keptSet parent → heapRead heap parent = some node →
        (byte, child) ∈ storedChildren node → keptSet child)
      (edges : List StoredEdge) (children : Forest) (rep : EdgesRep heap edges children)
      (kept : ∀ edge ∈ edges, keptSet edge.2) : EdgesRep next edges children := by
    cases rep with
    | nil => exact .nil
    | cons byte address tail child rest node remaining =>
        exact .cons byte address tail child rest
          (node_rep_transport heap next keptSet agree closed address child node (kept (byte, address) (by simp)))
          (edges_rep_transport heap next keptSet agree closed tail rest remaining
            (fun edge member => kept edge (by simp [member])))
end

theorem node_rep_write_frame (heap : NodeHeap) (root changed : NodeId) (tree : Tree) (node : Option StoredNode)
    (rep : NodeRep heap root tree) (separate : ¬ HeapReach heap root changed) :
    NodeRep (heapWrite heap changed node) root tree := by
  apply node_rep_transport heap (heapWrite heap changed node) (HeapReach heap root)
    (fun address reachable => heap_write_elsewhere heap changed address node
      (fun same => separate (same ▸ reachable)))
    (fun parent stored byte child previous read edge => .child parent child stored byte previous read edge)
    root tree rep .root

theorem node_rep_alloc (heap : NodeHeap) (root : NodeId) (tree : Tree) (fresh : StoredNode)
    (rep : NodeRep heap root tree) : NodeRep (heapAlloc heap fresh) root tree := by
  apply node_rep_transport heap (heapAlloc heap fresh) (HeapReach heap root)
  · intro address reachable
    obtain ⟨child, childRep⟩ := node_rep_reachable heap root tree rep address reachable
    obtain ⟨node, read⟩ := node_rep_has_cell heap address child childRep
    rw [read]
    exact heap_alloc_preserves_read heap address node fresh read
  · exact fun parent stored byte child previous read edge => .child parent child stored byte previous read edge
  · exact rep
  · exact .root

theorem node_rep_fresh_separate (heap : NodeHeap) (root : NodeId) (tree : Tree) (rep : NodeRep heap root tree) :
    ¬ HeapReach heap root heap.length := by
  intro reachable
  obtain ⟨child, childRep⟩ := node_rep_reachable heap root tree rep heap.length reachable
  obtain ⟨node, read⟩ := node_rep_has_cell heap heap.length child childRep
  have bound := heap_read_bound heap heap.length node read
  exact Nat.lt_irrefl heap.length bound

end Kv9.Radix
