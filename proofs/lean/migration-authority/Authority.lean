import Std

namespace Kv9.MigrationAuthority

/-- One committed migration's exact root/operation/group/destination binding is
abstracted by `authority`; the canonical image manifest digest and its complete
object closure by `image`. Equality means equality of the whole encoding. -/
inductive Pin where
  | absent
  | held
  | published
  | quiesced
  | released
  deriving DecidableEq, Repr

structure State where
  intent : Option Nat := none
  subject : Option Nat := none
  source : Pin := .absent
  destination : Pin := .absent
  voting : Bool := false
  serving : Bool := false
  deriving DecidableEq, Repr

/-- Committed metadata transitions only. Replication atomicity, ledger recovery
and the immutable owner-binding rule are premises supplied by the existing
retention-ledger model, not re-proved here. There is deliberately no
constructor for quiescing or releasing either pin, and none that grants voting
or serving: those capabilities require a later committed settlement seam. -/
inductive Step (authority image : Nat) : State → State → Prop where
  | commit {s} (fresh : s.intent = none) :
      Step authority image s {s with intent := some authority}
  | confirm {s} (committed : s.intent = some authority) : Step authority image s s
  | acquire {s} (committed : s.intent = some authority) (fresh : s.source = .absent) :
      Step authority image s {s with source := .held, subject := some image}
  | publishSource {s} (held : s.source = .held) :
      Step authority image s {s with source := .published}
  | share {s} (committed : s.intent = some authority)
      (published : s.source = .published) (bound : s.subject = some image)
      (fresh : s.destination = .absent) :
      Step authority image s {s with destination := .held}
  | publishDestination {s} (held : s.destination = .held) :
      Step authority image s {s with destination := .published}

def Safe (authority image : Nat) (s : State) : Prop :=
  (s.source ≠ .absent → s.intent = some authority ∧ s.subject = some image) ∧
  (s.destination ≠ .absent →
    s.source = .published ∧ s.subject = some image ∧ s.intent = some authority) ∧
  s.source ≠ .quiesced ∧ s.source ≠ .released ∧
  s.destination ≠ .quiesced ∧ s.destination ≠ .released ∧
  s.voting = false ∧ s.serving = false

theorem initial_safe (authority image : Nat) : Safe authority image {} := by
  simp [Safe]

theorem step_safe (authority image : Nat) {s t : State}
    (safe : Safe authority image s) (step : Step authority image s t) :
    Safe authority image t := by
  obtain ⟨source, destination, rest⟩ := safe
  cases step with
  | share committed published bound fresh => exact ⟨by simp_all, by simp_all, by simp_all⟩
  | publishDestination held =>
    obtain ⟨srcPublished, subject, intent⟩ := destination (by simp [held])
    exact ⟨by simp_all, by simp_all, by simp_all⟩
  | _ => simp_all [Safe]

inductive Reachable (authority image : Nat) : State → Prop where
  | initial : Reachable authority image {}
  | step {s t} : Reachable authority image s → Step authority image s t →
      Reachable authority image t

theorem run_safe (authority image : Nat) {s : State}
    (run : Reachable authority image s) : Safe authority image s := by
  induction run with
  | initial => exact initial_safe authority image
  | step _ transition ih => exact step_safe authority image ih transition

theorem owners_require_committed_intent (authority image : Nat) {s : State}
    (run : Reachable authority image s) (pinned : s.source ≠ .absent) :
    s.intent = some authority ∧ s.subject = some image :=
  (run_safe authority image run).1 pinned

theorem destination_requires_published_source (authority image : Nat) {s : State}
    (run : Reachable authority image s) (shared : s.destination ≠ .absent) :
    s.source = .published :=
  ((run_safe authority image run).2.1 shared).1

theorem destination_binds_the_exact_image (authority image : Nat) {s : State}
    (run : Reachable authority image s) (shared : s.destination ≠ .absent) :
    s.subject = some image ∧ s.intent = some authority :=
  ⟨((run_safe authority image run).2.1 shared).2.1,
   ((run_safe authority image run).2.1 shared).2.2⟩

theorem no_quiesce_or_release_is_reachable (authority image : Nat) {s : State}
    (run : Reachable authority image s) :
    s.source ≠ .quiesced ∧ s.source ≠ .released ∧
      s.destination ≠ .quiesced ∧ s.destination ≠ .released := by
  obtain ⟨_, _, a, b, c, d, _⟩ := run_safe authority image run
  exact ⟨a, b, c, d⟩

theorem no_voting_or_serving_capability (authority image : Nat) {s : State}
    (run : Reachable authority image s) : s.voting = false ∧ s.serving = false := by
  obtain ⟨_, _, _, _, _, _, voting, serving⟩ := run_safe authority image run
  exact ⟨voting, serving⟩

theorem intent_is_monotone (authority image : Nat) {s t : State}
    (step : Step authority image s t) (existing : s.intent = some authority) :
    t.intent = some authority := by
  cases step <;> simp_all

theorem subject_is_monotone (authority image : Nat) {s t : State}
    (step : Step authority image s t) (bound : s.subject = some image) :
    t.subject = some image := by
  cases step <;> simp_all

theorem published_source_is_monotone (authority image : Nat) {s t : State}
    (step : Step authority image s t) (published : s.source = .published) :
    t.source = .published := by
  cases step <;> simp_all

theorem published_destination_is_monotone (authority image : Nat) {s t : State}
    (step : Step authority image s t) (published : s.destination = .published) :
    t.destination = .published := by
  cases step <;> simp_all

theorem a_description_alone_binds_nothing :
    ∃ s, Reachable 7 9 s ∧ s.intent = some 7 ∧ s.source = Pin.absent ∧
      s.destination = Pin.absent := by
  refine ⟨{intent := some 7}, .step .initial (.commit rfl), rfl, rfl, rfl⟩

theorem confirmation_is_not_a_new_mutation (authority image : Nat) {s : State}
    (committed : s.intent = some authority) :
    Step authority image s s :=
  .confirm committed

end Kv9.MigrationAuthority
