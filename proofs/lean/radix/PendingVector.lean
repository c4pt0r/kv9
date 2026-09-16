import InsertWordLoop

set_option autoImplicit false

namespace Kv9.Radix

-- Lists here retain Rust Vec order: append pushes at the end and pop removes
-- the last item. The earlier cursor model instead keeps its top at the head.
def vectorPop (pending : List CursorTask) : Option (CursorTask × List CursorTask) :=
  pending.getLast?.map (fun task => (task, pending.dropLast))

def branchPush (reverse : Bool) (terminal : Option Entry) (children : Forest) : List CursorTask :=
  if reverse then terminalTasks terminal ++ forestTasks children
  else (forestTasks children).reverse ++ terminalTasks terminal

theorem branch_push_reverse (reverse : Bool) (terminal : Option Entry) (children : Forest) :
    (branchPush reverse terminal children).reverse = branchTasks reverse terminal children := by
  cases reverse <;> cases terminal <;> simp [branchPush, branchTasks, terminalTasks]

theorem vector_pop_refines (pending : List CursorTask) :
    (vectorPop pending).map (fun pair => (pair.1, pair.2.reverse)) =
      match pending.reverse with
      | [] => none
      | task :: tail => some (task, tail) := by
  unfold vectorPop
  rw [Option.map_map]
  change pending.getLast?.map (fun task => (task, pending.dropLast.reverse)) = _
  rw [← List.head?_reverse, ← List.tail_reverse]
  cases pending.reverse <;> rfl

theorem vector_pop_none (pending : List CursorTask) (empty : vectorPop pending = none) : pending.reverse = [] := by
  have relation := vector_pop_refines pending
  rw [empty] at relation
  cases h : pending.reverse with
  | nil => rfl
  | cons task tail => simp [h] at relation

theorem vector_pop_some (pending rest : List CursorTask) (task : CursorTask)
    (popped : vectorPop pending = some (task, rest)) : pending.reverse = task :: rest.reverse := by
  have relation := vector_pop_refines pending
  rw [popped] at relation
  cases h : pending.reverse with
  | nil => simp [h] at relation
  | cons head tail =>
      have same : task = head ∧ rest.reverse = tail := by simpa only [h, Option.map_some, Option.some.injEq, Prod.mk.injEq] using relation
      rw [same.1, same.2]

theorem vector_expansion_decreases (pending rest : List CursorTask) (reverse : Bool)
    (pfx : Key) (terminal : Option Entry) (children : Forest)
    (popped : vectorPop pending = some (.node (.branch pfx terminal children), rest)) :
    pendingWork (rest ++ branchPush reverse terminal children).reverse < pendingWork pending.reverse := by
  rw [vector_pop_some pending rest _ popped, List.reverse_append, branch_push_reverse]
  simp only [pending_work_append, branch_tasks_work, pendingWork, taskWork, treeWork]
  omega

def nextVector (reverse : Bool) (pending : List CursorTask) : Option (Entry × List CursorTask) :=
  match _popped : vectorPop pending with
  | none => none
  | some (.entry entry, rest) => some (entry, rest)
  | some (.node (.leaf entry), rest) => some (entry, rest)
  | some (.node (.branch _pfx terminal children), rest) =>
      nextVector reverse (rest ++ branchPush reverse terminal children)
termination_by pendingWork pending.reverse
decreasing_by
  exact vector_expansion_decreases pending rest reverse _pfx terminal children _popped

theorem next_vector_refines (reverse : Bool) (pending : List CursorTask) :
    (nextVector reverse pending).map (fun pair => (pair.1, pair.2.reverse)) = nextTask reverse pending.reverse := by
  rw [nextVector]
  split
  · rename_i popped
    rw [vector_pop_none pending popped, nextTask]
    rfl
  · rename_i entry rest popped
    rw [vector_pop_some pending rest _ popped, nextTask]
    rfl
  · rename_i entry rest popped
    rw [vector_pop_some pending rest _ popped, nextTask]
    rfl
  · rename_i pfx terminal children rest popped
    have smaller := vector_expansion_decreases pending rest reverse pfx terminal children popped
    rw [next_vector_refines reverse (rest ++ branchPush reverse terminal children)]
    rw [vector_pop_some pending rest _ popped, nextTask]
    simp only [List.reverse_append, branch_push_reverse]
termination_by pendingWork pending.reverse
decreasing_by
  assumption

theorem next_vector_yield (reverse : Bool) (pending rest : List CursorTask) (entry : Entry)
    (yielded : nextVector reverse pending = some (entry, rest)) :
    pendingRows reverse pending.reverse = entry :: pendingRows reverse rest.reverse := by
  have relation := next_vector_refines reverse pending
  rw [yielded] at relation
  exact next_task_yield reverse pending.reverse rest.reverse entry relation.symm

theorem next_vector_empty (reverse : Bool) (pending : List CursorTask) (empty : nextVector reverse pending = none) :
    pendingRows reverse pending.reverse = [] := by
  have relation := next_vector_refines reverse pending
  rw [empty] at relation
  exact next_task_empty reverse pending.reverse relation.symm

end Kv9.Radix
