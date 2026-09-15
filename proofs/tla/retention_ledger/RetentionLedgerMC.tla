------------------------ MODULE RetentionLedgerMC ------------------------
EXTENDS RetentionLedger
ModelClosure == [o \in {1,2,3} |-> IF o = 3 THEN {1} ELSE {1,2}]
ModelSubject == [o \in {1,2,3} |-> 1]
FullClosure == [o \in {1,2,3} |-> {1,2}]
SeparateSubjects == [o \in {1,2,3} |-> IF o = 3 THEN 2 ELSE 1]
=============================================================================
