import HeapDropLoop

set_option autoImplicit false

namespace Kv9.Radix

structure DropPool where
  heap : NodeHeap
  workers : List (List NodeId)

def poolReferences (held : List NodeId) (state : DropPool) : List NodeId := held ++ state.workers.flatten
def poolPotential (state : DropPool) : Nat := (heapTargets state.heap).length + state.workers.flatten.length

theorem worker_references_permute (held : List NodeId) (before after : List (List NodeId)) (pending : List NodeId) :
    (held ++ (before ++ pending :: after).flatten).Perm
      (((held ++ before.flatten) ++ after.flatten) ++ pending) := by
  simpa only [List.flatten_append, List.flatten_cons, List.append_assoc] using
    (List.Perm.append_left (held ++ before.flatten)
      (List.perm_append_comm : (pending ++ after.flatten).Perm (after.flatten ++ pending)))

-- Each constructor is one completed ownership-token release in one worker.
-- The standard library's linearizable no-Weak final-owner handoff is a contract
-- of this graph model, not a proof of native atomic instructions or scheduling.
inductive PoolRelease (held : List NodeId) : DropPool → DropPool → Prop where
  | worker (heap next : NodeHeap) (before after : List (List NodeId)) (pending work : List NodeId)
      (step : stepDrop heap ((held ++ before.flatten) ++ after.flatten) pending = .more next work) :
      PoolRelease held ⟨heap, before ++ pending :: after⟩ ⟨next, before ++ work :: after⟩

theorem pool_release_owned (held : List NodeId) (first next : DropPool)
    (step : PoolRelease held first next) (owned : HeapOwned first.heap (poolReferences held first)) :
    HeapOwned next.heap (poolReferences held next) ∧
      (∀ root tree, root ∈ held → NodeRep first.heap root tree → NodeRep next.heap root tree) := by
  cases step with
  | worker heap next before after pending work transition =>
      have input := worker_references_permute held before after pending
      have output := worker_references_permute held before after work
      have inputOwned := heap_owned_permute heap _ _ input owned
      obtain ⟨nextOwned, preserve⟩ := drop_step_owned heap next ((held ++ before.flatten) ++ after.flatten) pending work inputOwned transition
      refine ⟨heap_owned_permute next _ _ output.symm nextOwned, ?_⟩
      intro root tree member rep
      exact preserve root tree (List.mem_append_left _ (List.mem_append_left _ member)) rep

theorem pool_release_potential (held : List NodeId) (first next : DropPool) (step : PoolRelease held first next) :
    poolPotential next + 1 = poolPotential first := by
  cases step with
  | worker heap next before after pending work transition =>
      have balance := drop_step_potential heap next ((held ++ before.flatten) ++ after.flatten) pending work transition
      simp only [poolPotential, List.flatten_append, List.flatten_cons, List.length_append]
      omega

theorem pool_worker_progress (held : List NodeId) (heap : NodeHeap) (before after : List (List NodeId))
    (pending : List NodeId) (busy : pending ≠ [])
    (owned : HeapOwned heap (held ++ (before ++ pending :: after).flatten)) :
    ∃ next, PoolRelease held ⟨heap, before ++ pending :: after⟩ next := by
  have localOwned := heap_owned_permute heap _ _ (worker_references_permute held before after pending) owned
  cases transition : stepDrop heap ((held ++ before.flatten) ++ after.flatten) pending with
  | failed => exact False.elim (drop_step_no_failure heap _ pending localOwned transition)
  | done => exact False.elim (busy (drop_step_done heap _ pending transition))
  | more next work => exact ⟨⟨next, before ++ work :: after⟩, .worker heap next before after pending work transition⟩

inductive PoolHistory (held : List NodeId) : Nat → DropPool → DropPool → Prop where
  | nil (state : DropPool) : PoolHistory held 0 state state
  | step {count : Nat} {first middle last : DropPool}
      (advance : PoolRelease held first middle) (rest : PoolHistory held count middle last) :
      PoolHistory held (count + 1) first last

theorem pool_history_owned (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) (owned : HeapOwned first.heap (poolReferences held first)) :
    HeapOwned last.heap (poolReferences held last) ∧
      (∀ root tree, root ∈ held → NodeRep first.heap root tree → NodeRep last.heap root tree) := by
  induction run with
  | nil => exact ⟨owned, fun _ _ _ rep => rep⟩
  | step advance rest ih =>
      obtain ⟨middleOwned, preserve⟩ := pool_release_owned held _ _ advance owned
      obtain ⟨lastOwned, finalPreserve⟩ := ih middleOwned
      refine ⟨lastOwned, ?_⟩
      intro root tree member rep
      exact finalPreserve root tree member (preserve root tree member rep)

theorem pool_history_budget (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) : poolPotential last + count = poolPotential first := by
  induction run with
  | nil => simp
  | step advance rest ih =>
      have balance := pool_release_potential held _ _ advance
      omega

theorem pool_history_bounded (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) : count ≤ poolPotential first := by
  have balance := pool_history_budget held count first last run
  omega

theorem pool_finished_cells_reachable (held : List NodeId) (count : Nat) (first last : DropPool)
    (run : PoolHistory held count first last) (owned : HeapOwned first.heap (poolReferences held first))
    (finished : last.workers.flatten = []) (address : NodeId) (node : StoredNode)
    (read : heapRead last.heap address = some node) : ∃ root ∈ held, HeapReach last.heap root address := by
  have finalOwned := (pool_history_owned held count first last run owned).1
  simp only [poolReferences, finished, List.append_nil] at finalOwned
  exact owned_cell_reachable last.heap held finalOwned address node read

theorem pool_finished_no_held_no_cells (count : Nat) (first last : DropPool)
    (run : PoolHistory [] count first last) (owned : HeapOwned first.heap (poolReferences [] first))
    (finished : last.workers.flatten = []) (address : NodeId) : heapRead last.heap address = none := by
  have finalOwned := (pool_history_owned [] count first last run owned).1
  simp only [poolReferences, finished, List.append_nil] at finalOwned
  exact owned_no_roots_no_cells last.heap finalOwned address

end Kv9.Radix
