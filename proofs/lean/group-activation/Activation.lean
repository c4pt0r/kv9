import Std

namespace Kv9.GroupActivation

inductive Phase where
  | intent | ready | active
  deriving DecidableEq, Repr

/-- A prepared store has exact committed creation identity and valid stores.
Recovery validation is renewed in each process; durable Active is one-way. -/
structure Replica where
  phase : Phase := .ready
  validated : Bool := true
  authorized : Bool := false
  owner : Bool := false
  failed : Bool := false
  sent : Bool := false
  deriving DecidableEq, Repr

inductive ReplicaStep : Replica → Replica → Prop where
  | authorize (s) : ReplicaStep s {s with authorized := true}
  | publish (s) (ready : s.phase = .ready) (validated : s.validated = true)
      (healthy : s.failed = false) : ReplicaStep s {s with phase := .active}
  | validate (s) (healthy : s.failed = false) : ReplicaStep s {s with validated := true}
  | start (s) (active : s.phase = .active) (validated : s.validated = true)
      (authorized : s.authorized = true) (healthy : s.failed = false) :
      ReplicaStep s {s with owner := true}
  | send (s) (owner : s.owner = true) : ReplicaStep s {s with sent := true}
  | fail (s) : ReplicaStep s {s with failed := true, owner := false}
  | restart (s) : ReplicaStep s {s with validated := false, authorized := false, owner := false, failed := false}

def Safe (s : Replica) : Prop :=
  (s.owner = true → s.phase = .active ∧ s.validated = true ∧ s.authorized = true ∧ s.failed = false) ∧
  (s.sent = true → s.phase = .active)

theorem replica_initial_safe : Safe {} := by simp [Safe]

theorem replica_step_safe {before after : Replica} (safe : Safe before)
    (step : ReplicaStep before after) : Safe after := by
  cases step <;> simp_all [Safe]

inductive ReplicaRun : Replica → Prop where
  | initial : ReplicaRun {}
  | step {before after} : ReplicaRun before → ReplicaStep before after → ReplicaRun after

theorem replica_run_safe {s : Replica} (run : ReplicaRun s) : Safe s := by
  induction run with
  | initial => exact replica_initial_safe
  | step _ step ih => exact replica_step_safe ih step

theorem send_requires_durable_active {s : Replica} (run : ReplicaRun s) (sent : s.sent = true) :
    s.phase = .active := (replica_run_safe run).2 sent

theorem active_is_monotone {before after : Replica} (step : ReplicaStep before after)
    (active : before.phase = .active) : after.phase = .active := by
  cases step <;> simp_all

def mayInitialize (phase : Phase) : Bool := phase == .intent

theorem active_never_initializes : mayInitialize .active = false := rfl

theorem restart_requires_new_validation (s : Replica) :
    ({s with validated := false, authorized := false, owner := false, failed := false} : Replica).owner = false ∧
    ({s with validated := false, authorized := false, owner := false, failed := false} : Replica).validated = false := by
  simp

/-- Recovery accepts only the exact identity and existing store topology, with
every positioned engine frame justified by durable committed Raft history.
These predicates are implementation checks, not assumptions of the caller. -/
def mayRecover (identity logs engine history : Bool) : Bool := identity && logs && engine && history

theorem recovery_requires_all_gates (identity logs engine history : Bool)
    (accepted : mayRecover identity logs engine history = true) :
    identity = true ∧ logs = true ∧ engine = true ∧ history = true := by
  simpa [mayRecover, and_assoc] using accepted

/-- A registry slot has at most one worker, and one coalesced child work bit.
The shared wake bit is a hint; clearing it cannot consume child work. -/
structure Slot where
  pending : Bool := false
  owner : Option Nat := none
  wake : Bool := false
  deriving DecidableEq, Repr

def notify (s : Slot) : Slot := {s with pending := true, wake := true}
def release (s : Slot) : Slot := {s with owner := none, wake := true}
def clearWake (s : Slot) : Slot := {s with wake := false}

inductive WorkStep : Slot → Slot → Prop where
  | notify (s) : WorkStep s (notify s)
  | claim (s) (worker : Nat) (idle : s.owner = none) : WorkStep s {s with owner := some worker}
  | begin (s) (worker : Nat) (owned : s.owner = some worker) : WorkStep s {s with pending := false}
  | release (s) : WorkStep s (release s)
  | clearWake (s) : WorkStep s (clearWake s)

theorem notifications_coalesce (s : Slot) : notify (notify s) = notify s := rfl

theorem release_preserves_inflight_work (s : Slot) : (release s).pending = s.pending := rfl

theorem shared_wake_cannot_consume_child_work (s : Slot) : (clearWake s).pending = s.pending := rfl

theorem work_consumption_requires_owner {before after : Slot} (step : WorkStep before after)
    (pending : before.pending = true) (consumed : after.pending = false) : before.owner ≠ none := by
  cases step <;> simp_all [notify, release, clearWake]

theorem work_step_never_replaces_owner {before after : Slot} (step : WorkStep before after)
    (first second : Nat) (owned : before.owner = some first) (retained : after.owner = some second) :
    first = second := by
  cases step <;> simp_all [notify, release, clearWake]

/-- Only the independent monotonic deadline emits a tick. A long pause does
not create an election-tick burst; notification count is absent from this rule. -/
def tick (now next period : Nat) : Nat × Nat :=
  if next ≤ now then (1, now + period) else (0, next)

theorem tick_count_bounded (now next period : Nat) : (tick now next period).1 ≤ 1 := by
  simp only [tick]; split <;> simp

theorem due_tick_moves_deadline_forward (now next period : Nat)
    (positive : 0 < period) (due : next ≤ now) : now < (tick now next period).2 := by
  simp [tick, due]; omega

end Kv9.GroupActivation
