------------------------ MODULE RetentionLedgerProof ------------------------
EXTENDS RetentionLedger, TLAPS
THEOREM LInvariantInit == LInit => LInvariant
BY LInputs, SMT DEF LInit, LInvariant, LType, LRecords, LAbsent, LExactLinks,
                   LShapes, LPublicationCovered
THEOREM LInvariantStep == ASSUME LInvariant, [LNext]_lVars PROVE LInvariant'
<1>1. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => LInvariant'
    <2>1. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => LType'
        BY LInputs, SMT DEF LAcquire, LSet, LPinsAfter, LRecord, LInvariant, LType, LRecords
    <2>2. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => LExactLinks'
        BY LInputs, SMT DEF LAcquire, LSet, LPinsAfter, LRecord, LInvariant, LType, LExactLinks
    <2>3. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => LShapes'
        BY LInputs, SMT DEF LAcquire, LSet, LRecord, LInvariant, LType, LRecords, LShapes
    <2>4. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => LPublicationCovered'
        BY LInputs, SMT DEF LAcquire, LSet, LRecord, LInvariant, LType, LPublicationCovered
    <2>5. \A o \in LOwners, g \in 1..LMaxGeneration : LAcquire(o,g) => ~lStale'
        BY SMT DEF LAcquire, LInvariant
    <2> QED BY <2>1, <2>2, <2>3, <2>4, <2>5, SMT DEF LInvariant
<1>2. \A o \in LOwners, g \in 1..LMaxGeneration : LPublish(o,g) => LInvariant'
    BY LInputs, SMT DEF LPublish, LSet, LPinsAfter, LRecord, LInvariant,
        LType, LRecords, LAbsent, LExactLinks, LShapes, LPublicationCovered
<1>3. \A o \in LOwners, g \in 1..LMaxGeneration : LRelease(o,g) => LInvariant'
    BY LInputs, SMT DEF LRelease, LSet, LPinsAfter, LRecord, LInvariant,
        LType, LRecords, LAbsent, LExactLinks, LShapes, LPublicationCovered
<1>4. \A from, to \in LOwners, g \in 1..LMaxGeneration : LShare(from,to,g) => LInvariant'
    BY <1>1, SMT DEF LShare
<1>5. \A from, to \in LOwners, g \in 1..LMaxGeneration : LQuiesce(from,to,g) => LInvariant'
    BY LInputs, SMT DEF LQuiesce, LSet, LPinsAfter, LRecord, LInvariant,
        LType, LRecords, LAbsent, LExactLinks, LShapes, LPublicationCovered
<1>6. UNCHANGED lVars => LInvariant'
    BY DEF lVars, LInvariant, LType, LExactLinks, LShapes, LPublicationCovered
<1> QED BY <1>1, <1>2, <1>3, <1>4, <1>5, <1>6, SMT DEF LNext
THEOREM LInvariantAlways == LSpec => []LInvariant
BY LInvariantInit, LInvariantStep, PTL DEF LSpec
THEOREM LAtomicClosure == LInvariant => LExactLinks
BY DEF LInvariant
THEOREM LSuccessorCoverage == LInvariant => LPublicationCovered
BY DEF LInvariant
THEOREM LStaleTokenNeverApplied == LInvariant => ~lStale
BY DEF LInvariant
THEOREM LShareKeepsSource ==
    ASSUME NEW from \in LOwners, NEW to \in LOwners,
           NEW g \in 1..LMaxGeneration, LType, LShare(from,to,g)
    PROVE lOwners'[from] = lOwners[from]
BY LInputs, SMT DEF LShare, LAcquire, LSet, LType
=============================================================================
