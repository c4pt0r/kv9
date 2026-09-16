import Std

namespace Kv9.RoutedClient

inductive Phase where
  | lookup | ready | pending | unknown | done
  deriving DecidableEq, Repr

structure State where
  phase : Phase := .lookup
  attempts : Nat := 0
  sent : Nat := 0
  closed : Nat := 0
  effects : List Nat := []
  now : Nat := 0

/-- One logical write, including every discovery and data attempt. Closing a
data attempt requires a trustworthy no-effect refusal. A timeout is not one. -/
inductive Step (budget deadline : Nat) : State → State → Prop where
  | lookup (s) (ready : Bool) (phase : s.phase = .lookup)
      (room : s.attempts < budget) (time : s.now < deadline) :
      Step budget deadline s {s with attempts := s.attempts + 1, phase := if ready then .ready else .lookup}
  | invalidate (s) (phase : s.phase = .ready) :
      Step budget deadline s {s with phase := .lookup}
  | dispatch (s) (phase : s.phase = .ready)
      (room : s.attempts < budget) (time : s.now < deadline) :
      Step budget deadline s {s with attempts := s.attempts + 1, sent := s.sent + 1, phase := .pending}
  | refuse (s) (refresh : Bool) (phase : s.phase = .pending)
      (noEffects : s.effects = []) :
      Step budget deadline s {s with closed := s.sent, phase := if refresh then .lookup else .ready}
  | effect (s) (i : Nat) (live : s.closed < i) (sent : i ≤ s.sent) :
      Step budget deadline s {s with effects := i :: s.effects}
  | unknown (s) (phase : s.phase = .pending) :
      Step budget deadline s {s with phase := .unknown}
  | success (s) (phase : s.phase = .pending) :
      Step budget deadline s {s with phase := .done}
  | stop (s) (phase : s.phase = .lookup ∨ s.phase = .ready) :
      Step budget deadline s {s with phase := .done}
  | tick (s) (next : Nat) (forward : s.now ≤ next) :
      Step budget deadline s {s with now := next}

def Invariant (budget : Nat) (s : State) : Prop :=
  s.attempts ≤ budget ∧ s.sent ≤ s.attempts ∧ s.closed ≤ s.sent ∧
  s.sent ≤ s.closed + 1 ∧
  (∀ i ∈ s.effects, s.closed < i ∧ i ≤ s.sent) ∧
  ((s.phase = .lookup ∨ s.phase = .ready) → s.sent = s.closed)

theorem initial_invariant (budget : Nat) : Invariant budget {} := by
  simp [Invariant]

theorem step_invariant (budget deadline : Nat) {s t : State}
    (step : Step budget deadline s t) (inv : Invariant budget s) : Invariant budget t := by
  rcases inv with ⟨h1, h2, h3, h4, h5, h6⟩
  cases step <;> simp_all [Invariant]
  · omega
  · constructor
    · omega
    · intro i hi
      have effect := h5 i hi
      omega

inductive Run (budget deadline : Nat) : State → State → Prop where
  | nil (s) : Run budget deadline s s
  | next {s t u} : Run budget deadline s t → Step budget deadline t u → Run budget deadline s u

theorem trace_invariant (budget deadline : Nat) {s t : State}
    (run : Run budget deadline s t) (inv : Invariant budget s) : Invariant budget t := by
  induction run with
  | nil => exact inv
  | next _ step ih => exact step_invariant budget deadline step ih

theorem at_most_one_effectful_dispatch (budget : Nat) (s : State)
    (inv : Invariant budget s) (i j : Nat) (hi : i ∈ s.effects) (hj : j ∈ s.effects) : i = j := by
  rcases inv with ⟨_, _, _, bound, effects, _⟩
  have ei := effects i hi
  have ej := effects j hj
  omega

theorem arbitrary_trace_has_no_replayed_write (budget deadline : Nat) {s : State}
    (run : Run budget deadline {} s) (i j : Nat)
    (hi : i ∈ s.effects) (hj : j ∈ s.effects) : i = j := by
  exact at_most_one_effectful_dispatch budget s
    (trace_invariant budget deadline run (initial_invariant budget)) i j hi hj

theorem unknown_step_freezes_dispatch (budget deadline : Nat) {s t : State}
    (step : Step budget deadline s t) (unknown : s.phase = .unknown) :
    t.phase = .unknown ∧ t.sent = s.sent ∧ t.attempts = s.attempts := by
  cases step <;> simp_all

theorem unknown_trace_freezes_dispatch (budget deadline : Nat) {s t : State}
    (run : Run budget deadline s t) (unknown : s.phase = .unknown) :
    t.phase = .unknown ∧ t.sent = s.sent ∧ t.attempts = s.attempts := by
  induction run with
  | nil => exact ⟨unknown, rfl, rfl⟩
  | next _ step ih =>
    obtain ⟨phase, sent, attempts⟩ := unknown_step_freezes_dispatch budget deadline step ih.1
    exact ⟨phase, sent.trans ih.2.1, attempts.trans ih.2.2⟩

theorem each_rpc_consumes_shared_budget (budget deadline : Nat) {s t : State}
    (step : Step budget deadline s t) (rpc : s.attempts ≠ t.attempts) :
    t.attempts = s.attempts + 1 ∧ s.attempts < budget ∧ s.now < deadline := by
  cases step <;> simp_all

theorem dispatch_requires_original_budget_and_deadline (budget deadline : Nat) {s t : State}
    (step : Step budget deadline s t) (dispatch : s.sent ≠ t.sent) :
    s.attempts < budget ∧ s.now < deadline ∧ t.attempts = s.attempts + 1 := by
  cases step <;> simp_all

theorem no_new_rpc_after_deadline (budget deadline : Nat) {s t : State}
    (step : Step budget deadline s t) (expired : deadline ≤ s.now) :
    t.attempts = s.attempts ∧ t.sent = s.sent := by
  cases step <;> simp_all <;> omega

/-- Late completion after Unknown remains possible; client termination is not
server cancellation. This is an admitted transition of the actual model. -/
theorem unknown_can_complete_late (budget deadline : Nat) (s : State)
    (live : s.closed < s.sent) :
    Step budget deadline {s with phase := .unknown}
      {s with phase := .unknown, effects := s.sent :: s.effects} := by
  exact Step.effect _ s.sent live (Nat.le_refl _)

structure Scope where
  root : Nat
  creation : Nat
  region : Nat
  tenant : Nat
  keyspace : Nat
  conf : Nat
  version : Nat
  lower : Nat
  upper : Option Nat
  sealed : Bool
  deriving DecidableEq

def contains (scope : Scope) (key : Nat) : Bool :=
  decide (scope.lower ≤ key) && scope.upper.all (fun upper => decide (key < upper))

def discovered (root tenant keyspace : Nat) (scope : Scope) (key : Nat) : Bool :=
  decide (scope.root = root) && decide (scope.tenant = tenant) &&
  decide (scope.keyspace = keyspace) && !scope.sealed && contains scope key

def authorized (owner request : Scope) (keys : List Nat) : Bool :=
  decide (owner = request) && !owner.sealed && keys.all (contains owner)

theorem discovered_scope_matches_client (root tenant keyspace : Nat) (scope : Scope) (key : Nat)
    (ok : discovered root tenant keyspace scope key = true) :
    scope.root = root ∧ scope.tenant = tenant ∧ scope.keyspace = keyspace ∧
    scope.sealed = false ∧ contains scope key = true := by
  simpa [discovered, and_assoc] using ok

theorem server_requires_complete_scope (owner request : Scope) (keys : List Nat)
    (ok : authorized owner request keys = true) : owner = request := by
  simp_all [authorized]

theorem successful_batch_has_no_foreign_key (owner request : Scope) (keys : List Nat)
    (ok : authorized owner request keys = true) : keys.all (contains owner) = true := by
  simp only [authorized, Bool.and_eq_true] at ok
  exact ok.2

theorem sealed_owner_refuses (owner request : Scope) (keys : List Nat)
    (sealed : owner.sealed = true) : authorized owner request keys = false := by
  simp [authorized, sealed]

end Kv9.RoutedClient
