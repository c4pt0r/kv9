--------------------- MODULE CheckpointAdmissionMC ---------------------
EXTENDS CheckpointAdmission
Bounded == \A e \in Executors : generation[e] <= 2
=======================================================================
