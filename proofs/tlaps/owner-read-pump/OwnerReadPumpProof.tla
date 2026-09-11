------------------------- MODULE OwnerReadPumpProof -------------------------
EXTENDS OwnerReadPump, TLAPS
THEOREM ORInvariantInit == ORInit => ORInvariant
BY SMT DEF ORInit, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORNotifyStep == ASSUME ORInvariant, ORNotify PROVE ORInvariant'
BY SMT DEF ORNotify, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORSubmitStep == ASSUME ORInvariant, NEW ok \in BOOLEAN, ORSubmit(ok) PROVE ORInvariant'
BY SMT DEF ORSubmit, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORPumpStep == ASSUME ORInvariant, NEW ok \in BOOLEAN, ORPump(ok) PROVE ORInvariant'
BY SMT DEF ORPump, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORAbortStep == ASSUME ORInvariant, ORAbort PROVE ORInvariant'
BY SMT DEF ORAbort, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORInvariantStep == ASSUME ORInvariant, [ORNext]_orVars PROVE ORInvariant'
BY ORNotifyStep, ORSubmitStep, ORPumpStep, ORAbortStep, SMT
   DEF ORNext, orVars, ORInvariant, ORType, OROrder, ORNoLostNotification
THEOREM ORInvariantAlways == ORSpec => []ORInvariant
BY ORInvariantInit, ORInvariantStep, PTL DEF ORSpec
=============================================================================
