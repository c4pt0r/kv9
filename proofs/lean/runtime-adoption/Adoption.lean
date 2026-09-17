import Std

namespace Kv9.RuntimeAdoption

/-- One destination group generation crossing from installation to runtime.
Selection atomicity and whole-pair image verification are premises from the
joint-install model; cut exactness from the source-capture model; learner
membership from the attach model. Adoption is ONE-WAY: it closes the
installation path and restores the driver's unified position from the
installed cut alone — never from a guessed watermark. The replica neither
serves nor promotes here, and a network snapshot has no installing step. -/
structure State where
  selected : Bool := false
  adopted : Bool := false
  installationOpen : Bool := true
  cut : Nat := 0
  engine : Nat := 0
  driver : Option Nat := none
  serving : Bool := false
  promoted : Bool := false
  networkSnapshot : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | install {s} (opened : s.installationOpen = true) (fresh : s.adopted = false)
      (c : Nat) (exact : 0 < c) :
      Step s {s with selected := true, cut := c}
  | observeEngine {s} (fresh : s.adopted = false) (e : Nat) :
      Step s {s with engine := e}
  | adopt {s} (sel : s.selected = true) (floor : s.cut ≤ s.engine) :
      Step s {s with adopted := true, installationOpen := false, driver := some s.cut}
  | restart {s} : Step s {s with driver := none}
  | readopt {s} (marked : s.adopted = true) :
      Step s {s with driver := some s.cut}
  | tail {s} (marked : s.adopted = true) (d : Nat) :
      Step s {s with engine := s.engine + d, driver := some (s.engine + d)}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.adopted = true →
    s.selected = true ∧ s.installationOpen = false ∧ s.cut ≤ s.engine) ∧
  (s.selected = true → 0 < s.cut) ∧
  (∀ p, s.driver = some p → s.adopted = true ∧ s.cut ≤ p ∧ p ≤ s.engine) ∧
  s.serving = false ∧ s.promoted = false ∧ s.networkSnapshot = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hA, hS, hD, hServ, hProm, hNet⟩ := safe
  cases step with
  | install opened fresh c exact =>
      refine ⟨by simp_all, by simp_all, fun p sp => ?_, hServ, hProm, hNet⟩
      exact absurd (hD p sp).1 (by simp_all)
  | observeEngine fresh e =>
      refine ⟨by simp_all, hS, fun p sp => ?_, hServ, hProm, hNet⟩
      exact absurd (hD p sp).1 (by simp_all)
  | adopt sel floor =>
      exact ⟨by simp_all, hS, fun p sp => by simp_all, hServ, hProm, hNet⟩
  | restart =>
      exact ⟨hA, hS, fun p sp => by simp_all, hServ, hProm, hNet⟩
  | readopt marked =>
      refine ⟨hA, hS, fun p sp => ?_, hServ, hProm, hNet⟩
      have floor := (hA marked).2.2
      simp_all
  | tail marked d =>
      refine ⟨fun m => ⟨(hA m).1, (hA m).2.1, ?_⟩, hS, fun p sp => ?_, hServ, hProm, hNet⟩
      · have floor := (hA m).2.2
        show s.cut ≤ s.engine + d
        omega
      · have floor := (hA marked).2.2
        have ep : s.engine + d = p := by simpa using sp
        refine ⟨marked, ?_, ?_⟩
        · show s.cut ≤ p
          omega
        · show p ≤ s.engine + d
          omega

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem adoption_requires_a_selected_generation {s : State}
    (run : Reachable s) (marked : s.adopted = true) : s.selected = true :=
  ((run_safe run).1 marked).1

theorem adoption_closes_installation {s : State}
    (run : Reachable s) (marked : s.adopted = true) :
    s.installationOpen = false :=
  ((run_safe run).1 marked).2.1

theorem the_engine_never_sits_behind_an_adopted_cut {s : State}
    (run : Reachable s) (marked : s.adopted = true) : s.cut ≤ s.engine :=
  ((run_safe run).1 marked).2.2

theorem the_restored_position_sits_between_cut_and_engine {s : State} {p : Nat}
    (run : Reachable s) (restored : s.driver = some p) :
    s.adopted = true ∧ s.cut ≤ p ∧ p ≤ s.engine :=
  (run_safe run).2.2.1 p restored

theorem no_serving_and_no_promotion {s : State} (run : Reachable s) :
    s.serving = false ∧ s.promoted = false :=
  ⟨(run_safe run).2.2.2.1, (run_safe run).2.2.2.2.1⟩

theorem network_snapshots_never_install {s : State} (run : Reachable s) :
    s.networkSnapshot = false :=
  (run_safe run).2.2.2.2.2

theorem adoption_is_permanent {s t : State} (step : Step s t)
    (marked : s.adopted = true) : t.adopted = true := by
  cases step <;> simp_all

theorem installation_never_reopens {s t : State} (step : Step s t)
    (closed : s.installationOpen = false) : t.installationOpen = false := by
  cases step <;> simp_all

theorem restart_forgets_and_readoption_restores_the_cut {s : State}
    (marked : s.adopted = true) :
    Step s {s with driver := none} ∧
    Step {s with driver := none} {s with driver := some s.cut} :=
  ⟨.restart, .readopt marked⟩

theorem a_selected_generation_alone_starts_nothing :
    ∃ s, Reachable s ∧ s.selected = true ∧ s.adopted = false ∧
      s.driver = none := by
  exact ⟨{selected := true, cut := 1},
    .step .initial (.install rfl rfl 1 Nat.one_pos), rfl, rfl, rfl⟩

theorem after_adoption_the_engine_never_regresses {s t : State}
    (step : Step s t) (marked : s.adopted = true) : s.engine ≤ t.engine := by
  cases step <;> simp_all <;> omega

end Kv9.RuntimeAdoption
