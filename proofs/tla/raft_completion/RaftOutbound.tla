------------------------- MODULE RaftOutbound -------------------------
EXTENDS Naturals
CONSTANTS ROMaxItems, ROMaxBatch, ROByteTarget, ROMaxWeight,
    RODestinations, ROAddresses, ROAddress, ROSession, RORoute, ROWeight
ASSUME ROLegalInputs ==
    /\ ROMaxItems \in Nat \ {0} /\ ROMaxBatch \in Nat \ {0}
    /\ ROByteTarget \in Nat \ {0} /\ ROMaxWeight \in Nat
    /\ RODestinations # {} /\ RODestinations \subseteq Nat \ {0}
    /\ ROAddresses # {} /\ ROAddresses \subseteq Nat \ {0}
    /\ ROAddress \in [RODestinations -> ROAddresses] /\ ROSession \in RODestinations
    /\ RORoute \in [1..ROMaxItems -> RODestinations] /\ RORoute[1] = ROSession
    /\ ROWeight \in [1..ROMaxItems -> 0..ROMaxWeight]
\* One coalescing call starts with a first envelope already validated for its
\* captured session. Input ordinals name immutable envelopes, not addresses.
\* Every nonblocking receive observes the queue at that call: concurrent
\* arrivals can join before a later receive; there is no entry-time snapshot.
ROItems == 1..ROMaxItems
ROMatches(i) == RORoute[i] = ROSession
VARIABLES roArrived, roHead, roCurrent, roUsedRoutes, roPhase, roInspected,
    roAccepted, roOrder, roCount, roBytes, roTotals, roClient
roVars == <<roArrived, roHead, roCurrent, roUsedRoutes, roPhase, roInspected,
    roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
ROInit ==
    /\ roArrived = 1 /\ roHead = 2 /\ roCurrent = ROSession /\ roUsedRoutes = {ROSession}
    /\ roPhase = "coalesce" /\ roInspected = 1 /\ roAccepted = {1}
    /\ roOrder = [i \in ROItems |-> IF i = 1 THEN 1 ELSE 0]
    /\ roCount = 1 /\ roBytes = ROWeight[1]
    /\ roTotals = [i \in ROItems |-> IF i = 1 THEN ROWeight[1] ELSE 0]
    /\ roClient = 0
ROArrive ==
    /\ roArrived < ROMaxItems /\ RORoute[roArrived + 1] = roCurrent
    /\ roArrived' = roArrived + 1
    /\ UNCHANGED <<roHead, roCurrent, roUsedRoutes, roPhase, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
RORouteChange(d) ==
    /\ d \in RODestinations /\ d \notin roUsedRoutes
    /\ roCurrent' = d /\ roUsedRoutes' = roUsedRoutes \cup {d}
    /\ UNCHANGED <<roArrived, roHead, roPhase, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
ROCanPop == roInspected < ROMaxBatch /\ roBytes < ROByteTarget /\ roHead <= roArrived
ROPop ==
    /\ roPhase = "coalesce" /\ ROCanPop
    /\ roHead' = roHead + 1 /\ roInspected' = roInspected + 1
    /\ roAccepted' = (IF ROMatches(roHead) THEN roAccepted \cup {roHead} ELSE roAccepted)
    /\ roOrder' = (IF ROMatches(roHead) THEN [roOrder EXCEPT ![roHead] = roCount + 1] ELSE roOrder)
    /\ roCount' = (IF ROMatches(roHead) THEN roCount + 1 ELSE roCount)
    /\ roBytes' = (IF ROMatches(roHead) THEN roBytes + ROWeight[roHead] ELSE roBytes)
    /\ roTotals' = [roTotals EXCEPT ![roHead] = IF ROMatches(roHead) THEN roBytes + ROWeight[roHead] ELSE roBytes]
    /\ UNCHANGED <<roArrived, roCurrent, roUsedRoutes, roPhase, roClient>>
ROFinish == roPhase = "coalesce" /\ ~ROCanPop /\ roPhase' = "ready"
    /\ UNCHANGED <<roArrived, roHead, roCurrent, roUsedRoutes, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
ROOffer == roPhase = "ready" /\ roPhase' = "offered" /\ roClient' = ROSession
    /\ UNCHANGED <<roArrived, roHead, roCurrent, roUsedRoutes, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals>>
ROCancel == roPhase \in {"coalesce", "ready"} /\ roPhase' = "closed"
    /\ UNCHANGED <<roArrived, roHead, roCurrent, roUsedRoutes, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
ROQuiesce == UNCHANGED roVars
ROService == ROPop \/ ROFinish
RONext == ROArrive \/ (\E d \in RODestinations : RORouteChange(d)) \/ ROService \/ ROOffer \/ ROCancel \/ ROQuiesce
ROSpec == ROInit /\ [][RONext]_roVars
ROType ==
    /\ roArrived \in 1..ROMaxItems /\ roHead \in 2..(ROMaxItems + 1)
    /\ roCurrent \in RODestinations /\ roUsedRoutes \subseteq RODestinations /\ roCurrent \in roUsedRoutes
    /\ roPhase \in {"coalesce", "ready", "offered", "closed"}
    /\ roInspected \in Nat /\ roAccepted \subseteq ROItems
    /\ roOrder \in [ROItems -> Nat] /\ roCount \in Nat /\ roBytes \in Nat
    /\ roTotals \in [ROItems -> Nat] /\ roClient \in RODestinations \cup {0}
ROPrefix ==
    /\ roHead <= roArrived + 1 /\ roInspected = roHead - 1
    /\ roInspected <= ROMaxBatch /\ roCount \in 1..roInspected
    /\ roAccepted = {i \in 1..(roHead - 1) : ROMatches(i)}
ROFIFO ==
    /\ \A i \in ROItems : (i \in roAccepted) <=> (roOrder[i] \in 1..roCount)
    /\ \A i \in ROItems : i \notin roAccepted => roOrder[i] = 0
    /\ \A i, j \in roAccepted : (i < j) <=> (roOrder[i] < roOrder[j])
    /\ \A k \in 1..roCount : \E i \in roAccepted : roOrder[i] = k
ROByteLedger ==
    /\ roTotals[1] = ROWeight[1] /\ roBytes = roTotals[roHead - 1]
    /\ \A i \in 2..(roHead - 1) : roTotals[i] = roTotals[i - 1] + (IF ROMatches(i) THEN ROWeight[i] ELSE 0)
    /\ roBytes < ROByteTarget + ROMaxWeight
ROCapturedClient ==
    /\ (roPhase = "offered" => roClient = ROSession)
    /\ (roPhase # "offered" => roClient = 0)
ROExactDestination == \A i \in roAccepted : RORoute[i] = ROSession
ROInvariant == ROExactDestination /\ ROType /\ ROPrefix /\ ROFIFO /\ ROByteLedger /\ ROCapturedClient
ROTerminal == roPhase \in {"offered", "closed"} =>
    UNCHANGED <<roHead, roPhase, roInspected, roAccepted, roOrder, roCount, roBytes, roTotals, roClient>>
ROSafety == [][ROTerminal]_roVars
ROFairness == WF_roVars(ROService) /\ WF_roVars(ROOffer)
ROFairSpec == ROInvariant /\ [][RONext]_roVars /\ ROFairness
ROSettled == roPhase \in {"ready", "offered", "closed"}
ROProgress == (roPhase = "coalesce") ~> ROSettled
ROOfferProgress == (roPhase = "ready") ~> (roPhase \in {"offered", "closed"})
RONoStaleWitness == roCount = roInspected
RONoConcurrentWitness == roArrived = 1
RONoOvershootWitness == roBytes <= ROByteTarget
RONoOfferWitness == roPhase # "offered"
=============================================================================
