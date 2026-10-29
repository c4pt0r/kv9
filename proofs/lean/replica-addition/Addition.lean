import Std

namespace Kv9.ReplicaAddition

/-- The committed rows and live facts one replica plan is driven from
(docs/REPLICA-ADDITION.md §2). `plan` is the kind-111 row (with its kind-103 intent);
`capture`, `pinned`, `evidence`, `decided` are committed catalog facts; `learner`,
`installed`, `adopted`, `voter`, `sourceRemoved` are what the group leader or the
destination observes locally; `quorum` counts the group's voters. -/
structure State where
  plan : Bool := false
  move : Bool := false
  aborted : Bool := false
  learner : Bool := false
  capture : Bool := false
  pinned : Bool := false
  uploaded : Bool := false
  installed : Bool := false
  adopted : Bool := false
  evidence : Bool := false
  voter : Bool := false
  decided : Bool := false
  sourceRemoved : Bool := false
  destinationRemoved : Bool := false
  quorum : Nat := 3
  deriving DecidableEq, Repr

/-- The verbs the driver may call, one per party and pass. -/
inductive Action where
  | attach | planImage | bind | capture | install | adopt | record | promote | decide | remove
  deriving DecidableEq, Repr

/-- Enabledness: exactly the precondition column of the step table. -/
def enabled (s : State) : Action → Bool
  | .attach    => s.plan && !s.aborted && !s.learner && !s.voter
  | .planImage => s.plan && !s.aborted && s.learner && !s.capture
  | .bind      => s.plan && !s.aborted && s.capture && !s.pinned
  | .capture   => s.plan && !s.aborted && s.pinned && !s.evidence && !s.uploaded
  | .install   => s.plan && !s.aborted && s.pinned && s.uploaded && !s.installed
  | .adopt     => s.installed && !s.adopted
  | .record    => s.plan && !s.aborted && s.adopted && !s.evidence
  | .promote   => s.plan && !s.aborted && s.evidence && s.learner && !s.voter
  | .decide    => s.plan && s.move && !s.aborted && s.evidence && !s.decided
  | .remove    => s.plan && s.move && !s.aborted && s.decided && s.voter && !s.sourceRemoved

def apply (s : State) : Action → State
  | .attach    => {s with learner := true}
  | .planImage => {s with capture := true}
  | .bind      => {s with pinned := true}
  | .capture   => {s with uploaded := true}
  | .install   => {s with installed := true}
  | .adopt     => {s with adopted := true}
  | .record    => {s with evidence := true}
  | .promote   => {s with voter := true, learner := false, quorum := s.quorum + 1}
  | .decide    => {s with decided := true}
  | .remove    => {s with sourceRemoved := true, quorum := s.quorum - 1}

inductive Step : State → State → Prop where
  | commit {s} (fresh : s.plan = false) (m : Bool) : Step s {s with plan := true, move := m}
  | abort {s} : Step s {s with aborted := true}
  | act {s} (a : Action) (ok : enabled s a = true) : Step s (apply s a)

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

/-- The driver: the one step this state enables, in the order of the table. -/
def nextStep (s : State) : Option Action :=
  [Action.attach, .planImage, .bind, .capture, .install, .adopt, .record, .promote, .decide, .remove]
    |>.find? (enabled s)

def terminal (s : State) : Bool :=
  s.plan && s.evidence && s.voter && (!s.move || s.sourceRemoved)

def Safe (s : State) : Prop :=
  (s.capture = true → s.plan = true) ∧
  (s.pinned = true → s.capture = true) ∧
  (s.uploaded = true → s.pinned = true) ∧
  (s.installed = true → s.uploaded = true) ∧
  (s.adopted = true → s.installed = true) ∧
  (s.evidence = true → s.adopted = true) ∧
  (s.voter = true → s.evidence = true) ∧
  (s.decided = true → s.evidence = true ∧ s.move = true) ∧
  (s.sourceRemoved = true → s.decided = true ∧ s.voter = true) ∧
  3 ≤ s.quorum ∧
  (s.voter = true → s.sourceRemoved = false → s.quorum = 4) ∧
  (s.voter = false → s.quorum = 3) ∧
  s.destinationRemoved = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hC, hP, hU, hI, hA, hE, hV, hD, hR, hQ, hQ4, hQ3, hX⟩ := safe
  cases step with
  | commit fresh m =>
      exact ⟨by simp_all, hP, hU, hI, hA, hE, hV, by simp_all, hR, hQ, hQ4, hQ3, hX⟩
  | abort =>
      exact ⟨hC, hP, hU, hI, hA, hE, hV, hD, hR, hQ, hQ4, hQ3, hX⟩
  | act a ok =>
      cases a <;> simp only [enabled, Bool.and_eq_true, Bool.not_eq_true'] at ok <;>
        simp only [apply] <;>
        refine ⟨?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_, ?_⟩ <;>
        simp_all <;> omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- A1: every step is enabled only by the committed chain below it.
theorem promotion_requires_evidence {s : State} (run : Reachable s) (v : s.voter = true) :
    s.evidence = true :=
  (run_safe run).2.2.2.2.2.2.1 v

theorem evidence_requires_adoption_of_an_installed_pinned_image {s : State}
    (run : Reachable s) (e : s.evidence = true) :
    s.adopted = true ∧ s.installed = true ∧ s.pinned = true ∧ s.capture = true ∧ s.plan = true := by
  have safe := run_safe run
  have a := safe.2.2.2.2.2.1 e
  have i := safe.2.2.2.2.1 a
  have u := safe.2.2.2.1 i
  have p := safe.2.2.1 u
  have c := safe.2.1 p
  exact ⟨a, i, p, c, safe.1 c⟩

theorem removal_requires_the_decision_and_the_promoted_voter {s : State}
    (run : Reachable s) (r : s.sourceRemoved = true) :
    s.decided = true ∧ s.voter = true ∧ s.evidence = true :=
  let safe := run_safe run
  let dv := safe.2.2.2.2.2.2.2.2.1 r
  ⟨dv.1, dv.2, safe.2.2.2.2.2.2.1 dv.2⟩

theorem a_decision_requires_evidence_and_the_move_mode {s : State}
    (run : Reachable s) (d : s.decided = true) : s.evidence = true ∧ s.move = true :=
  (run_safe run).2.2.2.2.2.2.2.1 d

-- A2: the voter floor, and a Move passes through four voters.
theorem the_quorum_never_drops_below_three {s : State} (run : Reachable s) : 3 ≤ s.quorum :=
  (run_safe run).2.2.2.2.2.2.2.2.2.1

theorem a_promoted_destination_means_four_voters_until_the_source_goes {s : State}
    (run : Reachable s) (v : s.voter = true) (r : s.sourceRemoved = false) : s.quorum = 4 :=
  (run_safe run).2.2.2.2.2.2.2.2.2.2.1 v r

theorem removal_only_from_four_voters {s t : State} (run : Reachable s)
    (step : Step s t) (r : s.sourceRemoved = false) (r' : t.sourceRemoved = true) :
    s.quorum = 4 ∧ t.quorum = 3 := by
  have safe := run_safe run
  cases step with
  | commit fresh m => simp_all
  | abort => simp_all
  | act a ok =>
      cases a <;> simp only [apply] at r' <;> try simp_all
      simp only [enabled, Bool.and_eq_true, Bool.not_eq_true'] at ok
      have q4 := safe.2.2.2.2.2.2.2.2.2.2.1 ok.1.2 r
      simp only [apply]
      omega

-- A3: the destination is never removed.
theorem the_destination_is_never_removed {s : State} (run : Reachable s) :
    s.destinationRemoved = false :=
  (run_safe run).2.2.2.2.2.2.2.2.2.2.2.2

theorem the_source_is_removed_at_most_once {s t : State} (step : Step s t)
    (r : s.sourceRemoved = true) : t.quorum = s.quorum ∨ t.quorum = s.quorum + 1 := by
  cases step with
  | commit fresh m => simp
  | abort => simp
  | act a ok =>
      cases a <;> simp only [enabled, Bool.and_eq_true, Bool.not_eq_true'] at ok <;>
        simp only [apply] <;> simp_all

-- A4: after an abort nothing but adoption of an already installed image proceeds, and
-- no committed fact or configuration changes.
theorem an_abort_stops_the_driver {s : State} (aborted : s.aborted = true) (a : Action)
    (ok : enabled s a = true) : a = .adopt := by
  cases a <;> simp_all [enabled]

theorem no_step_after_abort_changes_the_voter_set {s t : State} (aborted : s.aborted = true)
    (step : Step s t) : t.quorum = s.quorum ∧ t.voter = s.voter ∧ t.sourceRemoved = s.sourceRemoved := by
  cases step with
  | commit fresh m => simp
  | abort => simp
  | act a ok =>
      have := an_abort_stops_the_driver aborted a ok
      subst this
      simp [apply]

-- A5: the driver's choice is sound and complete, hence deterministic.
theorem nextStep_is_enabled {s : State} {a : Action} (h : nextStep s = some a) :
    enabled s a = true := by
  unfold nextStep at h
  exact List.find?_some h

theorem nextStep_exists_until_terminal {s : State} (run : Reachable s)
    (plan : s.plan = true) (live : s.aborted = false) (open_ : terminal s = false) :
    ∃ a, nextStep s = some a := by
  have safe := run_safe run
  obtain ⟨-, -, hU, hI, hA, hE, hV, -, -, -, -, -, -⟩ := safe
  -- Exhibiting one enabled action is enough: `find?` succeeds when any element qualifies.
  suffices h : ∃ a, a ∈ [Action.attach, .planImage, .bind, .capture, .install, .adopt, .record,
      .promote, .decide, .remove] ∧ enabled s a = true by
    unfold nextStep
    exact Option.isSome_iff_exists.mp (List.find?_isSome.mpr h)
  by_cases hv : s.voter = true
  · -- Past promotion: the plan is still open only for a Move whose source still votes.
    have he := hV hv
    have hm : s.move = true ∧ s.sourceRemoved = false := by
      cases hm : s.move <;> cases hr : s.sourceRemoved <;> simp_all [terminal]
    by_cases hd : s.decided = true
    · exact ⟨.remove, by simp, by simp [enabled, plan, live, hv, hd, hm.1, hm.2]⟩
    · exact ⟨.decide, by simp, by simp_all [enabled]⟩
  · have hv' : s.voter = false := by simpa using hv
    by_cases hl : s.learner = true
    · by_cases hc : s.capture = true
      · by_cases hp : s.pinned = true
        · by_cases hu : s.uploaded = true
          · by_cases hi : s.installed = true
            · by_cases ha : s.adopted = true
              · by_cases he : s.evidence = true
                · exact ⟨.promote, by simp, by simp [enabled, plan, live, he, hl, hv']⟩
                · exact ⟨.record, by simp, by simp_all [enabled]⟩
              · exact ⟨.adopt, by simp, by simp_all [enabled]⟩
            · exact ⟨.install, by simp, by simp_all [enabled]⟩
          · have he : s.evidence = false := by
              cases he : s.evidence <;> simp_all
            exact ⟨.capture, by simp, by simp_all [enabled]⟩
        · exact ⟨.bind, by simp, by simp_all [enabled]⟩
      · exact ⟨.planImage, by simp, by simp_all [enabled]⟩
    · exact ⟨.attach, by simp, by simp_all [enabled]⟩

theorem nextStep_is_the_tables_choice {s : State} :
    (nextStep s = some .promote → s.evidence = true ∧ s.learner = true) ∧
    (nextStep s = some .remove → s.decided = true ∧ s.voter = true) ∧
    (nextStep s = some .install → s.pinned = true ∧ s.uploaded = true) := by
  refine ⟨fun h => ?_, fun h => ?_, fun h => ?_⟩ <;>
    have := nextStep_is_enabled h <;> simp_all [enabled]

-- Reachability: both terminal states are reached by the chain, in order.
theorem an_addition_reaches_four_voters :
    ∃ s, Reachable s ∧ terminal s = true ∧ s.move = false ∧ s.quorum = 4 := by
  refine ⟨⟨true, false, false, false, true, true, true, true, true, true, true, false, false, false, 4⟩,
    ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step (.step (.step (.step (.step .initial
    (.commit rfl false)) (.act .attach rfl)) (.act .planImage rfl)) (.act .bind rfl))
    (.act .capture rfl)) (.act .install rfl)) (.act .adopt rfl)) (.act .record rfl))
    (.act .promote rfl)

theorem a_move_reaches_three_voters_without_the_source :
    ∃ s, Reachable s ∧ terminal s = true ∧ s.move = true ∧ s.quorum = 3 ∧
      s.sourceRemoved = true := by
  refine ⟨⟨true, true, false, false, true, true, true, true, true, true, true, true, true, false, 3⟩,
    ?_, rfl, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step (.step (.step (.step (.step (.step (.step (.step .initial
    (.commit rfl true)) (.act .attach rfl)) (.act .planImage rfl)) (.act .bind rfl))
    (.act .capture rfl)) (.act .install rfl)) (.act .adopt rfl)) (.act .record rfl))
    (.act .promote rfl)) (.act .decide rfl)) (.act .remove rfl)

end Kv9.ReplicaAddition
