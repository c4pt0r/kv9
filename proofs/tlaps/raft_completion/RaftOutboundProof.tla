------------------------- MODULE RaftOutboundProof -------------------------
EXTENDS RaftOutbound, TLAPS, NaturalsInduction

THEOREM ROInvariantInit == ROInit => ROInvariant
<1>1. ROInit => ROType
    BY ROLegalInputs, SMT DEF ROInit, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>2. ROInit => ROPrefix
    BY ROLegalInputs, SMT DEF ROInit, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>3. ROInit => ROFIFO
    BY ROLegalInputs, SMT DEF ROInit, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>4. ROInit => ROByteLedger
    BY ROLegalInputs, SMT DEF ROInit, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>5. ROInit => ROCapturedClient
    BY ROLegalInputs, SMT DEF ROInit, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>6. ROInit => ROExactDestination
    BY ROLegalInputs, SMT DEF ROInit, ROExactDestination
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF ROInvariant

THEOREM ROArriveStep == ASSUME ROInvariant, ROArrive PROVE ROInvariant'
BY ROLegalInputs, SMT DEF ROArrive, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM RORouteChangeStep == ASSUME ROInvariant, NEW d \in RODestinations, RORouteChange(d) PROVE ROInvariant'
BY ROLegalInputs, SMT DEF RORouteChange, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROPopStep == ASSUME ROInvariant, ROPop PROVE ROInvariant'
<1>1. ROType'
    BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>2. ROPrefix'
    BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>3. ROFIFO'
    <2>1. CASE ~ROMatches(roHead)
        BY <2>1, SMT DEF ROPop, ROInvariant, ROFIFO
    <2>2. CASE ROMatches(roHead)
        <3>1. roHead \in ROItems /\ roHead \notin roAccepted /\ roCount \in Nat /\
            (\A i \in roAccepted : i < roHead /\ roOrder[i] \in 1..roCount)
            BY <2>2, ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
        <3>2. roAccepted' = roAccepted \cup {roHead} /\ roCount' = roCount + 1 /\
            roOrder' = [roOrder EXCEPT ![roHead] = roCount + 1]
            BY <2>2, SMT DEF ROPop
        <3>3. \A i \in ROItems : (i \in roAccepted') <=> (roOrder'[i] \in 1..roCount')
            BY <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROFIFO, ROItems
        <3>4. \A i \in ROItems : i \notin roAccepted' => roOrder'[i] = 0
            BY <3>1, <3>2, SMT DEF ROInvariant, ROExactDestination, ROType, ROFIFO
        <3>5. \A i, j \in roAccepted' : (i < j) <=> (roOrder'[i] < roOrder'[j])
            <4>1. SUFFICES ASSUME NEW i \in roAccepted', NEW j \in roAccepted'
                PROVE (i < j) <=> (roOrder'[i] < roOrder'[j]) OBVIOUS
            <4>2. CASE i = roHead
                BY <4>1, <4>2, <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROFIFO, ROItems
            <4>3. CASE j = roHead
                BY <4>1, <4>3, <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROFIFO, ROItems
            <4>4. CASE i # roHead /\ j # roHead
                BY <4>1, <4>4, <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROFIFO, ROItems
            <4> QED BY <4>2, <4>3, <4>4, SMT
        <3>6. \A k \in 1..roCount' : \E i \in roAccepted' : roOrder'[i] = k
            <4>1. SUFFICES ASSUME NEW k \in 1..roCount' PROVE \E i \in roAccepted' : roOrder'[i] = k OBVIOUS
            <4>2. CASE k = roCount + 1
                BY <4>1, <4>2, <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType
            <4>3. CASE k # roCount + 1
                <5>1. k \in 1..roCount BY <4>1, <4>3, <3>1, <3>2, SMT
                <5>2. PICK i \in roAccepted : roOrder[i] = k
                    BY <5>1, SMT DEF ROInvariant, ROFIFO
                <5> QED BY <5>2, <3>1, <3>2, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType
            <4> QED BY <4>2, <4>3, SMT
        <3> QED BY <3>3, <3>4, <3>5, <3>6 DEF ROFIFO
    <2> QED BY <2>1, <2>2, SMT
<1>4. ROByteLedger'
    <2>1. roHead \in ROItems /\ roHead >= 2 /\ roHead' = roHead + 1 /\
        roBytes = roTotals[roHead - 1] /\ ROWeight[roHead] \in 0..ROMaxWeight
        BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>2. roTotals'[1] = ROWeight[1]
        BY <2>1, ROLegalInputs, SMT DEF ROPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>3. roBytes' = roTotals'[roHead' - 1]
        BY <2>1, ROLegalInputs, SMT DEF ROPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>4. \A i \in 2..(roHead' - 1) : roTotals'[i] = roTotals'[i - 1] + (IF ROMatches(i) THEN ROWeight[i] ELSE 0)
        <3>1. SUFFICES ASSUME NEW i \in 2..(roHead' - 1)
            PROVE roTotals'[i] = roTotals'[i - 1] + (IF ROMatches(i) THEN ROWeight[i] ELSE 0) OBVIOUS
        <3>2. CASE i = roHead
            BY <3>1, <3>2, <2>1, ROLegalInputs, SMT DEF ROPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
        <3>3. CASE i # roHead
            BY <3>1, <3>3, <2>1, ROLegalInputs, SMT DEF ROPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
        <3> QED BY <3>2, <3>3, SMT
    <2>5. roBytes' < ROByteTarget + ROMaxWeight
        BY <2>1, ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2> QED BY <2>2, <2>3, <2>4, <2>5 DEF ROByteLedger
<1>5. ROCapturedClient'
    BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>6. ROExactDestination'
    BY ROLegalInputs, SMT DEF ROPop, ROInvariant, ROExactDestination, ROMatches
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6 DEF ROInvariant

THEOREM ROFinishStep == ASSUME ROInvariant, ROFinish PROVE ROInvariant'
BY ROLegalInputs, SMT DEF ROFinish, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROOfferStep == ASSUME ROInvariant, ROOffer PROVE ROInvariant'
BY ROLegalInputs, SMT DEF ROOffer, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROCancelStep == ASSUME ROInvariant, ROCancel PROVE ROInvariant'
BY ROLegalInputs, SMT DEF ROCancel, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROInvariantStep == ASSUME ROInvariant, [RONext]_roVars PROVE ROInvariant'
BY ROArriveStep, RORouteChangeStep, ROPopStep, ROFinishStep, ROOfferStep, ROCancelStep, SMT DEF RONext, ROService, ROQuiesce, roVars, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROInvariantAlways == ROSpec => []ROInvariant
BY ROInvariantInit, ROInvariantStep, PTL DEF ROSpec

THEOREM ROInspectionCounted == ASSUME ROInvariant, ROPop PROVE roInspected' = roInspected + 1 /\ roInspected' <= ROMaxBatch
BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROTerminalStep == ASSUME ROInvariant, RONext PROVE ROTerminal
BY ROLegalInputs, SMT DEF ROTerminal, RONext, ROService, ROQuiesce, roVars, ROArrive, RORouteChange, ROPop, ROFinish, ROOffer, ROCancel, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROTerminalAlways == ROSpec => ROSafety
<1>1. ROInvariant /\ [RONext]_roVars => [ROTerminal]_roVars
    BY ROTerminalStep, SMT DEF roVars
<1> QED BY <1>1, ROInvariantAlways, PTL DEF ROSpec, ROSafety

THEOREM ROPopStrictVariant == ASSUME ROInvariant, ROPop PROVE
    ROMaxBatch - roInspected' < ROMaxBatch - roInspected /\ ROMaxBatch - roInspected' \in Nat
BY ROLegalInputs, SMT DEF ROPop, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches

THEOREM ROFairInvariant == ROFairSpec => []ROInvariant
BY ROInvariantStep, PTL DEF ROFairSpec

THEOREM ROServiceEnabled == ASSUME ROInvariant, roPhase = "coalesce" PROVE ENABLED <<ROService>>_roVars
<1>1. ENABLED ROService
    BY ExpandENABLED, ROLegalInputs, Isa DEF ROService, ROPop, ROFinish, ROCanPop
<1>2. ROService <=> <<ROService>>_roVars
    BY ROLegalInputs, SMT DEF ROService, ROPop, ROFinish, ROCanPop, roVars, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>3. ENABLED ROService <=> ENABLED <<ROService>>_roVars
    BY <1>2, ENABLEDaxioms
<1> QED BY <1>1, <1>3, SMT

ROBudgetDrop(k) == ROFairSpec =>
    ((roPhase = "coalesce" /\ ROMaxBatch - roInspected = k) ~>
     (ROSettled \/ (roPhase = "coalesce" /\ ROMaxBatch - roInspected < k)))
THEOREM ROBudgetProgress == ASSUME NEW k \in Nat PROVE ROBudgetDrop(k)
<1>1. DEFINE P == roPhase = "coalesce" /\ ROMaxBatch - roInspected = k
            Q == ROSettled \/ (roPhase = "coalesce" /\ ROMaxBatch - roInspected < k)
<1>2. ROInvariant /\ P /\ [RONext]_roVars => ROInvariant' /\ (P' \/ Q')
    BY ROInvariantStep, ROLegalInputs, SMT DEF P, Q, ROSettled, RONext, ROService, ROArrive, RORouteChange, ROPop, ROFinish, ROOffer, ROCancel, ROCanPop, ROQuiesce, roVars, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>3. ROInvariant /\ P /\ ROService => Q'
    BY ROLegalInputs, SMT DEF P, Q, ROSettled, ROService, ROPop, ROFinish, ROCanPop, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>4. ROInvariant /\ P => ENABLED <<ROService>>_roVars
    BY ROServiceEnabled, SMT DEF P
<1> QED BY ROFairInvariant, <1>2, <1>3, <1>4, PTL DEF ROFairSpec, ROFairness, P, Q, ROBudgetDrop

THEOREM ROBoundedServiceProgress == ROFairSpec => ROProgress
<1>1. DEFINE P(n) == ROFairSpec =>
    ((roPhase = "coalesce" /\ ROMaxBatch - roInspected <= n) ~> ROSettled)
<1> HIDE DEF P
<1>2. P(0)
    <2>1. ROInvariant => ROMaxBatch - roInspected \in Nat
        BY ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>2. ROInvariant /\ roPhase = "coalesce" /\ ROMaxBatch - roInspected <= 0 =>
        roPhase = "coalesce" /\ ROMaxBatch - roInspected = 0
        BY <2>1, SMT
    <2>3. ROInvariant => ~(roPhase = "coalesce" /\ ROMaxBatch - roInspected < 0)
        BY <2>1, SMT
    <2>4. ROBudgetDrop(0) BY ROBudgetProgress, SMT
    <2> QED BY <2>2, <2>3, <2>4, ROFairInvariant, PTL DEF P, ROBudgetDrop
<1>3. ASSUME NEW n \in Nat PROVE P(n) => P(n + 1)
    <2>1. ROInvariant /\ roPhase = "coalesce" /\ ROMaxBatch - roInspected <= n + 1 =>
        (roPhase = "coalesce" /\ ROMaxBatch - roInspected <= n) \/
        (roPhase = "coalesce" /\ ROMaxBatch - roInspected = n + 1)
        BY <1>3, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>2. ROInvariant /\ roPhase = "coalesce" /\ ROMaxBatch - roInspected < n + 1 =>
        roPhase = "coalesce" /\ ROMaxBatch - roInspected <= n
        BY <1>3, ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
    <2>3. ROBudgetDrop(n + 1) BY <1>3, ROBudgetProgress, SMT
    <2> QED BY <2>1, <2>2, <2>3, ROFairInvariant, PTL DEF P, ROBudgetDrop
<1>4. \A n \in Nat : P(n)
    BY <1>2, <1>3, NatInduction, Isa
<1>5. P(ROMaxBatch) BY <1>4, ROLegalInputs, SMT
<1>6. ROInvariant /\ roPhase = "coalesce" => roPhase = "coalesce" /\ ROMaxBatch - roInspected <= ROMaxBatch
    BY ROLegalInputs, SMT DEF ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1> QED BY <1>5, <1>6, ROFairInvariant, PTL DEF P, ROProgress

THEOREM ROOfferProgressProof == ROFairSpec => ROOfferProgress
<1>1. DEFINE P == roPhase = "ready"
            Q == roPhase \in {"offered", "closed"}
<1>2. ROInvariant /\ P /\ [RONext]_roVars => ROInvariant' /\ (P' \/ Q')
    BY ROInvariantStep, ROLegalInputs, SMT DEF P, Q, RONext, ROService, ROArrive, RORouteChange, ROPop, ROFinish, ROOffer, ROCancel, ROCanPop, ROQuiesce, roVars, ROInvariant, ROExactDestination, ROType, ROPrefix, ROFIFO, ROByteLedger, ROCapturedClient, ROItems, ROMatches
<1>3. ROInvariant /\ P /\ ROOffer => Q'
    BY ROLegalInputs, SMT DEF P, Q, ROOffer
<1>4. ROInvariant /\ P => ENABLED <<ROOffer>>_roVars
    BY ExpandENABLED, ROLegalInputs, SMT DEF P, ROOffer, roVars
<1> QED BY ROFairInvariant, <1>2, <1>3, <1>4, PTL DEF ROFairSpec, ROFairness, P, Q, ROOfferProgress
=============================================================================
