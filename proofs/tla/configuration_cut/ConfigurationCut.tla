--------------------------- MODULE ConfigurationCut ---------------------------
EXTENDS Naturals
CONSTANT HCLimit, HCChanges, HCTerms
ASSUME HCInputs == HCLimit \in Nat \ {0}
                   /\ HCChanges \subseteq 1..HCLimit
                   /\ HCTerms \in [1..HCLimit -> Nat \ {0}]
\* The retained committed prefix and the full configuration payload at each
\* indexed publication are trusted storage/Raft interfaces. This module proves
\* selection and publication ordering, not upstream membership transitions.
VARIABLES hcDurable, hcApplied, hcPending, hcWriter, hcKnown,
          hcResult, hcCut, hcSelected, hcClaim, hcReadWriter, hcReadKnown
hcView == <<hcResult, hcCut, hcSelected, hcClaim, hcReadWriter, hcReadKnown>>
hcVars == <<hcDurable, hcApplied, hcPending, hcWriter, hcKnown,
            hcResult, hcCut, hcSelected, hcClaim, hcReadWriter, hcReadKnown>>
HCInit == /\ hcDurable = {} /\ hcApplied = {} /\ hcPending = 0
          /\ hcWriter = TRUE /\ hcKnown = TRUE /\ hcResult = "Idle"
          /\ hcCut = 0 /\ hcSelected = 0 /\ hcClaim = 0
          /\ hcReadWriter = FALSE /\ hcReadKnown = FALSE
HCStage(i) == /\ hcWriter /\ hcPending = 0 /\ i \notin hcApplied
              /\ hcPending' = i
              /\ UNCHANGED <<hcDurable, hcApplied, hcWriter, hcKnown, hcView>>
HCSync == /\ hcWriter /\ hcPending # 0
          /\ hcDurable' = hcDurable \cup {hcPending}
          /\ UNCHANGED <<hcApplied, hcPending, hcWriter, hcKnown, hcView>>
HCPublish == /\ hcWriter /\ hcPending # 0 /\ hcPending \in hcDurable
             /\ hcApplied' = hcApplied \cup {hcPending} /\ hcPending' = 0
             /\ UNCHANGED <<hcDurable, hcWriter, hcKnown, hcView>>
HCFail == /\ hcWriter /\ hcWriter' = FALSE /\ hcPending' = 0
          /\ hcDurable' \in IF hcPending = 0 THEN {hcDurable}
                           ELSE {hcDurable, hcDurable \cup {hcPending}}
          /\ UNCHANGED <<hcApplied, hcKnown, hcView>>
HCRecover == /\ ~hcWriter /\ hcWriter' = TRUE /\ hcApplied' = hcDurable /\ hcPending' = 0
             /\ UNCHANGED <<hcDurable, hcKnown, hcView>>
HCAmbiguous == /\ hcKnown /\ hcKnown' = FALSE
               /\ UNCHANGED <<hcDurable, hcApplied, hcPending, hcWriter, hcView>>
HCLatest(q, s) == /\ s \in {0} \cup HCChanges /\ s <= q
                  /\ \A i \in HCChanges : i <= q => i <= s
HCQuery(q, s, claim) ==
    /\ hcPending = 0 /\ s \in {0} \cup hcApplied /\ s <= q
    /\ hcResult' = IF ~hcWriter THEN "Invalid"
                    ELSE IF claim # HCTerms[q] THEN "Invalid"
                    ELSE IF ~hcKnown THEN "Unavailable"
                    ELSE IF s # 0 /\ s \notin HCChanges THEN "Invalid"
                    ELSE IF s # 0 /\ HCTerms[s] > HCTerms[q] THEN "Invalid"
                    ELSE IF \E i \in HCChanges : s < i /\ i <= q THEN "Unavailable"
                    ELSE "Found"
    /\ hcCut' = q /\ hcSelected' = s /\ hcClaim' = claim
    /\ hcReadWriter' = hcWriter /\ hcReadKnown' = hcKnown
    /\ UNCHANGED <<hcDurable, hcApplied, hcPending, hcWriter, hcKnown>>
HCQueries == \E q \in 1..HCLimit : \E s \in 0..HCLimit, claim \in {HCTerms[q], HCTerms[q] + 1} : HCQuery(q, s, claim)
HCNext == (\E i \in 1..HCLimit : HCStage(i)) \/ HCSync \/ HCPublish
          \/ HCFail \/ HCRecover \/ HCAmbiguous \/ HCQueries
HCSpec == HCInit /\ [][HCNext]_hcVars
HCType == /\ hcApplied \subseteq hcDurable /\ hcDurable \subseteq 1..HCLimit
          /\ hcPending \in 0..HCLimit /\ hcWriter \in BOOLEAN /\ hcKnown \in BOOLEAN
          /\ hcResult \in {"Idle", "Found", "Unavailable", "Invalid"}
          /\ hcCut \in 0..HCLimit /\ hcSelected \in 0..HCLimit /\ hcClaim \in Nat
          /\ hcReadWriter \in BOOLEAN /\ hcReadKnown \in BOOLEAN
HCFound == hcResult = "Found" =>
             /\ hcReadWriter /\ hcReadKnown /\ hcCut \in 1..HCLimit
             /\ hcSelected \in {0} \cup hcDurable /\ HCLatest(hcCut, hcSelected)
             /\ hcClaim = HCTerms[hcCut]
             /\ (hcSelected # 0 => HCTerms[hcSelected] <= hcClaim)
HCInvariant == HCType /\ HCFound
\* Recovery-only termination once the caller can inspect a stable local view;
\* no starvation-freedom or online latency bound under competing writers.
HCInspectSpec == HCInvariant /\ hcPending = 0 /\ hcResult = "Idle"
                 /\ [][HCQueries]_hcVars /\ WF_hcVars(HCQueries)
HCInspectProgress == <>(hcResult # "Idle")
=============================================================================
