import KeyOrder

set_option autoImplicit false

namespace Kv9.Radix

-- Top-first lists model the reverse of Rust's pending Vec: pop() reads the
-- head here. Node expansion preserves Rust's child/terminal push ordering.
inductive CursorTask where
  | node (tree : Tree)
  | entry (row : Entry)

def forestTasks : Forest → List CursorTask
  | .nil => []
  | .cons _ child tail => .node child :: forestTasks tail

def terminalTasks (terminal : Option Entry) : List CursorTask := terminal.toList.map CursorTask.entry

def branchTasks (reverse : Bool) (terminal : Option Entry) (children : Forest) : List CursorTask :=
  if reverse then (forestTasks children).reverse ++ terminalTasks terminal
  else terminalTasks terminal ++ forestTasks children

def taskRows (reverse : Bool) : CursorTask → List Entry
  | .node tree => orient reverse (entries tree)
  | .entry row => [row]

def pendingRows (reverse : Bool) : List CursorTask → List Entry
  | [] => []
  | task :: tail => taskRows reverse task ++ pendingRows reverse tail

theorem pending_rows_append (reverse : Bool) (a b : List CursorTask) :
    pendingRows reverse (a ++ b) = pendingRows reverse a ++ pendingRows reverse b := by
  induction a with
  | nil => rfl
  | cons task tail ih => simp [pendingRows, ih, List.append_assoc]

theorem task_rows_reverse (task : CursorTask) : taskRows true task = (taskRows false task).reverse := by
  cases task <;> simp [taskRows, orient]

theorem pending_rows_reverse (tasks : List CursorTask) :
    pendingRows true tasks.reverse = (pendingRows false tasks).reverse := by
  induction tasks with
  | nil => rfl
  | cons task tail ih =>
      simp [pending_rows_append, pendingRows, ih, task_rows_reverse]

theorem forest_tasks_rows (forest : Forest) : pendingRows false (forestTasks forest) = forestEntries forest := by
  cases forest with
  | nil => rfl
  | cons byte child tail => simp [forestTasks, pendingRows, taskRows, orient, forestEntries, forest_tasks_rows tail]

theorem terminal_tasks_rows (reverse : Bool) (terminal : Option Entry) :
    pendingRows reverse (terminalTasks terminal) = terminal.toList := by
  cases terminal <;> rfl

theorem branch_tasks_rows (reverse : Bool) (pfx : Key) (terminal : Option Entry) (children : Forest) :
    pendingRows reverse (branchTasks reverse terminal children) = orient reverse (entries (.branch pfx terminal children)) := by
  cases reverse with
  | false => simp [branchTasks, pending_rows_append, terminal_tasks_rows, forest_tasks_rows, orient, entries]
  | true =>
      simp only [branchTasks, if_true, pending_rows_append,
        pending_rows_reverse, forest_tasks_rows, terminal_tasks_rows, orient, entries, List.reverse_append]
      cases terminal <;> rfl

mutual
  def treeWork : Tree → Nat
    | .leaf _ => 1
    | .branch _ terminal children => 1 + terminal.toList.length + forestWork children
  def forestWork : Forest → Nat
    | .nil => 0
    | .cons _ child tail => treeWork child + forestWork tail
end

def taskWork : CursorTask → Nat
  | .node tree => treeWork tree
  | .entry _ => 1

def pendingWork : List CursorTask → Nat
  | [] => 0
  | task :: tail => taskWork task + pendingWork tail

theorem pending_work_append (a b : List CursorTask) :
    pendingWork (a ++ b) = pendingWork a + pendingWork b := by
  induction a with
  | nil => simp [pendingWork]
  | cons task tail ih => simp [pendingWork, ih, Nat.add_assoc]

theorem pending_work_reverse (tasks : List CursorTask) : pendingWork tasks.reverse = pendingWork tasks := by
  induction tasks with
  | nil => rfl
  | cons task tail ih => simp [pending_work_append, pendingWork, ih, Nat.add_comm]

theorem forest_tasks_work (forest : Forest) : pendingWork (forestTasks forest) = forestWork forest := by
  cases forest with
  | nil => rfl
  | cons byte child tail => simp [forestTasks, pendingWork, taskWork, forestWork, forest_tasks_work tail]

theorem terminal_tasks_work (terminal : Option Entry) : pendingWork (terminalTasks terminal) = terminal.toList.length := by
  cases terminal <;> rfl

theorem branch_tasks_work (reverse : Bool) (terminal : Option Entry) (children : Forest) :
    pendingWork (branchTasks reverse terminal children) = terminal.toList.length + forestWork children := by
  cases reverse <;>
    simp [branchTasks, pending_work_append, pending_work_reverse, terminal_tasks_work, forest_tasks_work, Nat.add_comm]

-- Well-founded recursion models the actual while loop; the measure counts
-- pending node expansions and entries, not an arbitrary caller-supplied fuel.
def nextTask (reverse : Bool) (pending : List CursorTask) : Option (Entry × List CursorTask) :=
  match pending with
  | [] => none
  | .entry entry :: tail => some (entry, tail)
  | .node (.leaf entry) :: tail => some (entry, tail)
  | .node (.branch _ terminal children) :: tail => nextTask reverse (branchTasks reverse terminal children ++ tail)
termination_by pendingWork pending
decreasing_by
  simp only [pendingWork, taskWork, treeWork, pending_work_append, branch_tasks_work]
  omega

theorem next_task_refines (reverse : Bool) (pending : List CursorTask) :
    match nextTask reverse pending with
    | none => pendingRows reverse pending = []
    | some (entry, tail) => pendingRows reverse pending = entry :: pendingRows reverse tail := by
  cases pending with
  | nil => simp [nextTask, pendingRows]
  | cons task tail =>
      cases task with
      | entry entry => simp [nextTask, pendingRows, taskRows]
      | node tree =>
          cases tree with
          | leaf entry => cases reverse <;> simp [nextTask, pendingRows, taskRows, entries, orient]
          | branch pfx terminal children =>
              rw [nextTask]
              have rest := next_task_refines reverse (branchTasks reverse terminal children ++ tail)
              simpa only [pending_rows_append, branch_tasks_rows reverse pfx terminal children,
                pendingRows, taskRows] using rest
termination_by pendingWork pending
decreasing_by
  simp only [pendingWork, taskWork, treeWork, pending_work_append, branch_tasks_work]
  omega

theorem next_task_empty (reverse : Bool) (pending : List CursorTask) (stopped : nextTask reverse pending = none) :
    pendingRows reverse pending = [] := by
  simpa only [stopped] using next_task_refines reverse pending

theorem next_task_yield (reverse : Bool) (pending tail : List CursorTask) (entry : Entry)
    (yielded : nextTask reverse pending = some (entry, tail)) :
    pendingRows reverse pending = entry :: pendingRows reverse tail := by
  simpa only [yielded] using next_task_refines reverse pending

end Kv9.Radix
