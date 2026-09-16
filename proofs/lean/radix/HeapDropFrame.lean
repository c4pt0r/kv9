import HeapRewrite

set_option autoImplicit false

namespace Kv9.Radix

theorem heap_write_none_read_back (heap : NodeHeap) (focus address : NodeId) (node : StoredNode)
    (read : heapRead (heapWrite heap focus none) address = some node) : heapRead heap address = some node := by
  by_cases same : address = focus
  · subst address
    have bound : focus < heap.length := by
      have after := heap_read_bound (heapWrite heap focus none) focus node read
      simpa only [heapWrite, List.length_set] using after
    rw [heap_write_here heap focus none bound] at read
    contradiction
  · simpa only [heap_write_elsewhere heap focus address none same] using read

theorem release_external_read_back (heap next : NodeHeap) (focus : NodeId) (others remaining : List NodeId)
    (completed : releaseExternal heap focus others = some (next, remaining))
    (address : NodeId) (node : StoredNode) (read : heapRead next address = some node) : heapRead heap address = some node := by
  cases access : heapRead heap focus with
  | none => simp [releaseExternal, access] at completed
  | some stored =>
      by_cases unique : strongCount heap (focus :: others) focus = 1
      · have same : (heapWrite heap focus none, (storedChildren stored).map Prod.snd ++ others) = (next, remaining) := by
          simpa [releaseExternal, access, unique] using completed
        cases same
        exact heap_write_none_read_back heap focus address node read
      · have same : (heap, others) = (next, remaining) := by simpa [releaseExternal, access, unique] using completed
        cases same
        exact read

theorem drop_step_read_back (heap next : NodeHeap) (held pending work : List NodeId)
    (step : stepDrop heap held pending = .more next work) (address : NodeId) (node : StoredNode)
    (read : heapRead next address = some node) : heapRead heap address = some node := by
  obtain ⟨focus, others, remaining, _, _, release, _⟩ := drop_step_release heap next held pending work step
  exact release_external_read_back heap next focus others remaining release address node read

theorem drop_loop_read_back (heap next : NodeHeap) (held pending : List NodeId)
    (completed : dropLoop heap held pending = some next) (address : NodeId) (node : StoredNode)
    (read : heapRead next address = some node) : heapRead heap address = some node := by
  rw [dropLoop] at completed
  split at completed
  · contradiction
  · have same := Option.some.inj completed
    simpa only [same] using read
  · rename_i intermediate work transition
    have balance := drop_step_potential heap intermediate held pending work transition
    have previous := drop_loop_read_back intermediate next held work completed address node read
    exact drop_step_read_back heap intermediate held pending work transition address node previous
termination_by (heapTargets heap).length + pending.length
decreasing_by omega

mutual
  theorem node_rep_read_back (heap next : NodeHeap)
      (back : ∀ address node, heapRead next address = some node → heapRead heap address = some node)
      (address : NodeId) (tree : Tree) (rep : NodeRep next address tree) : NodeRep heap address tree := by
    cases rep with
    | leaf _ entry read => exact .leaf address entry (back address (.leaf entry) read)
    | branch _ pfx terminal edges children read descendants =>
        exact .branch address pfx terminal edges children (back address (.branch pfx terminal edges) read)
          (edges_rep_read_back heap next back edges children descendants)
  theorem edges_rep_read_back (heap next : NodeHeap)
      (back : ∀ address node, heapRead next address = some node → heapRead heap address = some node)
      (edges : List StoredEdge) (children : Forest) (rep : EdgesRep next edges children) : EdgesRep heap edges children := by
    cases rep with
    | nil => exact .nil
    | cons byte address tail child rest node remaining =>
        exact .cons byte address tail child rest (node_rep_read_back heap next back address child node)
          (edges_rep_read_back heap next back tail rest remaining)
end

theorem read_back_reachable_rep (heap next : NodeHeap)
    (back : ∀ address node, heapRead next address = some node → heapRead heap address = some node)
    (root : NodeId) (tree : Tree) (rep : NodeRep next root tree) (address : NodeId)
    (reachable : HeapReach heap root address) : ∃ child, NodeRep next address child := by
  induction reachable with
  | root => exact ⟨tree, rep⟩
  | child parent address node byte _ read edge ih =>
      obtain ⟨parentTree, parentRep⟩ := ih
      obtain ⟨stored, nextRead⟩ := node_rep_has_cell next parent parentTree parentRep
      have same : stored = node := Option.some.inj ((back parent stored nextRead).symm.trans read)
      subst stored
      obtain ⟨child, childRep, _⟩ := node_rep_child_work next parent parentTree node byte address parentRep nextRead edge
      exact ⟨child, childRep⟩

-- A borrowed descendant is kept by a live root, without adding an Arc token
-- that would change the source's get_mut/make_mut uniqueness decisions.
theorem drop_loop_keeps_borrowed (heap next : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) (completed : dropLoop heap held pending = some next)
    (root address : NodeId) (member : root ∈ held) (reachable : HeapReach heap root address)
    (tree : Tree) (rep : NodeRep heap address tree) : NodeRep next address tree := by
  obtain ⟨rootTree, rootRep⟩ := owned.rootsRepresented root (List.mem_append_left _ member)
  obtain ⟨actual, result, _, preserve⟩ := drop_loop_owned heap held pending owned
  have same := Option.some.inj (result.symm.trans completed)
  subst actual
  have nextRoot := preserve root rootTree member rootRep
  have back := drop_loop_read_back heap next held pending completed
  obtain ⟨child, childRep⟩ := read_back_reachable_rep heap next back root rootTree nextRoot address reachable
  have sameTree := node_rep_unique heap address child tree (node_rep_read_back heap next back address child childRep) rep
  simpa only [sameTree] using childRep

theorem drop_loop_keeps_borrowed_read (heap next : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) (completed : dropLoop heap held pending = some next)
    (root address : NodeId) (member : root ∈ held) (reachable : HeapReach heap root address)
    (node : StoredNode) (read : heapRead heap address = some node) : heapRead next address = some node := by
  obtain ⟨tree, rep⟩ := owned.modelled address node read
  have nextRep := drop_loop_keeps_borrowed heap next held pending owned completed root address member reachable tree rep
  obtain ⟨stored, nextRead⟩ := node_rep_has_cell next address tree nextRep
  have same := Option.some.inj ((drop_loop_read_back heap next held pending completed address stored nextRead).symm.trans read)
  simpa only [same] using nextRead

end Kv9.Radix
