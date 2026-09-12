; Real-time Lemma 1 from docs/LEADER-LEASE-PROOF.md.
; Free constants are arbitrary: unsatisfiability proves the implication
; for every real assignment meeting the premises, not a finite sample.
; D*b <= E*a avoids division and is equivalent because b > 0.
(set-logic QF_NRA)
(declare-const a Real)
(declare-const b Real)
(declare-const E Real)
(declare-const D Real)
(declare-const s Real)
(declare-const g Real)
(declare-const t Real)
(declare-const leader_elapsed Real)
(declare-const voter_elapsed Real)
(assert (> a 0))
(assert (>= b a))
(assert (> E 0))
(assert (> D 0))
(assert (<= (* D b) (* E a)))
(assert (<= s g))
(assert (<= g t))
(assert (>= leader_elapsed (* a (- t s))))
(assert (< leader_elapsed D))
(assert (<= voter_elapsed (* b (- t g))))
; Negated conclusion: a grantor has expired despite the leader's check.
(assert (>= voter_elapsed E))
(check-sat)
