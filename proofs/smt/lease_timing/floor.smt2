; For nonnegative n and positive d, integer division rounds downward.
(set-logic ALL)
(declare-const n Int)
(declare-const d Int)
(assert (>= n 0))
(assert (> d 0))
(assert (not (<= (* (div n d) d) n)))
(check-sat)
