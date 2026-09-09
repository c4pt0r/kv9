----------------------------- MODULE EndpointCAS -----------------------------
EXTENDS Naturals
CONSTANT EPAddresses, EPStores, EPStore, EPInitial, EPLimit
ASSUME EPLegalInputs == EPAddresses # {} /\ EPAddresses \subseteq Nat \ {0}
                        /\ EPStores # {} /\ EPStores \subseteq Nat \ {0}
                        /\ EPStore \in EPStores /\ EPInitial \in EPAddresses
                        /\ EPLimit \in Nat \ {0}

\* One existing Active node/store binding. Evaluation is the serialized catalog
\* planner and committed transaction abstraction. Authentication, Raft receipts
\* and actual transport/restart behavior are separate refinement obligations.
\* EPLimit is arbitrary: the proof includes Rust's u64::MAX without enumerating it.
VARIABLES epGeneration, epAddress, epPrevious, epState,
          epExpected, epOld, epNew, epClaimedStore,
          epResultGeneration, epResultAddress, epResultPrevious
 epVars == <<epGeneration, epAddress, epPrevious, epState,
             epExpected, epOld, epNew, epClaimedStore,
             epResultGeneration, epResultAddress, epResultPrevious>>
EPStates == {"Idle", "Pending", "Changed", "Confirmed", "Refused"}
EPRequests == <<epExpected, epOld, epNew, epClaimedStore>>
EPResults == <<epResultGeneration, epResultAddress, epResultPrevious>>
EPDirectory == <<epGeneration, epAddress, epPrevious>>

EPInit == /\ epGeneration = 0 /\ epAddress = EPInitial /\ epPrevious = 0
          /\ epState = "Idle" /\ epExpected = 0 /\ epOld = EPInitial
          /\ epNew = EPInitial /\ epClaimedStore = EPStore
          /\ epResultGeneration = 0 /\ epResultAddress = 0 /\ epResultPrevious = 0
EPIssue(g, o, n, s) == /\ epState # "Pending"
                      /\ g \in 0..EPLimit /\ o \in EPAddresses
                      /\ n \in EPAddresses /\ s \in EPStores
                      /\ epExpected' = g /\ epOld' = o /\ epNew' = n /\ epClaimedStore' = s
                      /\ epState' = "Pending"
                      /\ epResultGeneration' = 0 /\ epResultAddress' = 0 /\ epResultPrevious' = 0
                      /\ UNCHANGED EPDirectory
EPMatch == epClaimedStore = EPStore /\ epExpected < EPLimit
           /\ epGeneration = epExpected /\ epAddress = epOld
EPReplay == epClaimedStore = EPStore /\ epExpected < EPLimit
            /\ epGeneration = epExpected + 1 /\ epAddress = epNew /\ epPrevious = epOld
EPEvaluate == /\ epState = "Pending"
              /\ IF EPMatch
                 THEN /\ epGeneration' = epGeneration + 1
                      /\ epAddress' = epNew /\ epPrevious' = epAddress
                      /\ epState' = "Changed"
                      /\ epResultGeneration' = epGeneration + 1
                      /\ epResultAddress' = epNew /\ epResultPrevious' = epAddress
                 ELSE IF EPReplay
                      THEN /\ UNCHANGED EPDirectory /\ epState' = "Confirmed"
                           /\ epResultGeneration' = epGeneration
                           /\ epResultAddress' = epAddress /\ epResultPrevious' = epPrevious
                      ELSE /\ UNCHANGED EPDirectory /\ epState' = "Refused"
                           /\ epResultGeneration' = 0 /\ epResultAddress' = 0 /\ epResultPrevious' = 0
              /\ UNCHANGED EPRequests
\* A different authorized caller can commit while the sampled RPC is delayed.
EPInterfere(a) == /\ a \in EPAddresses /\ epGeneration < EPLimit
                  /\ epGeneration' = epGeneration + 1
                  /\ epPrevious' = epAddress /\ epAddress' = a
                  /\ UNCHANGED <<epState, EPRequests, EPResults>>
EPRetry == /\ epState \in {"Changed", "Confirmed", "Refused"}
           /\ epState' = "Pending"
           /\ epResultGeneration' = 0 /\ epResultAddress' = 0 /\ epResultPrevious' = 0
           /\ UNCHANGED <<EPDirectory, EPRequests>>
EPNext == (\E g \in 0..EPLimit, o, n \in EPAddresses, s \in EPStores : EPIssue(g, o, n, s))
          \/ EPEvaluate \/ (\E a \in EPAddresses : EPInterfere(a)) \/ EPRetry
EPSpec == EPInit /\ [][EPNext]_epVars

EPType == /\ epGeneration \in 0..EPLimit /\ epAddress \in EPAddresses
          /\ epPrevious \in EPAddresses \cup {0} /\ epState \in EPStates
          /\ epExpected \in 0..EPLimit /\ epOld \in EPAddresses
          /\ epNew \in EPAddresses /\ epClaimedStore \in EPStores
          /\ epResultGeneration \in 0..EPLimit
          /\ epResultAddress \in EPAddresses \cup {0}
          /\ epResultPrevious \in EPAddresses \cup {0}
EPBinding == /\ (epGeneration = 0 <=> epPrevious = 0)
             /\ (epState \in {"Changed", "Confirmed"} =>
                   /\ epClaimedStore = EPStore /\ epResultGeneration = epExpected + 1
                   /\ epResultAddress = epNew /\ epResultPrevious = epOld)
EPInvariant == EPType /\ EPBinding
EPAtomicVersion == epGeneration' \in {epGeneration, epGeneration + 1}
                   /\ (epGeneration' = epGeneration => UNCHANGED <<epAddress, epPrevious>>)
EPChangePrecondition == (EPEvaluate /\ epState' = "Changed") =>
                         epClaimedStore = EPStore /\ epExpected = epGeneration
                         /\ epOld = epAddress /\ epExpected < EPLimit
EPConfirmation == (EPEvaluate /\ epState' = "Confirmed") =>
                    /\ UNCHANGED EPDirectory
                    /\ epClaimedStore = EPStore /\ epExpected + 1 = epGeneration
                    /\ epNew = epAddress /\ epOld = epPrevious
EPNoRefusalWrite == (EPEvaluate /\ epState' = "Refused") => UNCHANGED EPDirectory
EPStepEffects == EPAtomicVersion /\ EPChangePrecondition /\ EPConfirmation /\ EPNoRefusalWrite
EPEffects == [][EPStepEffects]_epVars

\* Backend completion needs fair execution and completion of its consensus work.
\* This proves a terminal result, which may be a conflict; it does not promise
\* success while other operators continually supersede the requested route.
EPFairSpec == EPInvariant /\ [][EPNext]_epVars /\ WF_epVars(EPEvaluate)
EPPending == EPInvariant /\ epState = "Pending"
EPDone == EPInvariant /\ epState # "Pending"
EPProgress == (epState = "Pending") ~> (epState # "Pending")
EPEligible == EPPending /\ EPMatch
EPChanged == epState = "Changed"
EPStableSpec == EPEligible /\ [][EPEvaluate]_epVars /\ WF_epVars(EPEvaluate)
EPSuccess == <>(epState = "Changed")
EPNoABA == ~(epGeneration >= 2 /\ epAddress = EPInitial)
EPNoConfirmation == epState # "Confirmed"
=============================================================================
