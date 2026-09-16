import HeapDropStep

set_option autoImplicit false

namespace Kv9.Radix

-- The transition models one iteration of the Vec pop/append loop. The recursive
-- mathematical evaluator follows those iterations, not recursive tree teardown.
def dropLoop (heap : NodeHeap) (held pending : List NodeId) : Option NodeHeap :=
  match _transition : stepDrop heap held pending with
  | .failed => none
  | .done => some heap
  | .more next work => dropLoop next held work
termination_by (heapTargets heap).length + pending.length
decreasing_by
  have balance := drop_step_potential heap next held pending work _transition
  omega

theorem drop_loop_owned (heap : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) :
    ∃ next, dropLoop heap held pending = some next ∧ HeapOwned next held ∧
      (∀ root tree, root ∈ held → NodeRep heap root tree → NodeRep next root tree) := by
  rw [dropLoop]
  split
  · rename_i transition
    exact False.elim (drop_step_no_failure heap held pending owned transition)
  · rename_i transition
    have empty := drop_step_done heap held pending transition
    exact ⟨heap, rfl, by simpa only [empty, List.append_nil] using owned, fun _ _ _ rep => rep⟩
  · rename_i next work transition
    obtain ⟨nextOwned, preserve⟩ := drop_step_owned heap next held pending work owned transition
    have balance := drop_step_potential heap next held pending work transition
    obtain ⟨finalHeap, result, finalOwned, finalPreserve⟩ := drop_loop_owned next held work nextOwned
    refine ⟨finalHeap, result, finalOwned, ?_⟩
    intro root tree member rep
    exact finalPreserve root tree member (preserve root tree member rep)
termination_by (heapTargets heap).length + pending.length
decreasing_by omega

theorem drop_loop_no_failure (heap : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) : dropLoop heap held pending ≠ none := by
  obtain ⟨next, completed, _⟩ := drop_loop_owned heap held pending owned
  rw [completed]
  simp

theorem drop_loop_reclaims_unreachable (heap next : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) (completed : dropLoop heap held pending = some next)
    (address : NodeId) (node : StoredNode) (read : heapRead next address = some node) :
    ∃ root ∈ held, HeapReach next root address := by
  obtain ⟨actual, result, finalOwned, _⟩ := drop_loop_owned heap held pending owned
  have same := Option.some.inj (result.symm.trans completed)
  subst actual
  exact owned_cell_reachable next held finalOwned address node read

theorem drop_loop_no_held_reclaims_all (heap : NodeHeap) (pending : List NodeId)
    (owned : HeapOwned heap pending) :
    ∃ next, dropLoop heap [] pending = some next ∧ ∀ address, heapRead next address = none := by
  obtain ⟨next, result, finalOwned, _⟩ := drop_loop_owned heap [] pending owned
  exact ⟨next, result, owned_no_roots_no_cells next finalOwned⟩

end Kv9.Radix
