import Std

namespace Kv9.JointInstall

/-- Flags refer to a single immutable generation while both owner locks are
held. Successful file/directory sync and atomic selector rename are premises.
Remote object hash verification and source authority are separate obligations. -/
structure State where
  engine : Bool := false
  protocol : Bool := false
  sealed : Bool := false
  visible : Bool := false
  durable : Bool := false
  receipt : Bool := false
  failed : Bool := false
  deriving DecidableEq, Repr

def initial : State := {}

inductive Step : State → State → Prop where
  | engine (s) (owner : s.failed = false) : Step s {s with engine := true}
  | protocol (s) (owner : s.failed = false) (prepared : s.engine = true) :
      Step s {s with protocol := true}
  | seal (s) (owner : s.failed = false) (engine : s.engine = true)
      (protocol : s.protocol = true) : Step s {s with sealed := true}
  | rename (s) (owner : s.failed = false) (sealed : s.sealed = true) :
      Step s {s with visible := true}
  | sync (s) (owner : s.failed = false) (renamed : s.visible = true) :
      Step s {s with durable := true}
  | acknowledge (s) (owner : s.failed = false) (synced : s.durable = true) :
      Step s {s with receipt := true}
  | failure (s) : Step s {s with failed := true, receipt := false}
  /-- Namespace recovery may retain either rename outcome before directory
  sync, and must retain the new selector after successful directory sync. -/
  | crash (s) (selected : Bool)
      (keepSynced : s.durable = true → selected = true)
      (noInvent : selected = true → s.visible = true) :
      Step s {s with visible := selected, durable := selected, failed := true, receipt := false}
  /-- Reopen acquires both locks and verifies the selected files. No serving
  handle is minted. Incomplete unselected generations remain unselected. -/
  | reopen (s) : Step s {s with failed := false, receipt := false}

def Safe (s : State) : Prop :=
  (s.sealed = true → s.engine = true ∧ s.protocol = true) ∧
  (s.visible = true → s.sealed = true) ∧
  (s.durable = true → s.visible = true) ∧
  (s.receipt = true → s.durable = true) ∧
  (s.failed = true → s.receipt = false)

theorem initial_safe : Safe initial := by simp [Safe, initial]

theorem step_safe {before after} (safe : Safe before) (step : Step before after) : Safe after := by
  cases step <;> simp_all [Safe]

inductive Run : State → Prop where
  | initial : Run initial
  | step {before after} : Run before → Step before after → Run after

theorem run_safe {s} (run : Run s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ step ih => exact step_safe ih step

theorem selected_requires_both_durable {s} (run : Run s) (selected : s.visible = true) :
    s.engine = true ∧ s.protocol = true :=
  (run_safe run).1 ((run_safe run).2.1 selected)

theorem receipt_requires_selector_sync {s} (run : Run s) (receipt : s.receipt = true) :
    s.durable = true := (run_safe run).2.2.2.1 receipt

theorem success_survives_every_step {before after} (step : Step before after)
    (durable : before.durable = true) : after.durable = true := by
  cases step <;> simp_all

theorem failed_has_no_receipt {s} (run : Run s) (failed : s.failed = true) :
    s.receipt = false := (run_safe run).2.2.2.2 failed

structure Pair where
  engineImage : Nat
  protocolImage : Nat
  cut : Nat
  term : Nat
  vote : Nat
  target : Nat
  deriving DecidableEq, Repr

def Valid (p : Pair) : Prop := p.engineImage = p.protocolImage

def Forward (old next : Pair) : Prop :=
  old.cut < next.cut ∧ old.term ≤ next.term ∧
  (old.term = next.term → old.vote ≠ 0 → next.vote = old.vote) ∧ next.target = old.target

/-- A missing/corrupt selected generation is refusal, never fallback. The valid
bit represents exact file/SST/range/position checks, not source authorization. -/
def recover (old next : Pair) (selected valid : Bool) (target : Nat) : Option Pair :=
  let pair := if selected then next else old
  if valid && (pair.target == target) then some pair else none

theorem recovery_is_one_whole_pair {old next selected valid target result}
    (ok : recover old next selected valid target = some result) : result = old ∨ result = next := by
  cases selected <;> simp_all [recover] <;> split at ok <;> simp_all

theorem recovery_has_no_mixed_image {old next selected valid target result}
    (oldValid : Valid old) (nextValid : Valid next)
    (ok : recover old next selected valid target = some result) : Valid result := by
  rcases recovery_is_one_whole_pair ok with h | h <;> simp_all

theorem corrupted_selection_refuses (old next : Pair) (selected : Bool) (target : Nat) :
    recover old next selected false target = none := by simp [recover]

theorem recovered_target_is_exact {old next selected valid target result}
    (ok : recover old next selected valid target = some result) : result.target = target := by
  cases selected <;> cases valid <;> simp_all [recover] <;> grind

theorem recovery_does_not_lower_term {old next selected valid target result}
    (forward : Forward old next) (ok : recover old next selected valid target = some result) :
    old.term ≤ result.term := by
  rcases recovery_is_one_whole_pair ok with h | h
  · simp [h]
  · simpa [h] using forward.2.1

theorem recovery_preserves_same_term_vote {old next selected valid target result}
    (forward : Forward old next) (ok : recover old next selected valid target = some result)
    (same : result.term = old.term) (voted : old.vote ≠ 0) : result.vote = old.vote := by
  rcases recovery_is_one_whole_pair ok with h | h
  · simp [h]
  · subst h
    exact forward.2.2.1 same.symm voted

end Kv9.JointInstall
