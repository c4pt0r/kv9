(* automatically generated -- do not edit manually *)
theory AppliedReceiptsProof imports Constant Zenon begin
ML_command \<open> writeln ("*** TLAPS PARSED\n"); \<close>
consts
  "isReal" :: c
  "isa_slas_a" :: "[c,c] => c"
  "isa_bksl_diva" :: "[c,c] => c"
  "isa_perc_a" :: "[c,c] => c"
  "isa_peri_peri_a" :: "[c,c] => c"
  "isInfinity" :: c
  "isa_lbrk_rbrk_a" :: "[c] => c"
  "isa_less_more_a" :: "[c] => c"

lemma ob'84:
fixes a_CONSTANTunde_ARCapacityunde_a
fixes a_CONSTANTunde_ARIndexesunde_a
fixes a_CONSTANTunde_ARTermsunde_a
fixes a_CONSTANTunde_AROutcomesunde_a
(* usable definition CONSTANT_ARLegalInputs_ suppressed *)
(* usable definition CONSTANT_ARReceipts_ suppressed *)
(* usable definition CONSTANT_ARVectorPush_ suppressed *)
(* usable definition CONSTANT_ARDequePush_ suppressed *)
(* usable definition CONSTANT_ARStrict_ suppressed *)
(* usable definition CONSTANT_ARCertificate_ suppressed *)
(* usable definition CONSTANT_ARMatches_ suppressed *)
(* usable definition CONSTANT_ARLookup_ suppressed *)
fixes a_VARIABLEunde_arVectorunde_a a_VARIABLEunde_arVectorunde_a'
fixes a_VARIABLEunde_arDequeunde_a a_VARIABLEunde_arDequeunde_a'
fixes a_VARIABLEunde_arOrderedunde_a a_VARIABLEunde_arOrderedunde_a'
(* usable definition STATE_arVars_ suppressed *)
(* usable definition STATE_ARType_ suppressed *)
(* usable definition STATE_ARRefinement_ suppressed *)
(* usable definition STATE_AROrdering_ suppressed *)
(* usable definition STATE_ARInvariant_ suppressed *)
(* usable definition STATE_ARInit_ suppressed *)
(* usable definition ACTION_ARPush_ suppressed *)
(* usable definition ACTION_ARNext_ suppressed *)
(* usable definition TEMPORAL_ARSpec_ suppressed *)
(* usable definition STATE_ARLookupRefinement_ suppressed *)
(* usable definition CONSTANT_EnabledWrapper_ suppressed *)
(* usable definition CONSTANT_CdotWrapper_ suppressed *)
fixes a_CONSTANTunde_sunde_a
assumes a_CONSTANTunde_sunde_a_in : "(a_CONSTANTunde_sunde_a \<in> ((Seq ((a_CONSTANTunde_ARReceiptsunde_a)))))"
assumes v'37: "((a_CONSTANTunde_ARStrictunde_a ((a_CONSTANTunde_sunde_a))))"
fixes a_CONSTANTunde_keyunde_a
assumes a_CONSTANTunde_keyunde_a_in : "(a_CONSTANTunde_keyunde_a \<in> (a_CONSTANTunde_ARIndexesunde_a))"
fixes a_CONSTANTunde_iunde_a
assumes a_CONSTANTunde_iunde_a_in : "(a_CONSTANTunde_iunde_a \<in> ((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))))"
assumes v'51: "((((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))) \<noteq> ({})))"
assumes v'52: "(((bChoice(((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))), %a_CONSTANTunde_junde_a. (TRUE))) = (a_CONSTANTunde_iunde_a)))"
assumes v'53: "(((bChoice(((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))), %a_CONSTANTunde_junde_a. (\<forall> a_CONSTANTunde_kunde_a \<in> ((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))) : ((leq ((a_CONSTANTunde_junde_a), (a_CONSTANTunde_kunde_a))))))) = (a_CONSTANTunde_iunde_a)))"
shows "(((cond(((((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))) = ({}))), (((''found'' :> (FALSE)))), (((''found'' :> (TRUE)) @@ (''receipt'' :> (fapply ((a_CONSTANTunde_sunde_a), (bChoice(((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))), %a_CONSTANTunde_iunde_a_1. (TRUE)))))))))) = (cond(((((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))) = ({}))), (((''found'' :> (FALSE)))), (((''found'' :> (TRUE)) @@ (''receipt'' :> (fapply ((a_CONSTANTunde_sunde_a), (bChoice(((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))), %a_CONSTANTunde_iunde_a_1. (\<forall> a_CONSTANTunde_junde_a \<in> ((a_CONSTANTunde_ARMatchesunde_a ((a_CONSTANTunde_sunde_a), (a_CONSTANTunde_keyunde_a)))) : ((leq ((a_CONSTANTunde_iunde_a_1), (a_CONSTANTunde_junde_a))))))))))))))))"(is "PROP ?ob'84")
proof -
ML_command \<open> writeln "*** TLAPS ENTER 84"; \<close>
show "PROP ?ob'84"

(* BEGIN ZENON INPUT
;; file=/tmp/kv9-receipt-tail-hint-proof-20260914-first/theorems/cache-AppliedReceiptsProof/AppliedReceiptsProof.tlaps/tlapm_a82664.znn; PATH='/tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/lib/tlapm/backends/bin:/tmp/kv9-p0-tools/tlapm-1.6.0-pre-20260731/lib/tlapm/backends/Isabelle/bin:/home/dongxu/.bun/install/global/node_modules/@openai/codex-linux-x64/vendor/x86_64-unknown-linux-musl/codex-path:/home/dongxu/.codex/tmp/arg0/codex-arg07f82JK:/home/dongxu/.bun/bin:/home/dongxu/.opencode/bin:/opt/homebrew/opt/mysql-client/bin:/Users/dongxu/Library/Python/3.9/bin:/home/dongxu/.local/bin:/home/dongxu/.deno/bin:/opt/homebrew/opt/mysql-client/bin:/home/dongxu/.tiup/bin:/home/dongxu/.local/share/pnpm:/home/dongxu/.bun/bin:/home/dongxu/.opencode/bin:/home/dongxu/.nvm/versions/node/v22.22.0/bin:/opt/homebrew/opt/mysql-client/bin:/Users/dongxu/Library/Python/3.9/bin:/home/dongxu/.local/bin:/home/dongxu/.deno/bin:/opt/homebrew/opt/mysql-client/bin:/home/dongxu/.tiup/bin:/home/dongxu/.cargo/bin:/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin:/usr/games:/usr/local/games:/snap/bin:/home/dongxu/Library/Python/3.8/bin:/Users/dongxu/.bin:/home/dongxu/.cargo/bin:/Users/dongxu/.local/bin:/home/dongxu/.tip/bin:/home/dongxu/google-cloud-sdk/bin:/home/dongxu/Library/Python/3.8/bin:/Users/dongxu/.bin:/home/dongxu/.cargo/bin:/Users/dongxu/.local/bin:/home/dongxu/.tip/bin:/home/dongxu/google-cloud-sdk/bin'; zenon -p0 -x tla -oisar -max-time 1d "$file" >/tmp/kv9-receipt-tail-hint-proof-20260914-first/theorems/cache-AppliedReceiptsProof/AppliedReceiptsProof.tlaps/tlapm_a82664.znn.out
;; obligation #84
$hyp "a_CONSTANTunde_sunde_a_in" (TLA.in a_CONSTANTunde_sunde_a (TLA.Seq a_CONSTANTunde_ARReceiptsunde_a))
$hyp "v'37" (a_CONSTANTunde_ARStrictunde_a a_CONSTANTunde_sunde_a)
$hyp "a_CONSTANTunde_keyunde_a_in" (TLA.in a_CONSTANTunde_keyunde_a a_CONSTANTunde_ARIndexesunde_a)
$hyp "a_CONSTANTunde_iunde_a_in" (TLA.in a_CONSTANTunde_iunde_a (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a))
$hyp "v'51" (-. (= (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a)
TLA.emptyset))
$hyp "v'52" (= (TLA.bChoice (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_junde_a) T.))
a_CONSTANTunde_iunde_a)
$hyp "v'53" (= (TLA.bChoice (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_junde_a) (TLA.bAll (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_kunde_a) (arith.le a_CONSTANTunde_junde_a
a_CONSTANTunde_kunde_a)))))
a_CONSTANTunde_iunde_a)
$goal (= (TLA.cond (= (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a)
TLA.emptyset) (TLA.record "found" F.) (TLA.record "found" T. "receipt" (TLA.fapply a_CONSTANTunde_sunde_a (TLA.bChoice (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_iunde_a_1) T.)))))
(TLA.cond (= (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a)
TLA.emptyset) (TLA.record "found" F.) (TLA.record "found" T. "receipt" (TLA.fapply a_CONSTANTunde_sunde_a (TLA.bChoice (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_iunde_a_1) (TLA.bAll (a_CONSTANTunde_ARMatchesunde_a a_CONSTANTunde_sunde_a
a_CONSTANTunde_keyunde_a) ((a_CONSTANTunde_junde_a) (arith.le a_CONSTANTunde_iunde_a_1
a_CONSTANTunde_junde_a)))))))))
END ZENON  INPUT *)
(* PROOF-FOUND *)
(* BEGIN-PROOF *)
proof (rule zenon_nnpp)
 have z_Hf:"(bChoice(a_CONSTANTunde_ARMatchesunde_a(a_CONSTANTunde_sunde_a, a_CONSTANTunde_keyunde_a), (\<lambda>a_CONSTANTunde_junde_a. TRUE))=a_CONSTANTunde_iunde_a)" (is "?z_hi=_")
 using v'52 by blast
 have z_He:"(a_CONSTANTunde_ARMatchesunde_a(a_CONSTANTunde_sunde_a, a_CONSTANTunde_keyunde_a)~={})" (is "?z_hj~=_")
 using v'51 by blast
 have z_Hg:"(bChoice(?z_hj, (\<lambda>a_CONSTANTunde_junde_a. bAll(?z_hj, (\<lambda>a_CONSTANTunde_kunde_a. (a_CONSTANTunde_junde_a <= a_CONSTANTunde_kunde_a)))))=a_CONSTANTunde_iunde_a)" (is "?z_hq=_")
 using v'53 by blast
 assume z_Hh:"(cond((?z_hj={}), (''found'' :> (FALSE)), (''found'' :> (TRUE) @@ ''receipt'' :> ((a_CONSTANTunde_sunde_a[?z_hi]))))~=cond((?z_hj={}), (''found'' :> (FALSE)), (''found'' :> (TRUE) @@ ''receipt'' :> ((a_CONSTANTunde_sunde_a[?z_hq])))))" (is "?z_hx~=?z_hbf")
 have z_Hbi_z_Hf: "((CHOOSE x:((x \\in ?z_hj)&TRUE))=a_CONSTANTunde_iunde_a) == (?z_hi=a_CONSTANTunde_iunde_a)" (is "?z_hbi == ?z_hf")
 by (unfold bChoose_def)
 have z_Hbi: "?z_hbi" (is "?z_hbj=_")
 by (unfold z_Hbi_z_Hf, fact z_Hf)
 have z_Hbn_z_Hg: "((CHOOSE x:((x \\in ?z_hj)&bAll(?z_hj, (\<lambda>a_CONSTANTunde_kunde_a. (x <= a_CONSTANTunde_kunde_a)))))=a_CONSTANTunde_iunde_a) == (?z_hq=a_CONSTANTunde_iunde_a)" (is "?z_hbn == ?z_hg")
 by (unfold bChoose_def)
 have z_Hbn: "?z_hbn" (is "?z_hbo=_")
 by (unfold z_Hbn_z_Hg, fact z_Hg)
 show FALSE
 proof (rule zenon_ifthenelse [of "(\<lambda>zenon_Vf. (zenon_Vf~=?z_hbf))" "(?z_hj={})" "(''found'' :> (FALSE))" "(''found'' :> (TRUE) @@ ''receipt'' :> ((a_CONSTANTunde_sunde_a[?z_hi])))", OF z_Hh])
  assume z_Hy:"(?z_hj={})"
  assume z_Hbw:"((''found'' :> (FALSE))~=?z_hbf)" (is "?z_hz~=_")
  show FALSE
  by (rule notE [OF z_He z_Hy])
 next
  assume z_He:"(?z_hj~={})"
  assume z_Hbx:"((''found'' :> (TRUE) @@ ''receipt'' :> ((a_CONSTANTunde_sunde_a[?z_hi])))~=?z_hbf)" (is "?z_hbc~=_")
  have z_Hby_z_Hbx: "((''found'' :> (TRUE) @@ ''receipt'' :> ((a_CONSTANTunde_sunde_a[?z_hbj])))~=?z_hbf) == (?z_hbc~=?z_hbf)" (is "?z_hby == ?z_hbx")
  by (unfold bChoose_def)
  have z_Hby: "?z_hby" (is "?z_hbz~=_")
  by (unfold z_Hby_z_Hbx, fact z_Hbx)
  sorry
 qed
qed
(* END-PROOF *)
ML_command \<open> writeln "*** TLAPS EXIT 84"; \<close> qed
end
