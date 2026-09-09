-------------------------- MODULE ReceiveAuthority --------------------------
EXTENDS Naturals
CONSTANT GRIncarnations
ASSUME GRLegalInputs == GRIncarnations # {} /\ GRIncarnations \subseteq Nat \ {0}

\* One dynamic replica id. Its committed store binding is immutable, even
\* across revocation/new tickets. Initial-root formation is a separate protocol.
\* Trusted registration and durable recovery are explicit environmental cuts;
\* this model does not prove the underlying Raft log or ticket cryptography.
VARIABLES grBound, grLocal, grLive, grReceipt, grCertificate, grGate,
          grOwner, grRoute, grReceived
grVars == <<grBound, grLocal, grLive, grReceipt, grCertificate, grGate,
            grOwner, grRoute, grReceived>>
GRInit == /\ grBound = 0 /\ grLocal \in GRIncarnations /\ grLive = TRUE
          /\ grReceipt = 0 /\ grCertificate = 0 /\ grGate = FALSE
          /\ grOwner = FALSE /\ grRoute = 0 /\ grReceived = {}

\* Leader has validated a pending ticket and durably committed its consume
\* and incarnation binding. Already-bound ids cannot acquire another binding.
GRBind(i) == /\ i \in GRIncarnations /\ grBound = 0 /\ grBound' = i
             /\ UNCHANGED <<grLocal, grLive, grReceipt, grCertificate, grGate,
                            grOwner, grRoute, grReceived>>
\* This endpoint side effect follows the validated exact binding.
GRRoute(i) == /\ i \in GRIncarnations /\ grBound = i /\ grRoute' = i
              /\ UNCHANGED <<grBound, grLocal, grLive, grReceipt, grCertificate,
                             grGate, grOwner, grReceived>>
\* A successful response to this live incarnation's own request. Lost or
\* rejected responses do not execute this step and do not grant permission.
GRReply == /\ grLive /\ grBound = grLocal /\ grReceipt' = grLocal
           /\ UNCHANGED <<grBound, grLocal, grLive, grCertificate, grGate,
                          grOwner, grRoute, grReceived>>
GRGrant == /\ grLive /\ grReceipt = grLocal /\ ~grGate
           /\ grGate' = TRUE
           /\ UNCHANGED <<grBound, grLocal, grLive, grReceipt, grCertificate,
                          grOwner, grRoute, grReceived>>
GRStart == /\ grLive /\ grGate /\ ~grOwner /\ grOwner' = TRUE
           /\ UNCHANGED <<grBound, grLocal, grLive, grReceipt, grCertificate,
                          grGate, grRoute, grReceived>>
GRReceive == /\ grLive /\ grGate /\ grReceived' = grReceived \cup {grLocal}
             /\ UNCHANGED <<grBound, grLocal, grLive, grReceipt, grCertificate,
                            grGate, grOwner, grRoute>>
\* The exact local Active row and ConfState have been durably applied.
GRCertify == /\ grLive /\ grOwner /\ grCertificate' = grLocal
             /\ UNCHANGED <<grBound, grLocal, grLive, grReceipt, grGate,
                            grOwner, grRoute, grReceived>>
GRCrash == /\ grLive /\ grLive' = FALSE /\ grGate' = FALSE /\ grOwner' = FALSE
           /\ grReceipt' = 0
           /\ UNCHANGED <<grBound, grLocal, grCertificate, grRoute, grReceived>>
\* Fresh disks have a different incarnation. A saved certificate grants
\* permission only to that same durable store; a volatile grant never survives.
GRRestart(i) == /\ ~grLive /\ i \in GRIncarnations /\ grLocal' = i /\ grLive' = TRUE
               /\ grGate' = (grCertificate = i) /\ grOwner' = FALSE /\ grReceipt' = 0
               /\ UNCHANGED <<grBound, grCertificate, grRoute, grReceived>>
GRQuiesce == UNCHANGED grVars
GRNext == (\E i \in GRIncarnations : GRBind(i) \/ GRRoute(i) \/ GRRestart(i))
          \/ GRReply \/ GRGrant \/ GRStart \/ GRReceive \/ GRCertify \/ GRCrash \/ GRQuiesce
GRSpec == GRInit /\ [][GRNext]_grVars
GRType == /\ grBound \in GRIncarnations \cup {0} /\ grLocal \in GRIncarnations
          /\ grLive \in BOOLEAN /\ grGate \in BOOLEAN /\ grOwner \in BOOLEAN
          /\ grReceipt \in GRIncarnations \cup {0} /\ grCertificate \in GRIncarnations \cup {0}
          /\ grRoute \in GRIncarnations \cup {0} /\ grReceived \in SUBSET GRIncarnations
GRCredentials == /\ (grReceipt # 0 => grReceipt = grBound)
                 /\ (grCertificate # 0 => grCertificate = grBound)
GRPermission == /\ (grGate => grLive /\ grLocal = grBound)
                /\ (grOwner => grGate)
GRRouting == grRoute # 0 => grRoute = grBound
GRSafety == grReceived \subseteq {grBound}
GRInvariant == GRType /\ GRCredentials /\ GRPermission /\ GRRouting /\ GRSafety
GRBindingStep == grBound # 0 => grBound' = grBound
GRBindingAlways == [][GRBindingStep]_grVars

\* Crash-free progress AFTER the environment has supplied the valid response.
\* Fair local owner scheduling and a repeatedly offered message are required;
\* no availability is claimed during unlimited crashes or absent quorum service.
GRStableNext == (\E i \in GRIncarnations : GRBind(i) \/ GRRoute(i))
                \/ GRReply \/ GRGrant \/ GRStart \/ GRReceive \/ GRCertify \/ GRQuiesce
GRFairSpec == GRInvariant /\ [][GRStableNext]_grVars
              /\ WF_grVars(GRGrant) /\ WF_grVars(GRStart) /\ WF_grVars(GRReceive)
GRReady == GRInvariant /\ grLive /\ grReceipt = grLocal
GRGated == GRReady /\ grGate
GRRunning == GRGated /\ grOwner
GRDelivered == GRRunning /\ grLocal \in grReceived
GRProgress == (grLive /\ grReceipt = grLocal) ~> (grOwner /\ grLocal \in grReceived)
GRNoReceive == grReceived = {}
GRNoRecoveredOwner == ~(grReceipt = 0 /\ grOwner)
=============================================================================
