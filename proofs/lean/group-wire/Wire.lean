import Std

namespace Kv9.GroupWire

inductive Method where
  | metadata | data
  deriving DecidableEq, Repr

inductive Version where
  | legacy | modern
  deriving DecidableEq, Repr

def method (region : Nat) : Option Method :=
  if region = 0 then some .metadata else if 1 < region then some .data else none

/-- The old receiver ignores region on its only Raft RPC. The new RPC is an
unknown method at that receiver, including after a successful modern session. -/
def receive (version : Version) (rpc : Method) (region : Nat) : Option Nat :=
  match version, rpc with
  | .legacy, .metadata => some 0
  | .legacy, .data => none
  | .modern, .metadata => if region = 0 then some 0 else none
  | .modern, .data => if 1 < region then some region else none

/-- Opaque, non-reused route generations are modeled by equality. Production
uses allocation identity retained by each queued envelope, not a wrapping ID. -/
def deliver (version : Version) (current queued region : Nat) : Option Nat :=
  if current = queued then (method region).bind (fun rpc => receive version rpc region)
  else none

theorem data_method (region : Nat) (data : 1 < region) :
    method region = some .data := by
  have nonzero : region ≠ 0 := by omega
  simp [method, nonzero, data]

theorem legacy_drops_data (current queued region : Nat) (data : 1 < region) :
    deliver .legacy current queued region = none := by
  simp [deliver, data_method region data, receive]

theorem modern_metadata_rejects_data (region : Nat) (data : 1 < region) :
    receive .modern .metadata region = none := by
  have nonzero : region ≠ 0 := by omega
  simp [receive, nonzero]

theorem modern_data_rejects_metadata : receive .modern .data 0 = none := by
  simp [receive]

theorem reserved_never_sent (version : Version) (current queued : Nat) :
    deliver version current queued 1 = none := by
  simp [deliver, method]

theorem stale_generation_dropped (version : Version) (current queued region : Nat)
    (stale : current ≠ queued) : deliver version current queued region = none := by
  simp [deliver, stale]

/-- For every endpoint version and every connection generation, a message
either stays in its exact group or is dropped. No old capability is a premise. -/
theorem no_cross_group_delivery (version : Version) (current queued region : Nat) :
    deliver version current queued region = none ∨
    deliver version current queued region = some region := by
  by_cases same : current = queued
  · by_cases zero : region = 0
    · cases version <;> simp [deliver, method, receive, same, zero]
    · by_cases data : 1 < region
      · cases version <;> simp [deliver, method, receive, same, zero, data]
      · simp [deliver, method, same, zero, data]
  · simp [deliver, same]

/-- Arbitrary sequences may downgrade, upgrade, or change addresses between
attempts. An old successful attempt provides no permission for a later one. -/
theorem arbitrary_reconnections_safe (versions : List Version) (current queued region : Nat) :
    ∀ version ∈ versions, deliver version current queued region = none ∨
      deliver version current queued region = some region := by
  intro version _
  exact no_cross_group_delivery version current queued region

theorem legacy_fallback_is_unsafe (region : Nat) (data : 1 < region) :
    receive .legacy .metadata region = some 0 ∧ region ≠ 0 := by
  constructor
  · rfl
  · omega

def accepts (rpc : Method) (region : Nat) : Bool := method region == some rpc

def admitBatch (rpc : Method) (regions : List Nat) : List Nat :=
  if regions.all (accepts rpc) then regions else []

theorem mixed_batch_rejected (rpc : Method) (regions : List Nat)
    (wrong : regions.all (accepts rpc) = false) : admitBatch rpc regions = [] := by
  simp [admitBatch, wrong]

/-- Queue counts abstract the two separate bounded Tokio senders. Payload
allocation, socket buffers and total node memory are outside this model. -/
structure Queues where
  metadata : Nat := 0
  data : Nat := 0
  deriving DecidableEq, Repr

def enqueue (capacity : Nat) (rpc : Method) (q : Queues) : Queues :=
  match rpc with
  | .metadata => {q with metadata := if q.metadata < capacity then q.metadata + 1 else q.metadata}
  | .data => {q with data := if q.data < capacity then q.data + 1 else q.data}

def Bounded (capacity : Nat) (q : Queues) : Prop :=
  q.metadata ≤ capacity ∧ q.data ≤ capacity

theorem enqueue_bounded (capacity : Nat) (rpc : Method) (q : Queues)
    (bounded : Bounded capacity q) : Bounded capacity (enqueue capacity rpc q) := by
  cases rpc <;> simp only [enqueue, Bounded] at * <;> split <;> omega

theorem data_preserves_metadata_capacity (capacity : Nat) (q : Queues) :
    (enqueue capacity .data q).metadata = q.metadata := rfl

theorem full_data_still_admits_metadata (capacity : Nat) (q : Queues)
    (room : q.metadata < capacity) (full : q.data = capacity) :
    (enqueue capacity .metadata q).metadata = q.metadata + 1 ∧
    (enqueue capacity .metadata q).data = capacity := by
  simp [enqueue, room, full]

inductive Step (capacity : Nat) : Queues → Queues → Prop where
  | send (q : Queues) (rpc : Method) : Step capacity q (enqueue capacity rpc q)
  | drain (q : Queues) (m d : Nat) :
      Step capacity q {metadata := q.metadata - m, data := q.data - d}
  | retry (q : Queues) : Step capacity q q

theorem step_bounded (capacity : Nat) {before after : Queues}
    (bounded : Bounded capacity before) (step : Step capacity before after) :
    Bounded capacity after := by
  cases step with
  | send rpc => exact enqueue_bounded capacity rpc before bounded
  | drain m d => simp only [Bounded] at *; omega
  | retry => exact bounded

inductive Reachable (capacity : Nat) : Queues → Prop where
  | initial : Reachable capacity {}
  | step {before after} : Reachable capacity before → Step capacity before after → Reachable capacity after

theorem reachable_bounded (capacity : Nat) {q : Queues} (run : Reachable capacity q) :
    Bounded capacity q := by
  induction run with
  | initial => simp [Bounded]
  | step _ transition ih => exact step_bounded capacity ih transition

end Kv9.GroupWire
