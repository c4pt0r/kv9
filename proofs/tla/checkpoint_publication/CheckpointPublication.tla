------------------------- MODULE CheckpointPublication -------------------------
EXTENDS Naturals
CONSTANTS CPLimit, CPImages, CPImage, CPPred, CPCut, CPTerm, CPReportedTerm,
          CPObservedImage, CPFresh, CPSelected, CPFinalHistoryMatches
ASSUME CPInputs == /\ CPLimit \in Nat \ {0} /\ CPImages # {}
                   /\ CPImage \in [1..CPLimit -> CPImages]
                   /\ CPPred \in [1..CPLimit -> 0..CPLimit]
                   /\ CPCut \in [CPImages -> 0..CPLimit]
                   /\ CPTerm \in [1..CPLimit -> Nat \ {0}]
                   /\ CPReportedTerm \in [1..CPLimit -> Nat \ {0}]
                   /\ CPObservedImage \in [1..CPLimit -> CPImages]
                   /\ CPFresh \in [1..CPLimit -> BOOLEAN]
                   /\ CPSelected \in CPImages /\ CPFinalHistoryMatches \in BOOLEAN
\* Committed command identities and atomic durable apply batches are existing
\* Raft/WAL premises. This module composes CAS winner creation with startup
\* observation; no descriptor presence or duplicate proposal is a winner.
VARIABLES cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
          cpPhase, cpCursor, cpSeen, cpGrantedAfterComplete
cpVars == <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
            cpPhase, cpCursor, cpSeen, cpGrantedAfterComplete>>
CPInit == /\ cpApplied = 0 /\ cpGeneration = 0 /\ cpWinners = {}
          /\ cpWinnerGeneration = [i \in 1..CPLimit |-> 0]
          /\ cpPhase = "write" /\ cpCursor = 0 /\ cpSeen = 0
          /\ cpGrantedAfterComplete = FALSE
CPWins(i) == CPPred[i] = cpGeneration /\ CPCut[CPImage[i]] < i /\ CPFresh[i]
CPApply == /\ cpPhase = "write" /\ cpApplied < CPLimit
           /\ LET i == cpApplied + 1 IN
                 /\ cpApplied' = i
                 /\ cpGeneration' = IF CPWins(i) THEN cpGeneration + 1 ELSE cpGeneration
                 /\ cpWinners' = IF CPWins(i) THEN cpWinners \cup {i} ELSE cpWinners
                 /\ cpWinnerGeneration' = IF CPWins(i)
                       THEN [cpWinnerGeneration EXCEPT ![i] = cpGeneration + 1]
                       ELSE cpWinnerGeneration
           /\ UNCHANGED <<cpPhase, cpCursor, cpSeen, cpGrantedAfterComplete>>
CPBegin == /\ cpPhase = "write" /\ CPCut[CPSelected] <= cpApplied
           /\ cpPhase' = "scan" /\ cpCursor' = CPCut[CPSelected] + 1
           /\ UNCHANGED <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
                          cpSeen, cpGrantedAfterComplete>>
CPMatches(i) == CPImage[i] = CPObservedImage[i] /\ CPTerm[i] = CPReportedTerm[i]
CPScan == /\ cpPhase = "scan" /\ cpCursor <= cpApplied
          /\ LET p == cpCursor IN
                IF p \in cpWinners /\ CPObservedImage[p] = CPSelected THEN
                    /\ cpPhase' = IF CPMatches(p) THEN "scan" ELSE "failed"
                    /\ cpSeen' = IF CPMatches(p) /\ cpSeen = 0 THEN p ELSE cpSeen
                ELSE /\ cpPhase' = "scan" /\ cpSeen' = cpSeen
          /\ cpCursor' = cpCursor + 1
          /\ UNCHANGED <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
                         cpGrantedAfterComplete>>
CPComplete == /\ cpPhase = "scan" /\ cpCursor > cpApplied
              /\ cpPhase' = "complete"
              /\ UNCHANGED <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
                             cpCursor, cpSeen, cpGrantedAfterComplete>>
CPGrant == /\ cpPhase = "complete"
           /\ cpPhase' = IF cpSeen # 0 /\ CPFinalHistoryMatches THEN "granted" ELSE "refused"
           /\ cpGrantedAfterComplete' = (cpPhase = "complete")
           /\ UNCHANGED <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration, cpCursor, cpSeen>>
CPFail == /\ cpPhase \in {"scan", "complete"} /\ cpPhase' = "failed"
          /\ UNCHANGED <<cpApplied, cpGeneration, cpWinners, cpWinnerGeneration,
                         cpCursor, cpSeen, cpGrantedAfterComplete>>
CPNext == CPApply \/ CPBegin \/ CPScan \/ CPComplete \/ CPGrant \/ CPFail
CPSpec == CPInit /\ [][CPNext]_cpVars
CPType == /\ cpApplied \in 0..CPLimit /\ cpGeneration \in 0..cpApplied
          /\ cpWinners \subseteq 1..cpApplied
          /\ cpWinnerGeneration \in [1..CPLimit -> 0..cpGeneration]
          /\ cpPhase \in {"write", "scan", "complete", "failed", "granted", "refused"}
          /\ cpCursor \in 0..(cpApplied + 1) /\ cpSeen \in 0..cpApplied
          /\ cpGrantedAfterComplete \in BOOLEAN
CPWinnerIdentity == \A p \in cpWinners :
                       /\ cpWinnerGeneration[p] = CPPred[p] + 1
                       /\ cpWinnerGeneration[p] > 0 /\ CPFresh[p]
                       /\ CPCut[CPImage[p]] < p
CPSeen == cpSeen # 0 => /\ cpSeen \in cpWinners /\ CPImage[cpSeen] = CPSelected
                       /\ CPCut[CPSelected] < cpSeen
                       /\ CPTerm[cpSeen] = CPReportedTerm[cpSeen]
CPGrantSafety == cpPhase = "granted" => cpSeen # 0 /\ cpGrantedAfterComplete /\ CPFinalHistoryMatches
CPScanBounds == cpPhase = "scan" => cpCursor \in 1..(cpApplied + 1)
CPInvariant == CPType /\ CPWinnerIdentity /\ CPSeen /\ CPGrantSafety /\ CPScanBounds
CPRank == CASE cpPhase = "scan" -> cpApplied + 3 - cpCursor
          [] cpPhase = "complete" -> 1
          [] OTHER -> 0
=============================================================================
