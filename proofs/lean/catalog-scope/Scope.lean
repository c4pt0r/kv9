namespace Kv9.CatalogScope

/-- One committed catalog row as the router sees it: an identity, the regions it
concerns (the row → regions map of docs/META-REPLICA-MODES.md §3b) and whether its
kind declares that map at all. -/
structure Row where
  id : Nat
  regions : List Nat
  declared : Bool
  deriving DecidableEq, Repr

/-- Does the row concern any region of `scope`? -/
def relevant (scope : List Nat) (r : Row) : Bool :=
  r.regions.any (fun q => scope.contains q)

/-- One scoped node. `seen` is a ghost: the full catalog up to the node's position
(its length is the position); `rows` is what the node actually stores; `scope` the
regions it is subscribed to; `halted` that its stream stopped on an undeclared row. -/
structure State where
  seen : List Row := []
  rows : List Row := []
  scope : List Nat := []
  halted : Bool := false
  deriving Repr

def State.pos (s : State) : Nat := s.seen.length

/-- The subscriber's handling of one stream message carrying the row committed at
position `at`. Only the message for exactly the current position is applied; an
undeclared row stops the stream; a row outside the scope only advances the position. -/
def deliver (s : State) (at_ : Nat) (r : Row) : State :=
  if at_ = s.pos ∧ s.halted = false then
    if r.declared = true then
      if relevant s.scope r = true then
        { s with seen := s.seen ++ [r], rows := s.rows ++ [r] }
      else
        { s with seen := s.seen ++ [r] }
    else
      { s with halted := true }
  else s

/-- Entering a region: the closure of the region at the current position comes in
the same step. -/
def enter (s : State) (q : Nat) : State :=
  { s with scope := q :: s.scope,
           rows := s.rows ++ s.seen.filter (fun x => x.regions.contains q) }

/-- Leaving a region: it stops being in the scope (its rows are deleted later, once
the local replica is retired; keeping them is safe). -/
def leave (s : State) (q : Nat) : State :=
  { s with scope := s.scope.filter (fun x => x != q) }

inductive Step : State → State → Prop where
  | deliver {s} (at_ : Nat) (r : Row) : Step s (deliver s at_ r)
  | enter {s} (q : Nat) (live : s.halted = false) : Step s (enter s q)
  | leave {s} (q : Nat) : Step s (leave s q)

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

/-- Complete for every scoped region, sound overall, and every committed row the
node has seen declares its regions. -/
def Safe (s : State) : Prop :=
  (∀ q, q ∈ s.scope → ∀ x, x ∈ s.seen → q ∈ x.regions → x ∈ s.rows)
    ∧ (∀ x, x ∈ s.rows → x ∈ s.seen)
    ∧ (∀ x, x ∈ s.seen → x.declared = true)

theorem initial_safe : Safe {} := by
  refine ⟨?_, ?_, ?_⟩ <;> intros <;> simp_all

theorem relevant_of_mem {scope : List Nat} {r : Row} {q : Nat}
    (hq : q ∈ scope) (hr : q ∈ r.regions) : relevant scope r = true := by
  unfold relevant
  rw [List.any_eq_true]
  exact ⟨q, hr, by simp [hq]⟩

theorem deliver_safe {s : State} (safe : Safe s) (at_ : Nat) (r : Row) : Safe (deliver s at_ r) := by
  obtain ⟨hC, hS, hD⟩ := safe
  unfold deliver
  by_cases g : at_ = s.pos ∧ s.halted = false
  · rw [if_pos g]
    by_cases d : r.declared = true
    · rw [if_pos d]
      by_cases rel : relevant s.scope r = true
      · rw [if_pos rel]
        refine ⟨?_, ?_, ?_⟩
        · intro q hq x hx hxq
          simp only [List.mem_append, List.mem_singleton] at hx ⊢
          rcases hx with hx | hx
          · exact Or.inl (hC q hq x hx hxq)
          · exact Or.inr hx
        · intro x hx
          simp only [List.mem_append, List.mem_singleton] at hx ⊢
          rcases hx with hx | hx
          · exact Or.inl (hS x hx)
          · exact Or.inr hx
        · intro x hx
          simp only [List.mem_append, List.mem_singleton] at hx
          rcases hx with hx | hx
          · exact hD x hx
          · subst hx; exact d
      · rw [if_neg rel]
        refine ⟨?_, ?_, ?_⟩
        · intro q hq x hx hxq
          simp only [List.mem_append, List.mem_singleton] at hx
          rcases hx with hx | hx
          · exact hC q hq x hx hxq
          · subst hx; exact absurd (relevant_of_mem hq hxq) rel
        · intro x hx
          exact List.mem_append_left _ (hS x hx)
        · intro x hx
          simp only [List.mem_append, List.mem_singleton] at hx
          rcases hx with hx | hx
          · exact hD x hx
          · subst hx; exact d
    · rw [if_neg d]
      exact ⟨hC, hS, hD⟩
  · rw [if_neg g]
    exact ⟨hC, hS, hD⟩

theorem enter_safe {s : State} (safe : Safe s) (q : Nat) : Safe (enter s q) := by
  obtain ⟨hC, hS, hD⟩ := safe
  refine ⟨?_, ?_, ?_⟩
  · intro q' hq x hx hxq
    simp only [enter, List.mem_cons] at hq
    simp only [enter, List.mem_append, List.mem_filter]
    rcases hq with hq | hq
    · subst hq; exact Or.inr ⟨hx, by simp [hxq]⟩
    · exact Or.inl (hC q' hq x hx hxq)
  · intro x hx
    simp only [enter, List.mem_append, List.mem_filter] at hx
    rcases hx with hx | hx
    · exact hS x hx
    · exact hx.1
  · exact hD

theorem leave_safe {s : State} (safe : Safe s) (q : Nat) : Safe (leave s q) := by
  obtain ⟨hC, hS, hD⟩ := safe
  refine ⟨?_, hS, hD⟩
  intro q' hq x hx hxq
  simp only [leave, List.mem_filter] at hq
  exact hC q' hq.1 x hx hxq

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  cases step with
  | deliver at_ r => exact deliver_safe safe at_ r
  | enter q _ => exact enter_safe safe q
  | leave q => exact leave_safe safe q

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

-- M1: for a scoped region, the node's local "is there such a row?" verdict equals
-- the full catalog's at the node's position, for ANY property of the row.
theorem absence_verdicts_match {s : State} (run : Reachable s) (q : Nat) (hq : q ∈ s.scope)
    (P : Row → Prop) :
    (∃ x, x ∈ s.seen ∧ q ∈ x.regions ∧ P x) ↔ (∃ x, x ∈ s.rows ∧ q ∈ x.regions ∧ P x) := by
  obtain ⟨hC, hS, _⟩ := run_safe run
  constructor
  · rintro ⟨x, hx, hxq, hp⟩; exact ⟨x, hC q hq x hx hxq, hxq, hp⟩
  · rintro ⟨x, hx, hxq, hp⟩; exact ⟨x, hS x hx, hxq, hp⟩

-- M3: a scoped node never holds a row the full catalog did not commit by its position.
theorem nothing_beyond_the_catalog {s : State} (run : Reachable s) : ∀ x, x ∈ s.rows → x ∈ s.seen :=
  (run_safe run).2.1

-- R2 (no broadcast): a change that concerns none of the node's regions delivers no
-- rows to it; only its position moves.
theorem an_unrelated_change_delivers_no_rows (s : State) (at_ : Nat) (r : Row)
    (unrelated : relevant s.scope r = false) : (deliver s at_ r).rows = s.rows := by
  unfold deliver
  by_cases g : at_ = s.pos ∧ s.halted = false
  · rw [if_pos g]
    by_cases d : r.declared = true
    · rw [if_pos d, if_neg (by simp [unrelated])]
    · rw [if_neg d]
  · rw [if_neg g]

-- R2: every row a delivery adds concerns a region in the node's scope.
theorem delivered_rows_are_in_scope (s : State) (at_ : Nat) (r : Row) :
    ∀ x, x ∈ (deliver s at_ r).rows → x ∈ s.rows ∨ relevant s.scope x = true := by
  intro x hx
  unfold deliver at hx
  by_cases g : at_ = s.pos ∧ s.halted = false
  · rw [if_pos g] at hx
    by_cases d : r.declared = true
    · rw [if_pos d] at hx
      by_cases rel : relevant s.scope r = true
      · rw [if_pos rel] at hx
        simp only [List.mem_append, List.mem_singleton] at hx
        rcases hx with hx | hx
        · exact Or.inl hx
        · subst hx; exact Or.inr rel
      · rw [if_neg rel] at hx; exact Or.inl hx
    · rw [if_neg d] at hx; exact Or.inl hx
  · rw [if_neg g] at hx; exact Or.inl hx

-- Idempotence: a message for any position other than the current one changes nothing
-- (a stale or duplicate message, or one past a gap).
theorem an_out_of_position_message_is_ignored (s : State) (at_ : Nat) (r : Row)
    (other : at_ ≠ s.pos) : deliver s at_ r = s := by
  unfold deliver
  rw [if_neg (by intro h; exact other h.1)]

-- Idempotence: delivering the same message twice equals delivering it once.
theorem a_duplicate_is_identity (s : State) (r : Row) :
    deliver (deliver s s.pos r) s.pos r = deliver s s.pos r := by
  by_cases g : s.halted = false
  · by_cases d : r.declared = true
    · have grown : (deliver s s.pos r).pos = s.pos + 1 := by
        unfold deliver
        rw [if_pos ⟨rfl, g⟩, if_pos d]
        by_cases rel : relevant s.scope r = true
        · rw [if_pos rel]; simp [State.pos]
        · rw [if_neg rel]; simp [State.pos]
      exact an_out_of_position_message_is_ignored _ _ _ (by omega)
    · have halted : deliver s s.pos r = { s with halted := true } := by
        unfold deliver; rw [if_pos ⟨rfl, g⟩, if_neg d]
      rw [halted]
      unfold deliver
      rw [if_neg (by simp)]
  · have same : deliver s s.pos r = s := by
      unfold deliver
      rw [if_neg (by intro h; exact g h.2)]
    rw [same, same]

-- M3: positions never go back.
theorem positions_are_monotonic {s t : State} (step : Step s t) : s.pos ≤ t.pos := by
  cases step with
  | deliver at_ r =>
      unfold deliver
      by_cases g : at_ = s.pos ∧ s.halted = false
      · rw [if_pos g]
        by_cases d : r.declared = true
        · rw [if_pos d]
          by_cases rel : relevant s.scope r = true
          · rw [if_pos rel]; simp [State.pos]
          · rw [if_neg rel]; simp [State.pos]
        · rw [if_neg d]; simp [State.pos]
      · rw [if_neg g]; exact Nat.le_refl _
  | enter q _ => simp [enter, State.pos]
  | leave q => simp [leave, State.pos]

-- Fail closed: once a row without a declared region map arrives, the stream is
-- stopped and the node's position never moves again.
theorem an_undeclared_row_stops_the_stream {s t : State} (halted : s.halted = true)
    (step : Step s t) : t.halted = true ∧ t.seen = s.seen := by
  cases step with
  | deliver at_ r =>
      unfold deliver
      rw [if_neg (by intro h; simp [halted] at h)]
      exact ⟨halted, rfl⟩
  | enter q live => simp [halted] at live
  | leave q => exact ⟨halted, rfl⟩

-- And the catalog a node has seen never contains an undeclared row.
theorem every_seen_row_is_declared {s : State} (run : Reachable s) :
    ∀ x, x ∈ s.seen → x.declared = true :=
  (run_safe run).2.2

end Kv9.CatalogScope
