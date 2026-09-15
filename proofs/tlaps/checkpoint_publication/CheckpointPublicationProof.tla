---------------------- MODULE CheckpointPublicationProof ----------------------
EXTENDS CheckpointPublication, TLAPS
THEOREM CPInvariantInit == CPInit => CPInvariant
BY CPInputs, SMT DEF CPInit, CPInvariant, CPType, CPWinnerIdentity, CPSeen, CPGrantSafety, CPScanBounds
THEOREM CPInvariantStep == ASSUME CPInvariant, [CPNext]_cpVars PROVE CPInvariant'
<1>1. CPApply => CPInvariant'
    BY CPInputs, SMT DEF CPApply, CPWins, CPInvariant, CPType, CPWinnerIdentity, CPSeen, CPGrantSafety, CPScanBounds
<1>2. CPBegin => CPInvariant'
    BY CPInputs, SMT DEF CPBegin, CPInvariant, CPType, CPWinnerIdentity, CPSeen, CPGrantSafety, CPScanBounds
<1>3. CPScan => CPInvariant'
    BY CPInputs, SMT DEF CPScan, CPMatches, CPInvariant, CPType, CPWinnerIdentity, CPSeen, CPGrantSafety, CPScanBounds
<1>4. CPComplete \/ CPGrant \/ CPFail \/ UNCHANGED cpVars => CPInvariant'
    BY CPInputs, SMT DEF CPComplete, CPGrant, CPFail, cpVars, CPInvariant, CPType, CPWinnerIdentity, CPSeen, CPGrantSafety, CPScanBounds
<1> QED BY <1>1, <1>2, <1>3, <1>4, SMT DEF CPNext
THEOREM CPInvariantAlways == CPSpec => []CPInvariant
BY CPInvariantInit, CPInvariantStep, PTL DEF CPSpec
THEOREM CPGrantRequiresActualWinner == CPInvariant /\ cpPhase = "granted" =>
    /\ cpSeen \in cpWinners /\ CPImage[cpSeen] = CPSelected
    /\ cpWinnerGeneration[cpSeen] = CPPred[cpSeen] + 1
    /\ CPTerm[cpSeen] = CPReportedTerm[cpSeen] /\ CPCut[CPSelected] < cpSeen
BY SMT DEF CPInvariant, CPGrantSafety, CPSeen, CPWinnerIdentity
THEOREM CPNoPartialRecoveryGrant == CPInvariant /\ cpPhase = "granted" =>
    cpGrantedAfterComplete /\ CPFinalHistoryMatches
BY DEF CPInvariant, CPGrantSafety
THEOREM CPCasLoserCreatesNoWitness ==
    ASSUME CPInvariant, CPApply, CPPred[cpApplied + 1] # cpGeneration
    PROVE cpWinners' = cpWinners /\ cpWinnerGeneration' = cpWinnerGeneration
BY SMT DEF CPApply, CPWins
THEOREM CPRecoveryKeepsFirstWinner ==
    ASSUME CPInvariant, cpSeen # 0, [CPNext]_cpVars PROVE cpSeen' = cpSeen
BY SMT DEF CPNext, CPApply, CPBegin, CPScan, CPComplete, CPGrant, CPFail, cpVars
THEOREM CPRecoveryRankIsNatural == CPInvariant => CPRank \in Nat
BY SMT DEF CPInvariant, CPType, CPScanBounds, CPRank
THEOREM CPRecoveryRankDecreases ==
    ASSUME CPInvariant, CPScan \/ CPComplete \/ CPGrant \/ CPFail
    PROVE CPRank' < CPRank
BY SMT DEF CPInvariant, CPType, CPScanBounds, CPRank, CPScan, CPComplete, CPGrant, CPFail
=============================================================================
