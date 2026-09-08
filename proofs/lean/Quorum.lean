import Std

namespace Kv9

/-- Count affirmative voters in an explicitly supplied membership list. -/
def countVotes (members : List α) (votes : α → Bool) : Nat :=
  match members with
  | [] => 0
  | voter :: rest => (if votes voter then 1 else 0) + countVotes rest votes

/-- Disjoint vote predicates cannot together cover more entries than membership. -/
theorem disjoint_votes_bound (members : List α) (left right : α → Bool)
    (disjoint : ∀ voter ∈ members, left voter = true → right voter = false) :
    countVotes members left + countVotes members right ≤ members.length := by
  induction members with
  | nil => simp [countVotes]
  | cons voter rest ih =>
    have tail : ∀ v ∈ rest, left v = true → right v = false := by
      intro v hv
      exact disjoint v (List.mem_cons_of_mem voter hv)
    have bound := ih tail
    have head := disjoint voter (List.mem_cons_self)
    cases hl : left voter <;> cases hr : right voter <;>
      simp_all [countVotes] <;> omega

/-- Any two strict majorities in the same membership share an affirmative voter. -/
theorem majority_intersection (members : List α) (left right : α → Bool)
    (hl : members.length < 2 * countVotes members left)
    (hr : members.length < 2 * countVotes members right) :
    ∃ voter ∈ members, left voter = true ∧ right voter = true := by
  apply Classical.byContradiction
  intro absent
  have disjoint : ∀ voter ∈ members, left voter = true → right voter = false := by
    intro voter hv hleft
    cases hright : right voter with
    | false => rfl
    | true => exact False.elim (absent ⟨voter, hv, hleft, hright⟩)
  have bound := disjoint_votes_bound members left right disjoint
  omega

/-- A fixed durable vote function cannot elect two candidates in one term. -/
theorem unique_election [DecidableEq Candidate] (members : List Voter)
    (vote : Voter → Option Candidate) (left right : Candidate)
    (hl : members.length < 2 * countVotes members (fun v => decide (vote v = some left)))
    (hr : members.length < 2 * countVotes members (fun v => decide (vote v = some right))) :
    left = right := by
  obtain ⟨voter, _, hleft, hright⟩ := majority_intersection members _ _ hl hr
  have l : vote voter = some left := of_decide_eq_true hleft
  have r : vote voter = some right := of_decide_eq_true hright
  exact Option.some.inj (l.symm.trans r)

/-- An odd group retains a strict majority after at most its fault budget fails. -/
theorem quorum_after_failures (faults unavailable : Nat) (h : unavailable ≤ faults) :
    (2 * faults + 1) / 2 + 1 ≤ (2 * faults + 1) - unavailable := by
  omega

/-- Removing any one voter leaves a majority in every group of at least three. -/
theorem no_single_voter_dependency (voters : Nat) (h : 3 ≤ voters) :
    voters / 2 + 1 ≤ voters - 1 := by
  omega

end Kv9
