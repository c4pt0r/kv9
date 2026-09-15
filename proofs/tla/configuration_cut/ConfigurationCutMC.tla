-------------------------- MODULE ConfigurationCutMC --------------------------
EXTENDS ConfigurationCut
SmallTerms == [i \in 1..HCLimit |-> IF i = 1 THEN 3 ELSE 4]
\* Every bounded stable inspection base satisfies the parameterized invariant.
SmallInspectInit == /\ hcDurable \in SUBSET HCChanges /\ hcApplied = hcDurable
                    /\ hcPending = 0 /\ hcWriter \in BOOLEAN /\ hcKnown \in BOOLEAN
                    /\ hcResult = "Idle" /\ hcCut = 0 /\ hcSelected = 0 /\ hcClaim = 0
                    /\ hcReadWriter = FALSE /\ hcReadKnown = FALSE
SmallInspectSpec == SmallInspectInit /\ [][HCQueries]_hcVars /\ WF_hcVars(HCQueries)
=============================================================================
