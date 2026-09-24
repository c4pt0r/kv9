import Std

namespace Kv9.WriteBackpressure

/-- End-to-end write backpressure (#20: bounded absolute log size).

A data group's RETAINED committed raft log (`raft_committed - log_first_index`,
`crates/server/src/runtime/range_api.rs`) is bounded by an absolute cap
(`KV9_MAX_RAFT_LOG_ENTRIES`). The gate is PRE-APPEND and WRITE-ONLY: a write is
admitted only while the retained length is STRICTLY below the cap; at or beyond
the cap every write is refused with a TYPED, retryable refusal
(`Error::WriteBackpressure` → `RESOURCE_EXHAUSTED`) that proposes nothing and
changes nothing. Reads are never gated (they take `view`, never `permit`). A
committed compaction floor DRAINS the log and reopens writes — backpressure
releases, it is not a permanent wedge.

The engine's own accounting (a committed append advances `raft_committed` by one;
a compaction floor raises `log_first_index`) is a premise from the storage layer
and its tests. This model constrains the ABSTRACT retained length under those
transitions and proves the gate keeps it bounded, refuses without mutation, never
gates reads, and always releases on drain. -/
structure State where
  retained : Nat := 0
  deriving DecidableEq, Repr

/-- One admissible transition against an absolute cap `cap`. -/
inductive Step (cap : Nat) : State → State → Prop where
  /-- A write is admitted (its committed entry lengthens the retained log by
  one) ONLY while strictly below the cap — the pre-append gate. -/
  | admitWrite {s} (below : s.retained < cap) :
      Step cap s {s with retained := s.retained + 1}
  /-- At or beyond the cap a write is refused with the typed refusal: the state
  is unchanged (nothing proposed, nothing queued). -/
  | refuseTyped {s} (full : cap ≤ s.retained) : Step cap s s
  /-- A read is served at ANY retention (reads are never gated) and changes
  nothing. -/
  | read {s} : Step cap s s
  /-- A committed compaction floor drains up to `k` retained entries. -/
  | drain {s} (k : Nat) : Step cap s {s with retained := s.retained - k}

inductive Reachable (cap : Nat) : State → Prop where
  | initial : Reachable cap {}
  | step {s t} : Reachable cap s → Step cap s t → Reachable cap t

/-- The retained log never exceeds the absolute cap. -/
def Safe (cap : Nat) (s : State) : Prop := s.retained ≤ cap

theorem initial_safe {cap : Nat} : Safe cap {} := by simp [Safe]

theorem step_safe {cap : Nat} {s t : State} (safe : Safe cap s)
    (step : Step cap s t) : Safe cap t := by
  cases step with
  | admitWrite below => simp [Safe] at *; omega
  | refuseTyped full => exact safe
  | read => exact safe
  | drain k => simp [Safe] at *; omega

theorem run_safe {cap : Nat} {s : State} (run : Reachable cap s) : Safe cap s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

/-- THE headline bound: the retained committed log of any reachable state never
exceeds the absolute cap. -/
theorem retained_never_exceeds_the_bound {cap : Nat} {s : State}
    (run : Reachable cap s) : s.retained ≤ cap := run_safe run

/-- Whenever a step grows the retained log, the pre-condition was that it was
strictly below the cap — writes are admitted only below the bound. -/
theorem an_admitted_write_was_below_the_bound {cap : Nat} {s t : State}
    (step : Step cap s t) (grew : s.retained < t.retained) : s.retained < cap := by
  cases step with
  | admitWrite below => exact below
  | refuseTyped _ => exact absurd grew (Nat.lt_irrefl _)
  | read => exact absurd grew (Nat.lt_irrefl _)
  | drain k => exact absurd grew (Nat.not_lt.2 (Nat.sub_le _ _))

/-- No write grows the log at or beyond the bound: any step from a full state
leaves the retained log no larger than it was. -/
theorem no_write_grows_the_log_at_the_bound {cap : Nat} {s t : State}
    (full : cap ≤ s.retained) (step : Step cap s t) : t.retained ≤ s.retained := by
  cases step with
  | admitWrite below => omega
  | refuseTyped _ => exact Nat.le_refl _
  | read => exact Nat.le_refl _
  | drain k => exact Nat.sub_le _ _

/-- At or beyond the bound the typed refusal is always available and it maps the
state to itself — nothing is proposed, nothing changes. -/
theorem a_full_log_refuses_writes_typed {cap : Nat} {s : State}
    (full : cap ≤ s.retained) : Step cap s s := .refuseTyped full

/-- Reads are never gated: the read transition is available at ANY retention,
including at or beyond the bound, and it leaves the state unchanged. -/
theorem reads_serve_at_any_retention {cap : Nat} (s : State) : Step cap s s := .read

/-- Release is not a wedge: from any state at the bound, a single-entry drain
lowers the retained log strictly below the cap, after which a write is admissible
again. -/
theorem draining_releases_backpressure {cap : Nat} (pos : 0 < cap)
    (s : State) (atBound : s.retained = cap) :
    ∃ t, Step cap s t ∧ t.retained < cap ∧ ∃ u, Step cap t u := by
  refine ⟨{s with retained := s.retained - 1}, .drain 1, ?_, ?_⟩
  · show s.retained - 1 < cap
    omega
  · exact ⟨{s with retained := s.retained - 1 + 1}, .admitWrite (by show s.retained - 1 < cap; omega)⟩

/-- Non-vacuity: any retention up to the cap is reachable. -/
theorem reachable_at (cap : Nat) : ∀ n, n ≤ cap → Reachable cap ⟨n⟩ := by
  intro n
  induction n with
  | zero => intro _; exact .initial
  | succ m ih => intro h; exact .step (ih (by omega)) (.admitWrite (by show m < cap; omega))

/-- The retained log can actually fill to the bound (the safety bound is tight,
not vacuously satisfied by an unreachable ceiling). -/
theorem the_log_can_fill_to_the_bound {cap : Nat} :
    ∃ s, Reachable cap s ∧ s.retained = cap :=
  ⟨⟨cap⟩, reachable_at cap cap (Nat.le_refl cap), rfl⟩

end Kv9.WriteBackpressure
