; One loop step. Bytes already emitted are frame[0..k); a successful
; Write::write_vectored emits exactly the next n bytes of the concatenation.
; The byte index i is arbitrary, so the query covers the entire new prefix.
(set-logic QF_AUFLIA)
(declare-const frame (Array Int Int))
(declare-const length Int)
(declare-const k Int)
(declare-const n Int)
(declare-const i Int)
(assert (and (<= 0 k) (< k length) (< 0 n) (<= n (- length k))))
(assert (and (<= 0 i) (< i (+ k n))))
(define-fun emitted () Int
  (ite (< i k) (select frame i) (select frame (+ k (- i k)))))
(assert (distinct emitted (select frame i)))
(check-sat)
