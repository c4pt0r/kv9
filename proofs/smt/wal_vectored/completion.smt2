; The frame is header || payload || checksum. Empty remaining buffers imply
; the complete concatenation has been emitted, including the checksum.
(set-logic QF_LIA)
(declare-const header Int)
(declare-const payload Int)
(declare-const checksum Int)
(declare-const k Int)
(assert (and (<= 0 header) (<= 0 payload) (< 0 checksum)))
(define-fun length () Int (+ header payload checksum))
(assert (and (<= 0 k) (<= k length) (= (- length k) 0)))
(assert (distinct k (+ header payload checksum)))
(check-sat)
