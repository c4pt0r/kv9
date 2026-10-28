---------------------- MODULE CheckpointIntentMC ----------------------
EXTENDS CheckpointIntent
CONSTANTS a, b, da, db
CanonicalMC == [o \in Operations |-> IF o = a THEN da ELSE db]
=======================================================================
