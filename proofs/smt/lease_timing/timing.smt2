; Successful Timing::new conversion and the composed safety inequalities.
(set-logic ALL)
(declare-const E Int)
(declare-const drift Int)
(declare-const margin Int)
(assert (and (<= 1 E) (<= E 18446744073709551615)))
(assert (and (<= 0 drift) (< drift 1000000000)))
(assert (and (<= 0 margin) (<= margin 18446744073709551615)))
(define-fun a () Int (- 1000000000 drift))
(define-fun b () Int (+ 1000000000 drift))
(define-fun leader () Int (- (div (* E a) b) margin))
(define-fun recovery () Int (+ (div (+ (* E b) (- a 1)) a) margin))
(assert (and (< 0 leader) (<= leader 18446744073709551615)))
(assert (and (<= 0 recovery) (<= recovery 18446744073709551615)))
; Checked dependency: floor.smt2 at n=E*a,d=b (both satisfy its premises).
(assert (<= (* (div (* E a) b) b) (* E a)))
; Checked dependency: ceil.smt2 at n=E*b,d=a (both satisfy its premises).
(assert (>= (* (div (+ (* E b) (- a 1)) a) a) (* E b)))
(assert (not (and
  (<= (* leader b) (* E a))
  (>= (* recovery a) (* E b))
  (<= (* E a) 340282366920938463463374607431768211455)
  (<= (+ (* E b) (- a 1) margin) 340282366920938463463374607431768211455))))
(check-sat)
