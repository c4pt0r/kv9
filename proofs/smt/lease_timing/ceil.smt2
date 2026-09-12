; The positive-integer formula used to specify Rust's div_ceil.
(set-logic ALL)
(declare-const n Int)
(declare-const d Int)
(assert (>= n 0))
(assert (> d 0))
(assert (not (>= (* (div (+ n (- d 1)) d) d) n)))
(check-sat)
