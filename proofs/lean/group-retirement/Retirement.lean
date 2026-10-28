import Std

namespace Kv9.GroupRetirement

/-- One immutable group ID. `opened` includes all driver/peer/engine owners;
external counts previously issued handles, not just currently executing calls.
The model assumes exact committed retirement authority at `stop`. -/
structure State where
  registered : Bool := true
  running : Bool := false
  stopped : Bool := false
  fenced : Bool := false
  opened : Bool := true
  locked : Bool := true
  payload : Bool := true
  external : Nat := 0
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | dispatch {s} (registered : s.registered = true) (live : s.stopped = false) :
      Step s {s with running := true}
  | finish {s} : Step s {s with running := false}
  | borrow {s} (opened : s.opened = true) : Step s {s with external := s.external + 1}
  | release {s} (n : Nat) (held : s.external = n + 1) : Step s {s with external := n}
  | stop {s} : Step s {s with stopped := true}
  | publish {s} (stopped : s.stopped = true) : Step s {s with fenced := true}
  | unregister {s} (stopped : s.stopped = true) (idle : s.running = false) :
      Step s {s with registered := false}
  | close {s} (detached : s.registered = false) (idle : s.running = false)
      (unborrowed : s.external = 0) (durable : s.fenced = true) :
      Step s {s with opened := false}
  | unlock {s} (closed : s.opened = false) : Step s {s with locked := false}
  | delete {s} (durable : s.fenced = true) (closed : s.opened = false) :
      Step s {s with payload := false}
  | crash {s} : Step s {s with registered := false, running := false, stopped := true, opened := false, locked := false, external := 0}
  | recover {s} (notRetired : s.fenced = false) (present : s.payload = true)
      (closed : s.opened = false) : Step s {s with registered := true, running := false, stopped := false, opened := true, locked := true}
  | idle {s} : Step s s

def Safe (s : State) : Prop :=
  (s.running = true → s.registered = true) ∧
  (s.registered = true → s.opened = true) ∧
  (s.external > 0 → s.opened = true) ∧
  (s.opened = true → s.locked = true ∧ s.payload = true) ∧
  (s.payload = false → s.fenced = true) ∧
  (s.fenced = true → s.stopped = true)

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  cases step with
  | release n held =>
    have oldPositive : s.external > 0 := by omega
    simp_all [Safe]
  | _ => simp_all [Safe]

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem running_slot_is_not_free {s : State} (run : Reachable s)
    (running : s.running = true) : s.registered = true := (run_safe run).1 running

theorem borrowed_payload_stays_open {s : State} (run : Reachable s)
    (held : s.external > 0) : s.opened = true := (run_safe run).2.2.1 held

theorem deletion_requires_fence_and_closed_owners {s : State} (run : Reachable s)
    (deleted : s.payload = false) : s.fenced = true ∧ s.opened = false := by
  have safe := run_safe run
  cases h : s.opened <;> simp_all [Safe]

theorem lock_retained_while_open {s : State} (run : Reachable s)
    (opened : s.opened = true) : s.locked = true := ((run_safe run).2.2.2.1 opened).1

theorem fence_survives_every_transition {s t : State} (step : Step s t)
    (fenced : s.fenced = true) : t.fenced = true := by
  cases step <;> simp_all

theorem retired_group_cannot_dispatch {s : State} (run : Reachable s)
    (fenced : s.fenced = true) : s.stopped = true := (run_safe run).2.2.2.2.2 fenced

/-- Conditional progress witness after workers and issued handles drain.
It proves an enabled finite continuation, not fairness of Rust/OS scheduling. -/
theorem drained_group_can_close {s : State}
    (detached : s.registered = false) (idle : s.running = false)
    (unborrowed : s.external = 0) (durable : s.fenced = true) :
    ∃ t u, Step s t ∧ Step t u ∧ u.opened = false ∧ u.locked = false := by
  refine ⟨{s with opened := false}, {s with opened := false, locked := false},
    .close detached idle unborrowed durable, .unlock rfl, rfl, rfl⟩

end Kv9.GroupRetirement
