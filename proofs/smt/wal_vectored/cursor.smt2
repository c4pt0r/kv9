; The same successful write advances the remaining IoSlice suffix by n.
; Progress is strict, bounded by the frame length, and cannot skip a byte.
(set-logic QF_LIA)
(declare-const length Int)
(declare-const k Int)
(declare-const n Int)
(declare-const i Int)
(assert (and (<= 0 k) (< k length) (< 0 n) (<= n (- length k))))
(define-fun next () Int (+ k n))
(assert (or (<= next k) (> next length)
  (and (<= 0 i) (< i (- length next))
    (distinct (+ next i) (+ k n i)))))
(check-sat)
