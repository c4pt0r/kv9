import Std

namespace Kv9.DurableVote

/-- A projection of one voter's state onto a fixed term. `replies` is ghost
history retained by observers, not local storage. `published` means that the
complete directory ancestry is durable, rather than merely visible. -/
structure State (Candidate : Type) where
  memory : Option Candidate
  durable : Option Candidate
  published : Bool
  replies : List Candidate

def initial : State Candidate := ⟨none, none, false, []⟩

/-- Once a reply escaped, both its data and namespace must remain durable.
The memory clause prevents replacing an already durable vote in this term. -/
def Invariant (s : State Candidate) : Prop :=
  (∀ c, s.durable = some c → s.memory = some c) ∧
  (∀ c ∈ s.replies, s.durable = some c ∧ s.published = true)

inductive Step : State Candidate → State Candidate → Prop where
  | choose (s : State Candidate) (c : Candidate)
      (allowed : s.memory = none ∨ s.memory = some c) :
      Step s { s with memory := some c }
  | sync (s : State Candidate) : Step s { s with durable := s.memory }
  | publish (s : State Candidate) : Step s { s with published := true }
  | reply (s : State Candidate) (c : Candidate)
      (data : s.durable = some c) (name : s.published = true) :
      Step s { s with replies := c :: s.replies }
  /-- An unsynced name may survive or disappear. A published name must survive.
  A failed sync can take the `sync` step and then stop without sending a reply. -/
  | crash (s : State Candidate) (keep : Bool)
      (stable : s.published = true → keep = true) :
      Step s ⟨if keep then s.durable else none,
        if keep then s.durable else none, keep, s.replies⟩
  | stop (s : State Candidate) : Step s s

theorem initial_invariant : Invariant (initial : State Candidate) := by
  simp [Invariant, initial]

theorem step_preserves_invariant (step : Step s t) (inv : Invariant s) :
    Invariant t := by
  obtain ⟨memory, replies⟩ := inv
  cases step with
  | choose c allowed =>
    constructor
    · intro other durable
      have previous := memory other durable
      rcases allowed with empty | same
      · simp_all
      · simpa [same] using previous
    · exact replies
  | sync =>
    constructor
    · intro c same
      exact same
    · intro c member
      obtain ⟨data, name⟩ := replies c member
      exact ⟨memory c data, name⟩
  | publish =>
    constructor
    · exact memory
    · intro c member
      exact ⟨(replies c member).1, rfl⟩
  | reply c data name =>
    constructor
    · exact memory
    · intro other member
      simp only [List.mem_cons] at member
      rcases member with same | old
      · subst other
        exact ⟨data, name⟩
      · exact replies other old
  | crash keep stable =>
    constructor
    · intro c same
      exact same
    · intro c member
      obtain ⟨data, name⟩ := replies c member
      have kept := stable name
      simp [kept, data]
  | stop => exact ⟨memory, replies⟩

inductive Reachable : State Candidate → Prop where
  | initial : Reachable initial
  | next : Reachable s → Step s t → Reachable t

/-- Induction over an arbitrary finite history, including repeated crashes. -/
theorem reachable_invariant (reachable : Reachable s) : Invariant s := by
  induction reachable with
  | initial => exact initial_invariant
  | next _ step ih => exact step_preserves_invariant step ih

/-- Observers cannot receive conflicting votes in the fixed term. -/
theorem replies_agree (reachable : Reachable s)
    (left : a ∈ s.replies) (right : b ∈ s.replies) : a = b := by
  have inv := reachable_invariant reachable
  have ha := (inv.2 a left).1
  have hb := (inv.2 b right).1
  exact Option.some.inj (ha.symm.trans hb)

end Kv9.DurableVote
