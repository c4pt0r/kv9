import Std

namespace Kv9.SourceCapture

/-- One capture attempt for one committed migration operation. `engineCut`
abstracts the frozen durable engine position; `configAt` the commit position
of the configuration the capture attaches. Ledger atomicity and the
installer's own validation are premises from their existing models. -/
structure State where
  cut : Option Nat := none
  configCut : Option Nat := none
  pinned : Bool := false
  uploaded : Bool := false
  described : Bool := false
  image : Option Nat := none
  serving : Bool := false
  voting : Bool := false
  deriving DecidableEq, Repr

/-- Committed/driver-owned transitions only. There is deliberately no step
that serves, votes, installs, releases a pin, or moves a frozen cut; and no
step may attach a configuration committed past the cut. -/
inductive Step (engineCut configAt : Nat) : State → State → Prop where
  | freeze {s} (fresh : s.cut = none)
      (bound : configAt ≤ engineCut) :
      Step engineCut configAt s
        {s with cut := some engineCut, configCut := some configAt}
  | pin {s} (frozen : s.cut = some engineCut) :
      Step engineCut configAt s {s with pinned := true}
  | upload {s} (pinned : s.pinned = true) :
      Step engineCut configAt s {s with uploaded := true}
  | describe {s} (uploaded : s.uploaded = true)
      (fresh : s.image = none ∨ s.image = some engineCut) :
      Step engineCut configAt s
        {s with described := true, image := some engineCut}
  | retry {s} (described : s.described = true) : Step engineCut configAt s s

inductive Reachable (engineCut configAt : Nat) : State → Prop where
  | initial : Reachable engineCut configAt {}
  | step {s t} : Reachable engineCut configAt s → Step engineCut configAt s t →
      Reachable engineCut configAt t

def Safe (engineCut configAt : Nat) (s : State) : Prop :=
  (s.cut ≠ none → s.cut = some engineCut ∧ s.configCut = some configAt ∧
    configAt ≤ engineCut) ∧
  (s.pinned = true → s.cut = some engineCut) ∧
  (s.uploaded = true → s.pinned = true) ∧
  (s.described = true → s.uploaded = true ∧ s.image = some engineCut) ∧
  (s.image ≠ none → s.image = some engineCut) ∧
  s.serving = false ∧ s.voting = false

theorem initial_safe (engineCut configAt : Nat) :
    Safe engineCut configAt {} := by simp [Safe]

theorem step_safe (engineCut configAt : Nat) {s t : State}
    (safe : Safe engineCut configAt s) (step : Step engineCut configAt s t) :
    Safe engineCut configAt t := by
  obtain ⟨cut, pinned, uploaded, described, image, rest⟩ := safe
  cases step with
  | freeze fresh bound => exact ⟨by simp_all, by simp_all, by simp_all,
      by simp_all, by simp_all, by simp_all⟩
  | _ => simp_all [Safe]

theorem run_safe (engineCut configAt : Nat) {s : State}
    (run : Reachable engineCut configAt s) : Safe engineCut configAt s := by
  induction run with
  | initial => exact initial_safe engineCut configAt
  | step _ transition ih => exact step_safe engineCut configAt ih transition

theorem configuration_is_at_or_before_the_cut (engineCut configAt : Nat)
    {s : State} (run : Reachable engineCut configAt s) (frozen : s.cut ≠ none) :
    s.configCut = some configAt ∧ configAt ≤ engineCut :=
  ⟨((run_safe engineCut configAt run).1 frozen).2.1,
   ((run_safe engineCut configAt run).1 frozen).2.2⟩

theorem upload_requires_committed_pins (engineCut configAt : Nat) {s : State}
    (run : Reachable engineCut configAt s) (uploaded : s.uploaded = true) :
    s.pinned = true ∧ s.cut = some engineCut := by
  have safe := run_safe engineCut configAt run
  exact ⟨safe.2.2.1 uploaded, safe.2.1 (safe.2.2.1 uploaded)⟩

theorem receipt_requires_pinned_upload_of_the_exact_cut (engineCut configAt : Nat)
    {s : State} (run : Reachable engineCut configAt s)
    (described : s.described = true) :
    s.uploaded = true ∧ s.pinned = true ∧ s.image = some engineCut := by
  have safe := run_safe engineCut configAt run
  obtain ⟨uploaded, image⟩ := safe.2.2.2.1 described
  exact ⟨uploaded, safe.2.2.1 uploaded, image⟩

theorem no_serving_or_voting_capability (engineCut configAt : Nat) {s : State}
    (run : Reachable engineCut configAt s) :
    s.serving = false ∧ s.voting = false := by
  obtain ⟨_, _, _, _, _, serving, voting⟩ := run_safe engineCut configAt run
  exact ⟨serving, voting⟩

theorem the_cut_is_immutable (engineCut configAt : Nat) {s t : State}
    (step : Step engineCut configAt s t) (frozen : s.cut = some engineCut) :
    t.cut = some engineCut := by
  cases step <;> simp_all

theorem one_image_per_operation (engineCut configAt : Nat) {s t : State}
    (step : Step engineCut configAt s t) (bound : s.image = some engineCut) :
    t.image = some engineCut := by
  cases step <;> simp_all

theorem retry_is_a_pure_receipt (engineCut configAt : Nat) {s : State}
    (described : s.described = true) : Step engineCut configAt s s :=
  .retry described

theorem a_frozen_cut_alone_uploads_nothing :
    ∃ s, Reachable 9 3 s ∧ s.cut = some 9 ∧ s.uploaded = false ∧
      s.described = false := by
  refine ⟨{cut := some 9, configCut := some 3}, ?_, rfl, rfl, rfl⟩
  exact .step .initial (.freeze rfl (by omega))

end Kv9.SourceCapture
