; The existing refusal implies representability on 32-bit and 64-bit usize,
; safe header/body slices and an exact u32 body-length conversion.
(declare-const payload-length Int)
(assert (<= 0 payload-length))
(assert (< payload-length 67108864))
(define-fun body-length () Int (+ 1 payload-length))
(define-fun frame-length () Int (+ 8 body-length))
(assert (not (and (<= 1 body-length) (<= body-length 67108864)
                  (<= 9 frame-length) (< frame-length 4294967296)
                  (<= 8 frame-length) (<= 4 8)
                  (= (- frame-length 8) body-length))))
(check-sat)
