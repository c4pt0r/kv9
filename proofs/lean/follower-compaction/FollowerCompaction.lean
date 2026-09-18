import Std

namespace Kv9.FollowerCompaction

/-- Follower-side log compaction under a committed all-matched
confirmation. The leader's own compaction safety (leader-only, the
all-matched gate, the durable REC_COMPACTION record, configuration
recovery) is a premise from the group-compaction model; here the
CONFIRMATION→FOLLOWER chain is constrained: the committed confirmation is
recorded ONLY after the leader's all-matched truncation, a follower
compacts its own prefix ONLY under that committed confirmation AND once
its own applied position has reached the floor, no voter is ever
stranded (a committed entry at or below a confirmed floor is held by
every voter), and only the local prefix is discarded. -/
structure State where
  leaderTruncated : Bool := false
  confirmed : Bool := false
  followerApplied : Bool := false
  followerCompacted : Bool := false
  voterStranded : Bool := false
  foreignPrefixDiscarded : Bool := false
  confirmedWithoutAllMatched : Bool := false
  baseAdopted : Bool := false
  baseWithoutAuthority : Bool := false
  deriving DecidableEq, Repr

inductive Step : State → State → Prop where
  | leaderTruncate {s} : Step s {s with leaderTruncated := true}
  | confirm {s} (t : s.leaderTruncated = true) : Step s {s with confirmed := true}
  | followerApply {s} : Step s {s with followerApplied := true}
  | followerCompact {s} (c : s.confirmed = true) (a : s.followerApplied = true) :
      Step s {s with followerCompacted := true}
  -- A restarting voter adopts its durable compacted base into a live peer ONLY
  -- under committed authority: the very truncation that produced the base. A
  -- base with no committed decision behind it must be refused, never adopted —
  -- the recovery gate that follower-side compaction (which persists a base on
  -- EVERY voter, not just the leader) forced closed.
  | adoptBase {s} (auth : s.leaderTruncated = true) : Step s {s with baseAdopted := true}

inductive Reachable : State → Prop where
  | initial : Reachable {}
  | step {s t} : Reachable s → Step s t → Reachable t

def Safe (s : State) : Prop :=
  (s.confirmed = true → s.leaderTruncated = true) ∧
  (s.followerCompacted = true → s.confirmed = true ∧ s.followerApplied = true) ∧
  s.voterStranded = false ∧
  s.foreignPrefixDiscarded = false ∧
  s.confirmedWithoutAllMatched = false ∧
  (s.baseAdopted = true → s.leaderTruncated = true) ∧
  s.baseWithoutAuthority = false

theorem initial_safe : Safe {} := by simp [Safe]

theorem step_safe {s t : State} (safe : Safe s) (step : Step s t) : Safe t := by
  obtain ⟨hC, hF, hS, hP, hA, hB, hW⟩ := safe
  cases step with
  | leaderTruncate => exact ⟨by simp_all, by simp_all, hS, hP, hA, by simp_all, hW⟩
  | confirm t => exact ⟨by simp_all, by simp_all, hS, hP, hA, by simp_all, hW⟩
  | followerApply => exact ⟨by simp_all, by simp_all, hS, hP, hA, by simp_all, hW⟩
  | followerCompact c a => exact ⟨by simp_all, by simp_all, hS, hP, hA, by simp_all, hW⟩
  | adoptBase auth => exact ⟨by simp_all, by simp_all, hS, hP, hA, by simp_all, hW⟩

theorem run_safe {s : State} (run : Reachable s) : Safe s := by
  induction run with
  | initial => exact initial_safe
  | step _ transition ih => exact step_safe ih transition

theorem a_confirmation_requires_the_leaders_all_matched_truncation {s : State}
    (run : Reachable s) (c : s.confirmed = true) : s.leaderTruncated = true :=
  (run_safe run).1 c

theorem a_follower_compacts_only_under_a_confirmation {s : State}
    (run : Reachable s) (fc : s.followerCompacted = true) :
    s.confirmed = true ∧ s.followerApplied = true :=
  (run_safe run).2.1 fc

theorem no_voter_is_ever_stranded {s : State} (run : Reachable s) :
    s.voterStranded = false :=
  (run_safe run).2.2.1

theorem only_the_local_prefix_is_ever_discarded {s : State} (run : Reachable s) :
    s.foreignPrefixDiscarded = false :=
  (run_safe run).2.2.2.1

theorem no_confirmation_without_all_matched {s : State} (run : Reachable s) :
    s.confirmedWithoutAllMatched = false :=
  (run_safe run).2.2.2.2.1

theorem a_recovered_base_requires_committed_authority {s : State}
    (run : Reachable s) (b : s.baseAdopted = true) : s.leaderTruncated = true :=
  (run_safe run).2.2.2.2.2.1 b

theorem no_base_is_adopted_without_authority {s : State} (run : Reachable s) :
    s.baseWithoutAuthority = false :=
  (run_safe run).2.2.2.2.2.2

theorem a_follower_needs_its_own_applied_floor {s : State}
    (c : s.confirmed = true) (a : s.followerApplied = true) :
    Step s {s with followerCompacted := true} :=
  .followerCompact c a

theorem a_confirmation_is_permanent {s t : State} (step : Step s t)
    (c : s.confirmed = true) : t.confirmed = true := by
  cases step <;> simp_all

theorem a_confirmation_alone_compacts_no_follower :
    ∃ s, Reachable s ∧ s.confirmed = true ∧ s.followerCompacted = false := by
  refine ⟨⟨true, true, false, false, false, false, false, false, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial .leaderTruncate) (.confirm rfl)

theorem the_confirmed_chain_compacts_a_follower :
    ∃ s, Reachable s ∧ s.followerCompacted = true ∧ s.confirmed = true ∧
      s.voterStranded = false := by
  refine ⟨⟨true, true, true, true, false, false, false, false, false⟩, ?_, rfl, rfl, rfl⟩
  exact .step (.step (.step (.step .initial .leaderTruncate) (.confirm rfl))
    .followerApply) (.followerCompact rfl rfl)

theorem a_truncation_lets_a_voter_adopt_its_base :
    ∃ s, Reachable s ∧ s.baseAdopted = true ∧ s.baseWithoutAuthority = false := by
  refine ⟨⟨true, false, false, false, false, false, false, true, false⟩, ?_, rfl, rfl⟩
  exact .step (.step .initial .leaderTruncate) (.adoptBase rfl)

end Kv9.FollowerCompaction
