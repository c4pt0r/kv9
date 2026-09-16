import Std

namespace Kv9.ProtocolSnapshot

/-- One record contains the complete protocol tuple. Image identity includes the
opaque payload and full incoming/outgoing/learner configuration. -/
structure Image where
  cut : Nat
  imageTerm : Nat
  durableTerm : Nat
  vote : Nat
  configuration : Nat
  payload : Nat
  deriving DecidableEq, Repr

def Forward (old next : Image) : Prop :=
  old.cut < next.cut ∧ old.durableTerm ≤ next.durableTerm ∧
  (old.durableTerm = next.durableTerm → old.vote ≠ 0 → next.vote = old.vote) ∧
  next.imageTerm ≤ next.durableTerm

inductive Phase where
  | idle | writing | synced | published | failed | down | recovered
  deriving DecidableEq, Repr

structure State where
  phase : Phase
  disk : Image
  live : Image
  acknowledged : Bool
  deriving DecidableEq, Repr

def initial (old : Image) : State := ⟨.idle, old, old, false⟩

/-- Torn-frame recovery is a lower-layer premise: a failed unsynchronized
append may leave the previous record or the complete new record, never a tuple
assembled from both. A successful sync makes the complete frame durable. -/
inductive Step (old next : Image) : State → State → Prop where
  | begin (s) (idle : s.phase = .idle) (valid : Forward old next) :
      Step old next s {s with phase := .writing}
  | sync (s) (writing : s.phase = .writing) :
      Step old next s {s with phase := .synced, disk := next}
  | publish (s) (synced : s.phase = .synced) :
      Step old next s {s with phase := .published, live := next, acknowledged := true}
  | failWrite (s) (writing : s.phase = .writing) (retained : Image)
      (whole : retained = old ∨ retained = next) :
      Step old next s {s with phase := .failed, disk := retained}
  | failSync (s) (synced : s.phase = .synced) :
      Step old next s {s with phase := .failed}
  | crash (s) : Step old next s {s with phase := .down, live := old}
  | recover (s) (down : s.phase = .down) :
      Step old next s {s with phase := .recovered, live := s.disk}

def Safe (old next : Image) (s : State) : Prop :=
  (s.disk = old ∨ s.disk = next) ∧
  (s.live = old ∨ s.live = next) ∧
  (s.acknowledged = true → s.disk = next) ∧
  (s.phase = .idle ∨ s.phase = .writing →
    s.disk = old ∧ s.live = old ∧ s.acknowledged = false) ∧
  (s.phase = .synced ∨ s.phase = .published → s.disk = next) ∧
  (s.phase = .failed → s.live = old ∧ s.acknowledged = false) ∧
  (s.phase = .recovered → s.live = s.disk) ∧
  (s.phase = .synced → s.live = old ∧ s.acknowledged = false)

theorem initial_safe (old next : Image) : Safe old next (initial old) := by
  simp [Safe, initial]

theorem step_safe {old next before after} (safe : Safe old next before)
    (step : Step old next before after) : Safe old next after := by
  cases step <;> simp_all [Safe]

inductive Run (old next : Image) : State → Prop where
  | initial : Run old next (initial old)
  | step {before after} : Run old next before → Step old next before after → Run old next after

theorem run_safe {old next s} (run : Run old next s) : Safe old next s := by
  induction run with
  | initial => exact initial_safe old next
  | step _ step ih => exact step_safe ih step

theorem recovery_selects_one_complete_tuple {old next s} (run : Run old next s)
    (recovered : s.phase = .recovered) : s.live = old ∨ s.live = next := by
  rw [(run_safe run).2.2.2.2.2.2.1 recovered]
  exact (run_safe run).1

theorem success_requires_durable_pair {old next s} (run : Run old next s)
    (ack : s.acknowledged = true) : s.disk = next :=
  (run_safe run).2.2.1 ack

theorem failed_write_never_publishes {old next s} (run : Run old next s)
    (failed : s.phase = .failed) : s.live = old ∧ s.acknowledged = false :=
  (run_safe run).2.2.2.2.2.1 failed

theorem recovery_preserves_term {old next s} (run : Run old next s)
    (valid : Forward old next) : old.durableTerm ≤ s.disk.durableTerm := by
  rcases (run_safe run).1 with same | advanced
  · simp [same]
  · simpa [advanced] using valid.2.1

theorem recovery_preserves_same_term_vote {old next s} (run : Run old next s)
    (valid : Forward old next) (sameTerm : s.disk.durableTerm = old.durableTerm)
    (voted : old.vote ≠ 0) : s.disk.vote = old.vote := by
  rcases (run_safe run).1 with same | advanced
  · simp [same]
  · subst advanced
    exact valid.2.2.1 sameTerm.symm voted

/-- This increment cannot grant a serving capability from a protocol base. -/
def mayStart (firstIndex : Nat) : Bool := firstIndex == 1

theorem compacted_base_cannot_start (cut : Nat) (positive : 0 < cut) :
    mayStart (cut + 1) = false := by simp [mayStart]; omega

/-- The receive guard is before RawNode::step; all protocol fields stay fixed. -/
def receiveSnapshot (s : Image) : Image := s

theorem uninstalled_receive_preserves_state (s : Image) : receiveSnapshot s = s := rfl

/-- Snapshot serving returns the exact installed image or refuses a newer cut. -/
def serve (image : Image) (requested : Nat) : Option Image :=
  if requested ≤ image.cut then some image else none

theorem snapshot_reply_is_exact (image reply : Image) (requested : Nat)
    (returned : serve image requested = some reply) : reply = image ∧ requested ≤ reply.cut := by
  unfold serve at returned
  split at returned <;> simp_all

end Kv9.ProtocolSnapshot
