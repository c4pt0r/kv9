----------------------------- MODULE EndpointMC -----------------------------
EXTENDS EndpointCAS
\* Finite test instances choose EPLimit in their configuration. The deductive
\* proof remains parameterized over every positive limit, including u64::MAX.
EPMCFairSpec == EPInit /\ EPFairSpec
\* Spell out the allowed stutter as an explicit successor so TLC can finish a
\* terminal successful RPC without reporting a deadlock. By expansion,
\* [EPEvaluate \/ UNCHANGED epVars]_epVars equals [EPEvaluate]_epVars.
EPMCStableSpec == EPEligible /\ [][EPEvaluate \/ UNCHANGED epVars]_epVars
                  /\ WF_epVars(EPEvaluate)
                  /\ epGeneration = 0 /\ epAddress = EPInitial
                  /\ epPrevious = 0 /\ epExpected = 0 /\ epOld = EPInitial
                  /\ epClaimedStore = EPStore
                  /\ epResultGeneration = 0 /\ epResultAddress = 0 /\ epResultPrevious = 0
=============================================================================
