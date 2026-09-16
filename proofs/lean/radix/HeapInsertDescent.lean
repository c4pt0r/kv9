import HeapMutationInvariant

set_option autoImplicit false

namespace Kv9.Radix

theorem edges_rep_get_address (heap : NodeHeap) (edges : List StoredEdge) (children : Forest)
    (index : Nat) (byte : UInt8) (child : Tree) (rep : EdgesRep heap edges children)
    (access : edgeGet index children = some (byte, child)) :
    ∃ address, edges[index]? = some (byte, address) ∧ NodeRep heap address child := by
  induction index generalizing edges children with
  | zero =>
      cases rep with
      | nil => simp [edgeGet] at access
      | cons storedByte address tail tree rest node _ =>
          have same : storedByte = byte ∧ tree = child := by simpa only [edgeGet, Option.some.injEq, Prod.mk.injEq] using access
          rcases same with ⟨rfl, rfl⟩
          exact ⟨address, rfl, node⟩
  | succ index ih =>
      cases rep with
      | nil => simp [edgeGet] at access
      | cons _ _ tail _ rest _ remaining => exact ih tail rest remaining access

-- branch_mut first establishes uniqueness at the existing borrowed slot.
-- Taking the indexed child borrow changes no heap cell and owns no Arc token.
def descendPlace (state : MutHeap) (others : List NodeId) (index : Nat) : Option MutHeap := do
  let (next, focus) ← makePlaceUnique state others
  match heapRead next.heap focus with
  | some (.branch _ _ edges) =>
      match edges[index]? with
      | some _ => some { next with place := .edge focus index }
      | none => none
  | _ => none

theorem mutation_invariant_descend (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (index : Nat) (byte : UInt8) (address : NodeId) (edges : List StoredEdge) (child : Tree)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (one : strongCount state.heap (state.root :: others) focus = 1)
    (separate : HeapSeparated state.heap others focus)
    (read : heapRead state.heap focus = some (.branch pfx terminal edges))
    (access : edges[index]? = some (byte, address)) (pureAccess : edgeGet index children = some (byte, child)) :
    MutationInvariant { state with place := .edge focus index } others address child
      (⟨focus, edges, ⟨pfx, terminal, children, index⟩⟩ :: frames) := by
  have descendants := node_rep_branch_edges state.heap focus pfx terminal edges children inv.focused read
  obtain ⟨actual, actualAccess, focused⟩ := edges_rep_get state.heap edges children index byte address descendants access
  have same : actual = child := (Prod.mk.inj (Option.some.inj (actualAccess.symm.trans pureAccess))).2
  subst actual
  refine ⟨inv.owned, .up address state.root _ frames read
    (edge_hole_from_edges state.heap edges children index byte address descendants access) inv.context, ?_, ?_, ⟨rfl, rfl⟩, focused⟩
  · intro frame member
    rcases List.mem_cons.mp member with here | later
    · subst frame
      exact one
    · exact inv.privateParents frame later
  · intro frame member
    rcases List.mem_cons.mp member with here | later
    · subst frame
      exact separate
    · exact inv.savedParents frame later

theorem descend_place_refines (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (index : Nat) (byte : UInt8) (child : Tree)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (access : edgeGet index children = some (byte, child)) :
    ∃ next address nextFrames, descendPlace state others index = some next ∧
      MutationInvariant next others address child nextFrames ∧
      nextFrames.map HeapFrame.value = ⟨pfx, terminal, children, index⟩ :: frames.map HeapFrame.value ∧
      (∀ root old, root ∈ others → NodeRep state.heap root old → NodeRep next.heap root old) := by
  obtain ⟨unique, parent, uniqueFrames, completed, uniqueInv, one, separate, values, preserve⟩ :=
    make_place_unique_invariant state others focus (.branch pfx terminal children) frames inv
  have stored : ∃ edges, heapRead unique.heap parent = some (.branch pfx terminal edges) ∧ EdgesRep unique.heap edges children := by
    cases uniqueInv.focused with
    | branch _ _ _ edges _ read descendants => exact ⟨edges, read, descendants⟩
  obtain ⟨edges, read, descendants⟩ := stored
  obtain ⟨address, found, _⟩ := edges_rep_get_address unique.heap edges children index byte child descendants access
  refine ⟨{ unique with place := .edge parent index }, address,
    ⟨parent, edges, ⟨pfx, terminal, children, index⟩⟩ :: uniqueFrames, ?_,
    mutation_invariant_descend unique others parent pfx terminal children uniqueFrames index byte address edges child
      uniqueInv one separate read found access, ?_, preserve⟩
  · simp [descendPlace, completed, read, found]
  · simp only [List.map_cons, values]

-- The source executes this action only after selection returned its index.
-- Depth arithmetic is handled by the existing word-step proof; the mutable
-- graph step has exactly the same abstract frame and focused child.
theorem descend_place_execute_correspondence (state : MutHeap) (others : List NodeId) (focus : NodeId)
    (pfx : Key) (terminal : Option Entry) (children : Forest) (frames : List HeapFrame)
    (depth nextDepth index : Nat) (fresh : Entry) (byte : UInt8) (child : Tree)
    (inv : MutationInvariant state others focus (.branch pfx terminal children) frames)
    (access : edgeGet index children = some (byte, child)) :
    executeInsert depth fresh (.branch pfx terminal children) (.descend index nextDepth) =
      .down ⟨pfx, terminal, children, index⟩ child nextDepth ∧
    ∃ next address nextFrames, descendPlace state others index = some next ∧
      MutationInvariant next others address child nextFrames ∧
      nextFrames.map HeapFrame.value = ⟨pfx, terminal, children, index⟩ :: frames.map HeapFrame.value ∧
      (∀ root old, root ∈ others → NodeRep state.heap root old → NodeRep next.heap root old) :=
  ⟨by simp [executeInsert, access], descend_place_refines state others focus pfx terminal children frames index byte child inv access⟩

end Kv9.Radix
