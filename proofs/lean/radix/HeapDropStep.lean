import HeapRootReach

set_option autoImplicit false

namespace Kv9.Radix

theorem strong_count_permute (heap : NodeHeap) (first second : List NodeId) (same : first.Perm second)
    (address : NodeId) : strongCount heap first address = strongCount heap second address := by
  simp only [strong_count_values, same.count_eq address]

theorem heap_owned_permute (heap : NodeHeap) (first second : List NodeId) (same : first.Perm second)
    (owned : HeapOwned heap first) : HeapOwned heap second := by
  refine ⟨owned.modelled, ?_, ?_⟩
  · intro root member
    exact owned.rootsRepresented root (same.mem_iff.mpr member)
  · intro address node read
    rw [← strong_count_permute heap first second same address]
    exact owned.live address node read

theorem last_reference_decompose (pending : List NodeId) (focus : NodeId)
    (pop : pending.getLast? = some focus) : pending = pending.dropLast ++ [focus] := by
  obtain ⟨rest, same⟩ := List.getLast?_eq_some_iff.mp pop
  simp only [same, List.dropLast_concat]

theorem popped_reference_permute (held pending : List NodeId) (focus : NodeId)
    (pop : pending.getLast? = some focus) :
    (held ++ pending).Perm (focus :: (held ++ pending.dropLast)) := by
  have split := last_reference_decompose pending focus pop
  have rearranged : held ++ pending = (held ++ pending.dropLast) ++ [focus] := by rw [List.append_assoc, ← split]
  rw [rearranged]
  exact List.perm_append_singleton focus _

inductive DropStep where
  | failed
  | done
  | more (heap : NodeHeap) (pending : List NodeId)

-- Physical Vec order: pop the last owned Arc, and append a last-owned branch's
-- child tokens at the end. Edge labels do not affect ownership accounting.
def stepDrop (heap : NodeHeap) (held pending : List NodeId) : DropStep :=
  match pending.getLast? with
  | none => .done
  | some focus => match heapRead heap focus with
    | none => .failed
    | some node =>
        if strongCount heap (held ++ pending) focus = 1 then
          .more (heapWrite heap focus none) (pending.dropLast ++ (storedChildren node).map Prod.snd)
        else .more heap pending.dropLast

theorem drop_step_release (heap next : NodeHeap) (held pending work : List NodeId)
    (step : stepDrop heap held pending = .more next work) :
    ∃ focus others remaining, (held ++ pending).Perm (focus :: others) ∧
      (∀ root ∈ held, root ∈ others) ∧ releaseExternal heap focus others = some (next, remaining) ∧
      remaining.Perm (held ++ work) := by
  cases pop : pending.getLast? with
  | none => simp [stepDrop, pop] at step
  | some focus =>
      have reorder := popped_reference_permute held pending focus pop
      have count := strong_count_permute heap (held ++ pending) (focus :: (held ++ pending.dropLast)) reorder focus
      cases read : heapRead heap focus with
      | none => simp [stepDrop, pop, read] at step
      | some node =>
          by_cases unique : strongCount heap (held ++ pending) focus = 1
          · have sourceUnique : strongCount heap (focus :: (held ++ pending.dropLast)) focus = 1 := count.symm.trans unique
            have same : (heapWrite heap focus none, pending.dropLast ++ (storedChildren node).map Prod.snd) = (next, work) := by
              simpa [stepDrop, pop, read, unique, Prod.mk.injEq] using step
            cases same
            refine ⟨focus, held ++ pending.dropLast, (storedChildren node).map Prod.snd ++ (held ++ pending.dropLast),
              reorder, ?_, ?_, ?_⟩
            · intro root member
              exact List.mem_append_left _ member
            · simp [releaseExternal, read, sourceUnique]
            · simpa only [List.append_assoc] using
                (List.perm_append_comm : ((storedChildren node).map Prod.snd ++ (held ++ pending.dropLast)).Perm
                  ((held ++ pending.dropLast) ++ (storedChildren node).map Prod.snd))
          · have sourceShared : strongCount heap (focus :: (held ++ pending.dropLast)) focus ≠ 1 := by
              rw [← count]
              exact unique
            have same : (heap, pending.dropLast) = (next, work) := by
              simpa [stepDrop, pop, read, unique, Prod.mk.injEq] using step
            cases same
            refine ⟨focus, held ++ pending.dropLast, held ++ pending.dropLast, reorder, ?_, ?_, .rfl⟩
            · intro root member
              exact List.mem_append_left _ member
            · simp [releaseExternal, read, sourceShared]

theorem drop_step_owned (heap next : NodeHeap) (held pending work : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) (step : stepDrop heap held pending = .more next work) :
    HeapOwned next (held ++ work) ∧
      (∀ root tree, root ∈ held → NodeRep heap root tree → NodeRep next root tree) := by
  obtain ⟨focus, others, remaining, before, included, release, after⟩ := drop_step_release heap next held pending work step
  have releaseOwned := release_external_owned heap next focus others remaining
    (heap_owned_permute heap (held ++ pending) (focus :: others) before owned) release
  refine ⟨heap_owned_permute next remaining (held ++ work) after releaseOwned, ?_⟩
  intro root tree member rep
  exact release_external_preserves_remaining heap next focus others remaining root tree (included root member) rep release

theorem drop_step_potential (heap next : NodeHeap) (held pending work : List NodeId)
    (step : stepDrop heap held pending = .more next work) :
    (heapTargets next).length + work.length + 1 = (heapTargets heap).length + pending.length := by
  obtain ⟨focus, others, remaining, before, _, release, after⟩ := drop_step_release heap next held pending work step
  have balance := release_external_potential heap next focus others remaining release
  have inputSize := before.length_eq
  have outputSize := after.length_eq
  simp only [List.length_append, List.length_cons] at inputSize outputSize balance
  omega

theorem drop_step_no_failure (heap : NodeHeap) (held pending : List NodeId)
    (owned : HeapOwned heap (held ++ pending)) : stepDrop heap held pending ≠ .failed := by
  cases pop : pending.getLast? with
  | none => simp [stepDrop, pop]
  | some focus =>
      obtain ⟨tree, rep⟩ := owned.rootsRepresented focus (List.mem_append_right _ (List.mem_of_getLast? pop))
      obtain ⟨node, read⟩ := node_rep_has_cell heap focus tree rep
      simp only [stepDrop, pop, read]
      split <;> simp

theorem drop_step_done (heap : NodeHeap) (held pending : List NodeId)
    (done : stepDrop heap held pending = .done) : pending = [] := by
  cases pop : pending.getLast? with
  | none => exact List.getLast?_eq_none_iff.mp pop
  | some focus =>
      cases read : heapRead heap focus with
      | none => simp [stepDrop, pop, read] at done
      | some node =>
          simp only [stepDrop, pop, read] at done
          split at done <;> contradiction

end Kv9.Radix
